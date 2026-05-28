//! Full-BAM end-to-end parity harness against cached Java TSV shards.
//!
//! This module streams large BAM-backed regions in chunked tile windows, dispatches by BAM tag,
//! and compares Rust output against the pre-generated Java cache under the configured fixture
//! root (default `tmp/sweep_fixtures/output/`; override with the
//! `VARDICT_E2E_SWEEP_FIXTURE_ROOT` environment variable).
//!
//! Prerequisites:
//! - The fixture root (default `tmp/sweep_fixtures/`, override via
//!   `VARDICT_E2E_SWEEP_FIXTURE_ROOT`) must exist and contain `output/` plus a valid
//!   `manifest.json`.
//! - Regenerate the cache with `bash scripts/gen_e2e_sweep_golden.sh` when fixtures are missing
//!   or stale.
//!
//! Environment:
//! - `VARDICT_E2E_SWEEP_CONFIG=<name>` selects the cache layout; defaults to `default`.
//! - `VARDICT_E2E_SWEEP_SHARD=i/N` optionally runs only one shard of the tile set.
//! - `VARDICT_E2E_SWEEP_HEARTBEAT_LOG=<path>` appends diagnostic phase markers and runtime
//!   telemetry to a side-channel file while also echoing them to stderr. Heartbeats never enter
//!   captured variant buffers, TSV rows, JSONL fixtures, or parity comparisons.
//! - `CI=true` converts missing-cache handling into a hard panic instead of a local skip.
//!
//! Run with:
//! `cargo test --profile debug-release --test parity_e2e_sweep -- --include-ignored --test-threads=10`
//!
//! To add a new BAM tag:
//! 1. Add the lookup entry in `tests/common/mod.rs::bam_tag_lookup`.
//! 2. Add a `<tag>_sweep.rs` trial-builder stub plus its `#[path]` line in `tests/parity_e2e_sweep.rs`.
//! 3. Append the tag in `scripts/gen_e2e_sweep_golden.sh`.
//! 4. Regenerate the cache before running this harness.
//!
//! Phase 0a note: the sample name is derived from the BAM file stem, not `test_sample`; keep
//! that behavior unchanged.
//!
//! ## Manifest cache_entries schema
//!
//! Single-BAM cache entries use key shape `{config}:{tag}` and record
//! `bam_stat=[{path,size,mtime_unix}]` with `reference_sha256` fingerprinted from
//! `testdata/hs37d5.fa.fai`.
//!
//! Somatic cache entries use key shape `{config}:somatic:{tag}` and record
//! `bam_stat=[{path,size,mtime_unix,role:"tumor"},{path,size,mtime_unix,role:"normal"}]`
//! with `reference_sha256` fingerprinted from `testdata/GRCh38.d1.vd1.fa.fai`.
//!
//! Backward-compatibility guarantee: this R2 harness only looks up `{config}:{tag}` keys, so
//! `:somatic:` entries are ignored here and consumed by the Phase 5 somatic validator instead.
// Phase 4 (somatic): pub(crate) visibility on helpers cross-binary somatic reuse.
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use libtest_mimic::{Failed, Trial};
use serde_json::Value;
use sha2::{Digest, Sha256};
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

pub const MAX_FAILURES: usize = 10;
pub const CHUNK_SIZE: usize = 20_000;

static FAILURE_COUNT: AtomicUsize = AtomicUsize::new(0);
static SPOOL_COUNTER: AtomicUsize = AtomicUsize::new(0);
static HEARTBEAT_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();
static MEMORY_PROFILE_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn sweep_bed_root() -> PathBuf {
    if let Ok(root) = std::env::var("VARDICT_E2E_SWEEP_BED_ROOT") {
        PathBuf::from(root)
    } else {
        PathBuf::from("tmp/sweep_beds")
    }
}

/// Root directory for sweep fixtures (manifest + per-config output trees).
///
/// Honors `VARDICT_E2E_SWEEP_FIXTURE_ROOT`; falls back to `tmp/sweep_fixtures` so default
/// CI behavior is byte-identical when the env var is unset.
pub(crate) fn sweep_fixture_root() -> PathBuf {
    std::env::var("VARDICT_E2E_SWEEP_FIXTURE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tmp/sweep_fixtures"))
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TileKey {
    pub(crate) chrom: String,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

type Row = Box<str>;
type RowsByTile = BTreeMap<TileKey, Vec<Row>>;

#[derive(Clone, Debug)]
struct TileMismatch {
    config: String,
    key: TileKey,
    java: Vec<Row>,
    rust: Vec<Row>,
}

struct SweepScopeGuard;

impl Drop for SweepScopeGuard {
    fn drop(&mut self) {
        GlobalReadOnlyScope::clear_thread_local();
    }
}

struct TagContext {
    tag: String,
    config_name: String,
    scope_config: Configuration,
    reference_resource: ReferenceResource,
    chr_lengths: HashMap<String, i32>,
    chroms: Vec<String>,
    sample: String,
    cache_validation: OnceLock<Result<(), String>>,
    cache_skip_logged: AtomicBool,
}

struct ChunkPlan {
    trial_name: String,
    chrom: String,
    ordinal: usize,
    tiles: Arc<Vec<TileKey>>,
    start: usize,
    end: usize,
}

#[derive(Default)]
struct ChunkRuntimeTimings {
    cache_ms: Option<u128>,
    java_load_ms: Option<u128>,
    rust_run_ms: Option<u128>,
    diff_ms: Option<u128>,
}

pub fn reset_failure_count() {
    FAILURE_COUNT.store(0, Ordering::Relaxed);
}

pub fn legacy_selector_to_chunk_filter(selector: &str) -> Option<String> {
    let (module_name, trial_name) = selector.split_once("::")?;
    let tag = module_name.strip_suffix("_sweep")?;
    (trial_name == format!("parity_e2e_sweep_{tag}"))
        .then(|| format!("{module_name}::parity_e2e_sweep_{tag}_chr"))
}

pub fn build_trials(tag: &str) -> Vec<Trial> {
    let Some(context) = prepare_tag_context(tag) else {
        return Vec::new();
    };

    let shard = parse_shard_env();
    let plans = build_chunk_plans(&context, shard)
        .unwrap_or_else(|error| panic!("Failed to load sweep BED tiles for {tag}: {error}"));

    plans
        .into_iter()
        .map(|plan| {
            let context = Arc::clone(&context);
            let trial_name = plan.trial_name.clone();
            let display_name = plan.trial_name.clone();
            let chrom = plan.chrom.clone();
            let tiles = Arc::clone(&plan.tiles);

            // NOTE: Each libtest-mimic trial owns one chunk execution. Total RSS scales
            // roughly with `--test-threads` times the per-chunk working set.
            Trial::test(trial_name, move || {
                run_chunk_trial(
                    Arc::clone(&context),
                    chrom.clone(),
                    plan.ordinal,
                    Arc::clone(&tiles),
                    plan.start,
                    plan.end,
                    display_name.clone(),
                )
                .map_err(Failed::from)
            })
            .with_ignored_flag(true)
        })
        .collect()
}

fn prepare_tag_context(tag: &str) -> Option<Arc<TagContext>> {
    let (bam_path, ref_path) = bam_paths_for_tag(tag);
    let bam_path = PathBuf::from(bam_path);
    let ref_path = PathBuf::from(ref_path);
    let sample = sample_name_for_bam(&bam_path);
    let config_name = active_config();

    let chroms = match discover_chroms(tag) {
        Ok(chroms) => chroms,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => panic!("Failed to discover sweep chromosomes for {tag}: {error}"),
    };

    let bam_path_string = bam_path.to_string_lossy().into_owned();
    let ref_path_string = ref_path.to_string_lossy().into_owned();
    let fai_path = format!("{ref_path_string}.fai");
    let chr_lengths = super::common::load_chr_lengths(&fai_path);
    let scope_config = sweep_config(&config_name, &bam_path_string, &ref_path_string);
    let reference_resource =
        ReferenceResource::new(ref_path_string.clone(), 1200, 0, chr_lengths.clone(), false);

    Some(Arc::new(TagContext {
        tag: tag.to_string(),
        config_name,
        scope_config,
        reference_resource,
        chr_lengths,
        chroms,
        sample,
        cache_validation: OnceLock::new(),
        cache_skip_logged: AtomicBool::new(false),
    }))
}

fn build_chunk_plans(
    context: &TagContext,
    shard: Option<(u64, u64)>,
) -> io::Result<Vec<ChunkPlan>> {
    let mut plans = Vec::new();

    for chrom in &context.chroms {
        let tiles = Arc::new(load_tiles_for_chrom(&context.tag, chrom)?);
        for (ordinal, window) in tiles.chunks(CHUNK_SIZE).enumerate() {
            if let Some((index, total)) = shard {
                if chunk_id(&context.tag, chrom, ordinal as u64) % total != index {
                    continue;
                }
            }

            let start = ordinal * CHUNK_SIZE;
            let end = start + window.len();
            plans.push(ChunkPlan {
                trial_name: chunk_trial_name(&context.tag, chrom, ordinal),
                chrom: chrom.clone(),
                ordinal,
                tiles: Arc::clone(&tiles),
                start,
                end,
            });
        }
    }

    Ok(plans)
}

fn chunk_trial_name(tag: &str, chrom: &str, ordinal: usize) -> String {
    let chrom_label = chrom.strip_prefix("chr").unwrap_or(chrom);
    format!("{tag}_sweep::parity_e2e_sweep_{tag}_chr{chrom_label}_chunk{ordinal:03}")
}

fn run_chunk_trial(
    context: Arc<TagContext>,
    chrom: String,
    ordinal: usize,
    tiles: Arc<Vec<TileKey>>,
    start: usize,
    end: usize,
    trial_name: String,
) -> Result<(), String> {
    let trial_started = Instant::now();
    let mut timings = ChunkRuntimeTimings::default();
    let mut final_status = "passed";
    let region_str = chunk_region_str(&tiles[start..end]);
    emit_chunk_heartbeat(
        &context,
        &trial_name,
        &chrom,
        ordinal,
        "start",
        Some(&format!(
            "tiles={} window={start}..{end} region_str={}",
            end - start,
            heartbeat_escape(&region_str)
        )),
    );
    let result = catch_unwind(AssertUnwindSafe(|| {
        if FAILURE_COUNT.load(Ordering::Relaxed) >= MAX_FAILURES {
            final_status = "skipped";
            emit_chunk_heartbeat(
                &context,
                &trial_name,
                &chrom,
                ordinal,
                "cap-skip",
                Some(&format!("max_failures={MAX_FAILURES}")),
            );
            eprintln!("{trial_name}: skipped after MAX_FAILURES cap ({MAX_FAILURES})");
            return Ok(());
        }

        emit_chunk_heartbeat(&context, &trial_name, &chrom, ordinal, "cache-start", None);
        let cache_started = Instant::now();
        if let Err(error) = validate_tag_cache(&context) {
            timings.cache_ms = Some(elapsed_ms(cache_started));
            emit_chunk_heartbeat(
                &context,
                &trial_name,
                &chrom,
                ordinal,
                "cache-failed",
                Some(&format!("error={}", heartbeat_escape(&error))),
            );
            if is_ci() {
                final_status = "failed";
                return fail_or_skip_after_cap(
                    &trial_name,
                    format!(
                        "E2E sweep cache validation failed for {}: {error}",
                        context.tag
                    ),
                );
            }
            if !context.cache_skip_logged.swap(true, Ordering::Relaxed) {
                handle_missing_cache(&context.tag, &error);
            }
            final_status = "skipped";
            return Ok(());
        }
        timings.cache_ms = Some(elapsed_ms(cache_started));
        emit_chunk_heartbeat(&context, &trial_name, &chrom, ordinal, "cache-ok", None);

        let window = &tiles[start..end];
        let failures = if use_disk_backed_diff(&context.scope_config) {
            run_spooled_chunk_diff(
                &context,
                window,
                &trial_name,
                &chrom,
                ordinal,
                &mut timings,
                &mut final_status,
            )
        } else {
            run_in_memory_chunk_diff(
                &context,
                window,
                &trial_name,
                &chrom,
                ordinal,
                &mut timings,
                &mut final_status,
            )
        }?;
        if failures.is_empty() {
            return Ok(());
        }

        final_status = "failed";
        fail_or_skip_after_cap(&trial_name, format_report(&failures))
    }));

    match result {
        Ok(Ok(())) => {
            emit_chunk_heartbeat(
                &context,
                &trial_name,
                &chrom,
                ordinal,
                "end-ok",
                Some(&chunk_runtime_detail(
                    final_status,
                    &region_str,
                    elapsed_ms(trial_started),
                    &timings,
                    None,
                )),
            );
            Ok(())
        }
        Ok(Err(error)) => {
            emit_chunk_heartbeat(
                &context,
                &trial_name,
                &chrom,
                ordinal,
                "end-error",
                Some(&chunk_runtime_detail(
                    "failed",
                    &region_str,
                    elapsed_ms(trial_started),
                    &timings,
                    Some(&error),
                )),
            );
            Err(error)
        }
        Err(payload) => {
            emit_chunk_heartbeat(
                &context,
                &trial_name,
                &chrom,
                ordinal,
                "panic",
                Some(&chunk_runtime_detail(
                    "panic",
                    &region_str,
                    elapsed_ms(trial_started),
                    &timings,
                    None,
                )),
            );
            if let Some(message) = payload.downcast_ref::<&'static str>() {
                return Err((*message).to_string());
            }
            if let Some(message) = payload.downcast_ref::<String>() {
                return Err(message.clone());
            }
            std::panic::resume_unwind(payload);
        }
    }
}

fn elapsed_ms(started_at: Instant) -> u128 {
    started_at.elapsed().as_millis()
}

fn chunk_region_str(window: &[TileKey]) -> String {
    match (window.first(), window.last()) {
        (Some(first), Some(last)) if first.chrom == last.chrom => {
            format!("{}:{}-{}", first.chrom, first.start, last.end)
        }
        (Some(first), Some(last)) => format!(
            "{}:{}-{}:{}",
            first.chrom, first.start, last.chrom, last.end
        ),
        _ => "unavailable".to_string(),
    }
}

fn chunk_runtime_detail(
    status: &str,
    region_str: &str,
    total_ms: u128,
    timings: &ChunkRuntimeTimings,
    error: Option<&str>,
) -> String {
    let mut parts = vec![
        format!("status={status}"),
        format!("region_str={}", heartbeat_escape(region_str)),
        format!("total_ms={total_ms}"),
    ];
    push_timing_detail(&mut parts, "cache_ms", timings.cache_ms);
    push_timing_detail(&mut parts, "java_load_ms", timings.java_load_ms);
    push_timing_detail(&mut parts, "rust_run_ms", timings.rust_run_ms);
    push_timing_detail(&mut parts, "diff_ms", timings.diff_ms);
    if let Some(error) = error {
        parts.push(format!("error={}", heartbeat_escape(error)));
    }
    parts.join(" ")
}

fn push_timing_detail(parts: &mut Vec<String>, key: &str, value: Option<u128>) {
    if let Some(value) = value {
        parts.push(format!("{key}={value}"));
    }
}

fn fail_or_skip_after_cap(trial_name: &str, message: String) -> Result<(), String> {
    let previous = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed);
    if previous >= MAX_FAILURES {
        eprintln!("{trial_name}: skipped after MAX_FAILURES cap ({MAX_FAILURES})");
        return Ok(());
    }

    Err(message)
}

fn heartbeat_log() -> Option<&'static Mutex<File>> {
    HEARTBEAT_LOG
        .get_or_init(|| {
            let path = std::env::var("VARDICT_E2E_SWEEP_HEARTBEAT_LOG").ok()?;
            let path = path.trim();
            if path.is_empty() {
                return None;
            }
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    eprintln!(
                        "HEARTBEAT phase=init status=disabled reason=create-dir-failed path={} error={}",
                        path.display(),
                        heartbeat_escape(&error.to_string())
                    );
                    return None;
                }
            }
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => Some(Mutex::new(file)),
                Err(error) => {
                    eprintln!(
                        "HEARTBEAT phase=init status=disabled reason=open-failed path={} error={}",
                        path.display(),
                        heartbeat_escape(&error.to_string())
                    );
                    None
                }
            }
        })
        .as_ref()
}

fn memory_profile_log() -> Option<&'static Mutex<File>> {
    MEMORY_PROFILE_LOG
        .get_or_init(|| {
            let path = std::env::var("VARDICT_E2E_SWEEP_MEMORY_PROFILE").ok()?;
            let path = path.trim();
            if path.is_empty() {
                return None;
            }
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    eprintln!(
                        "MEMORY_PROFILE phase=init status=disabled reason=create-dir-failed path={} error={}",
                        path.display(),
                        heartbeat_escape(&error.to_string())
                    );
                    return None;
                }
            }
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => Some(Mutex::new(file)),
                Err(error) => {
                    eprintln!(
                        "MEMORY_PROFILE phase=init status=disabled reason=open-failed path={} error={}",
                        path.display(),
                        heartbeat_escape(&error.to_string())
                    );
                    None
                }
            }
        })
        .as_ref()
}

fn emit_memory_profile(
    context: &TagContext,
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
    phase: &str,
    java: Option<&RowsByTile>,
    rust: Option<&RowsByTile>,
    mismatches: Option<usize>,
) {
    emit_memory_profile_stats(
        context,
        trial_name,
        chrom,
        ordinal,
        phase,
        java.map(row_stats).unwrap_or_default(),
        rust.map(row_stats).unwrap_or_default(),
        mismatches,
    );
}

fn emit_memory_profile_stats(
    context: &TagContext,
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
    phase: &str,
    java_stats: RowStats,
    rust_stats: RowStats,
    mismatches: Option<usize>,
) {
    let Some(handle) = memory_profile_log() else {
        return;
    };

    let mut line = format!(
        "MEMORY_PROFILE ts={} phase={phase} config={} tag={} chrom={chrom} chunk={ordinal} trial={trial_name} vmrss_kib={} java_tiles={} java_rows={} java_row_bytes={} rust_tiles={} rust_rows={} rust_row_bytes={}",
        heartbeat_timestamp(),
        context.config_name,
        context.tag,
        current_rss_kib()
            .map(|rss| rss.to_string())
            .unwrap_or_else(|| "unavailable".to_string()),
        java_stats.tiles,
        java_stats.rows,
        java_stats.row_bytes,
        rust_stats.tiles,
        rust_stats.rows,
        rust_stats.row_bytes,
    );
    if let Some(mismatches) = mismatches {
        line.push_str(&format!(" mismatches={mismatches}"));
    }

    if let Ok(mut file) = handle.lock() {
        let _ = writeln!(file, "{line}");
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct RowStats {
    tiles: usize,
    rows: usize,
    row_bytes: usize,
}

fn row_stats(rows_by_tile: &RowsByTile) -> RowStats {
    RowStats {
        tiles: rows_by_tile.len(),
        rows: count_rows(rows_by_tile),
        row_bytes: rows_by_tile
            .values()
            .flat_map(|rows| rows.iter())
            .map(|row| row.len())
            .sum(),
    }
}

fn current_rss_kib() -> Option<usize> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse().ok()
    })
}

fn emit_chunk_heartbeat(
    context: &TagContext,
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
    phase: &str,
    detail: Option<&str>,
) {
    let mut line = format!(
        "HEARTBEAT ts={} phase={phase} config={} tag={} chrom={chrom} chunk={ordinal} trial={trial_name}",
        heartbeat_timestamp(),
        context.config_name,
        context.tag,
    );
    if let Some(detail) = detail {
        line.push(' ');
        line.push_str(detail);
    }

    eprintln!("{line}");
    if let Some(handle) = heartbeat_log() {
        if let Ok(mut file) = handle.lock() {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn heartbeat_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn heartbeat_escape(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("_")
}

fn count_rows(rows_by_tile: &RowsByTile) -> usize {
    rows_by_tile.values().map(Vec::len).sum()
}

fn validate_tag_cache(context: &TagContext) -> Result<(), String> {
    match context.cache_validation.get_or_init(|| {
        check_e2e_sweep_manifest(&context.config_name, &context.tag)?;
        for chrom in &context.chroms {
            let java_path = java_tsv_path(&context.tag, chrom, &context.config_name);
            if !java_path.is_file() {
                return Err(format!(
                    "Missing Java TSV cache for {}/{chrom} at {}",
                    context.tag,
                    java_path.display()
                ));
            }
        }
        Ok(())
    }) {
        Ok(()) => Ok(()),
        Err(error) => Err(error.clone()),
    }
}

fn bam_paths_for_tag(tag: &str) -> (&'static str, &'static str) {
    super::common::bam_tag_lookup(tag)
}

fn sample_name_for_bam(bam_path: &Path) -> String {
    bam_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| panic!("BAM path has no file stem: {}", bam_path.display()))
}

fn active_config() -> String {
    std::env::var("VARDICT_E2E_SWEEP_CONFIG")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

fn config_path_segment(config: &str) -> String {
    if config == "default" {
        String::new()
    } else {
        format!("{config}/")
    }
}

fn check_e2e_sweep_manifest(config: &str, tag: &str) -> Result<(), String> {
    let manifest_path = sweep_fixture_root().join("manifest.json");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("Failed to read {}: {error}", manifest_path.display()))?;
    let manifest_json: Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("Failed to parse {}: {error}", manifest_path.display()))?;

    let manifest_commit = manifest_json
        .get("vardictjava_commit")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing vardictjava_commit in {}", manifest_path.display()))?;
    let live_commit = live_vardictjava_commit()?;
    if manifest_commit != live_commit {
        return Err(format!(
            "Sweep cache commit mismatch: manifest={manifest_commit}, live={live_commit}"
        ));
    }

    let cache_key = format!("{config}:{tag}");
    let cache_entry = manifest_json
        .get("cache_entries")
        .and_then(Value::as_object)
        .and_then(|entries| entries.get(&cache_key))
        .ok_or_else(|| {
            format!(
                "Missing cache_entries[{cache_key}] in {}; regenerate the E2E sweep cache",
                manifest_path.display()
            )
        })?;

    compare_manifest_field(
        cache_entry,
        "bed_sha256",
        &compute_bed_fingerprint(tag).map_err(|error| error.to_string())?,
    )?;
    compare_manifest_field(
        cache_entry,
        "bam_stat",
        &compute_bam_stat(tag).map_err(|error| error.to_string())?,
    )?;
    compare_manifest_field(
        cache_entry,
        "reference_sha256",
        &compute_reference_fingerprint(tag).map_err(|error| error.to_string())?,
    )?;
    compare_manifest_field(
        cache_entry,
        "generator_flags_hash",
        &compute_generator_flags_hash(config, tag),
    )?;

    Ok(())
}

pub(crate) fn compare_manifest_field(
    entry: &Value,
    field: &str,
    expected: &Value,
) -> Result<(), String> {
    let actual = entry
        .get(field)
        .ok_or_else(|| format!("Missing {field} in cache entry"))?;
    if actual != expected {
        return Err(format!(
            "Cache manifest mismatch for {field}: expected {}, found {}",
            expected, actual
        ));
    }
    Ok(())
}

pub(crate) fn parse_shard_env() -> Option<(u64, u64)> {
    let value = std::env::var("VARDICT_E2E_SWEEP_SHARD").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (index, total) = trimmed
        .split_once('/')
        .unwrap_or_else(|| panic!("Invalid VARDICT_E2E_SWEEP_SHARD value: {trimmed}"));
    let index = index
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("Invalid shard index in VARDICT_E2E_SWEEP_SHARD: {trimmed}"));
    let total = total
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("Invalid shard count in VARDICT_E2E_SWEEP_SHARD: {trimmed}"));
    assert!(
        total > 0,
        "VARDICT_E2E_SWEEP_SHARD must have N > 0: {trimmed}"
    );
    assert!(
        index < total,
        "VARDICT_E2E_SWEEP_SHARD index must be < N: {trimmed}"
    );
    Some((index, total))
}

pub(crate) fn chunk_id(tag: &str, chrom: &str, ordinal: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    (tag, chrom, ordinal).hash(&mut hasher);
    hasher.finish()
}

fn discover_chroms(tag: &str) -> io::Result<Vec<String>> {
    let root = sweep_bed_root().join(tag);
    let mut chroms = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("bed") {
            continue;
        }
        let chrom = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .ok_or_else(|| invalid_data(format!("Invalid BED file name: {}", path.display())))?;
        chroms.push(chrom);
    }

    chroms.sort_by(|left, right| chrom_sort_key(left).cmp(&chrom_sort_key(right)));
    Ok(chroms)
}

fn load_tiles_for_chrom(tag: &str, chrom: &str) -> io::Result<Vec<TileKey>> {
    let bed_path = sweep_bed_root().join(tag).join(format!("{chrom}.bed"));
    let reader = BufReader::new(File::open(&bed_path)?);
    let mut tiles = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 3 {
            return Err(invalid_data(format!(
                "Expected at least 3 BED columns in {}: {line}",
                bed_path.display()
            )));
        }
        let start = fields[1].parse::<u32>().map_err(|error| {
            invalid_data(format!(
                "Invalid BED start in {}: {error}",
                bed_path.display()
            ))
        })?;
        let end = fields[2].parse::<u32>().map_err(|error| {
            invalid_data(format!(
                "Invalid BED end in {}: {error}",
                bed_path.display()
            ))
        })?;
        tiles.push(TileKey {
            chrom: fields[0].to_string(),
            start,
            end,
        });
    }

    Ok(tiles)
}

fn java_tsv_path(tag: &str, chrom: &str, config: &str) -> PathBuf {
    sweep_fixture_root()
        .join("output")
        .join(config_path_segment(config))
        .join(chrom)
        .join(format!("{tag}_{chrom}.tsv.zst"))
}

fn use_disk_backed_diff(config: &Configuration) -> bool {
    config.do_pileup
}

fn run_in_memory_chunk_diff(
    context: &TagContext,
    window: &[TileKey],
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
    timings: &mut ChunkRuntimeTimings,
    final_status: &mut &str,
) -> Result<Vec<TileMismatch>, String> {
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "java-load-start",
        Some(&format!("tiles={}", window.len())),
    );
    let java_started = Instant::now();
    let java_result = load_java_tsv_chunk(&context.tag, chrom, &context.config_name, window);
    timings.java_load_ms = Some(elapsed_ms(java_started));
    let java = java_result.map_err(|error| {
        *final_status = "failed";
        format!(
            "Failed to load cached Java TSV chunk for {}/{chrom} chunk {ordinal}: {error}",
            context.tag
        )
    })?;
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "java-load-ok",
        Some(&format!("rows={}", count_rows(&java))),
    );
    emit_memory_profile(
        context,
        trial_name,
        chrom,
        ordinal,
        "java-load-ok",
        Some(&java),
        None,
        None,
    );

    emit_chunk_heartbeat(context, trial_name, chrom, ordinal, "rust-run-start", None);
    let rust_started = Instant::now();
    let rust_result = run_rust_chunk(context, window, trial_name, chrom, ordinal);
    timings.rust_run_ms = Some(elapsed_ms(rust_started));
    let rust = rust_result.map_err(|error| {
        *final_status = "failed";
        format!(
            "Failed to run Rust chunk for {}/{chrom} chunk {ordinal}: {error}",
            context.tag
        )
    })?;
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-run-ok",
        Some(&format!("rows={}", count_rows(&rust))),
    );
    emit_memory_profile(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-run-ok",
        Some(&java),
        Some(&rust),
        None,
    );

    emit_chunk_heartbeat(context, trial_name, chrom, ordinal, "diff-start", None);
    emit_memory_profile(
        context,
        trial_name,
        chrom,
        ordinal,
        "diff-start",
        Some(&java),
        Some(&rust),
        None,
    );
    let diff_started = Instant::now();
    let failures = diff_chunk(&java, &rust, &context.config_name);
    timings.diff_ms = Some(elapsed_ms(diff_started));
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "diff-complete",
        Some(&format!("mismatches={}", failures.len())),
    );
    emit_memory_profile(
        context,
        trial_name,
        chrom,
        ordinal,
        "diff-complete",
        Some(&java),
        Some(&rust),
        Some(failures.len()),
    );
    Ok(failures)
}

fn run_spooled_chunk_diff(
    context: &TagContext,
    window: &[TileKey],
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
    timings: &mut ChunkRuntimeTimings,
    final_status: &mut &str,
) -> Result<Vec<TileMismatch>, String> {
    let spool = ChunkSpool::new(&context.tag, chrom, ordinal, window).map_err(|error| {
        *final_status = "failed";
        format!(
            "Failed to initialize disk-backed diff spool for {}/{chrom} chunk {ordinal}: {error}",
            context.tag
        )
    })?;

    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "java-load-start",
        Some(&format!("tiles={} mode=disk-spool", window.len())),
    );
    let java_started = Instant::now();
    let java_stats = spool
        .spool_java_tsv_chunk(&context.tag, chrom, &context.config_name)
        .map_err(|error| {
            *final_status = "failed";
            format!(
                "Failed to spool cached Java TSV chunk for {}/{chrom} chunk {ordinal}: {error}",
                context.tag
            )
        })?;
    timings.java_load_ms = Some(elapsed_ms(java_started));
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "java-load-ok",
        Some(&format!("rows={} mode=disk-spool", java_stats.rows)),
    );
    emit_memory_profile_stats(
        context,
        trial_name,
        chrom,
        ordinal,
        "java-load-ok",
        java_stats,
        RowStats::default(),
        None,
    );

    emit_chunk_heartbeat(context, trial_name, chrom, ordinal, "rust-run-start", None);
    let rust_started = Instant::now();
    let rust_stats = run_rust_chunk_to_spool(context, window, trial_name, chrom, ordinal, &spool)
        .map_err(|error| {
        *final_status = "failed";
        format!(
            "Failed to run Rust chunk for {}/{chrom} chunk {ordinal}: {error}",
            context.tag
        )
    })?;
    timings.rust_run_ms = Some(elapsed_ms(rust_started));
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-run-ok",
        Some(&format!("rows={} mode=disk-spool", rust_stats.rows)),
    );
    emit_memory_profile_stats(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-run-ok",
        java_stats,
        rust_stats,
        None,
    );

    emit_chunk_heartbeat(context, trial_name, chrom, ordinal, "diff-start", None);
    emit_memory_profile_stats(
        context,
        trial_name,
        chrom,
        ordinal,
        "diff-start",
        java_stats,
        rust_stats,
        None,
    );
    let diff_started = Instant::now();
    let failures = spool.diff_sorted(&context.config_name).map_err(|error| {
        *final_status = "failed";
        format!(
            "Failed to diff disk-backed rows for {}/{chrom} chunk {ordinal}: {error}",
            context.tag
        )
    })?;
    timings.diff_ms = Some(elapsed_ms(diff_started));
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "diff-complete",
        Some(&format!("mismatches={} mode=disk-spool", failures.len())),
    );
    emit_memory_profile_stats(
        context,
        trial_name,
        chrom,
        ordinal,
        "diff-complete",
        java_stats,
        rust_stats,
        Some(failures.len()),
    );
    Ok(failures)
}

fn load_java_tsv_chunk(
    tag: &str,
    chrom: &str,
    config: &str,
    chunk_tiles: &[TileKey],
) -> io::Result<RowsByTile> {
    let path = java_tsv_path(tag, chrom, config);
    let decoder = zstd::stream::read::Decoder::new(File::open(&path)?)?;
    let reader = BufReader::new(decoder);
    let mut rows_by_tile = ChunkRowBuckets::new(chunk_tiles);
    let mut region_index = None;
    let mut line = String::new();

    let mut reader = reader;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        strip_line_ending(&mut line);
        if line.trim().is_empty() {
            continue;
        }

        let region = if let Some(region_index) = region_index {
            tab_field(&line, region_index).ok_or_else(|| {
                invalid_data(format!(
                    "Missing Region column {region_index} in {} row: {line}",
                    path.display()
                ))
            })?
        } else {
            let columns: Vec<&str> = line.split('\t').collect();
            if let Some(index) = columns
                .iter()
                .position(|field| field.eq_ignore_ascii_case("Region"))
            {
                region_index = Some(index);
                continue;
            }
            let index = detect_region_column(&columns).ok_or_else(|| {
                invalid_data(format!(
                    "Could not locate Region column in {} from row: {line}",
                    path.display()
                ))
            })?;
            region_index = Some(index);
            columns[index]
        };
        let row_index = rows_by_tile.tile_index_for_region(region)?;
        if let Some(row_index) = row_index {
            rows_by_tile.push_owned_at(row_index, std::mem::take(&mut line));
        }
    }

    Ok(rows_by_tile.into_btree_map())
}

struct ChunkRowBuckets {
    tiles: Vec<TileKey>,
    rows: Vec<Vec<Row>>,
    tile_index_by_region: HashMap<String, usize>,
}

impl ChunkRowBuckets {
    fn new(chunk_tiles: &[TileKey]) -> Self {
        let unique_tiles: Vec<_> = chunk_tiles
            .iter()
            .cloned()
            .map(|tile| (tile, ()))
            .collect::<BTreeMap<_, _>>()
            .into_keys()
            .collect();
        let mut tile_index_by_region = HashMap::with_capacity(unique_tiles.len());
        for (index, tile) in unique_tiles.iter().enumerate() {
            tile_index_by_region.insert(tile.to_region_string(), index);
        }

        Self {
            rows: (0..unique_tiles.len()).map(|_| Vec::new()).collect(),
            tiles: unique_tiles,
            tile_index_by_region,
        }
    }

    fn push_owned_at(&mut self, index: usize, line: String) {
        self.rows[index].push(line.into_boxed_str());
    }

    fn tile_index_for_region(&self, region: &str) -> io::Result<Option<usize>> {
        if !looks_like_tile_key(region) {
            return Err(invalid_data(format!(
                "Invalid Region column value: {region}"
            )));
        }
        Ok(self.tile_index_by_region.get(region).copied())
    }

    fn into_btree_map(self) -> RowsByTile {
        self.tiles.into_iter().zip(self.rows).collect()
    }
}

impl TileKey {
    fn to_region_string(&self) -> String {
        format!("{}:{}-{}", self.chrom, self.start, self.end)
    }
}

struct ChunkSpool {
    root: PathBuf,
    java_unsorted: PathBuf,
    java_sorted: PathBuf,
    rust_unsorted: PathBuf,
    rust_sorted: PathBuf,
    buckets: ChunkRowBuckets,
    keep_files: bool,
}

impl ChunkSpool {
    fn new(tag: &str, chrom: &str, ordinal: usize, chunk_tiles: &[TileKey]) -> io::Result<Self> {
        let root = spool_root(tag, chrom, ordinal);
        fs::create_dir_all(&root)?;
        Ok(Self {
            java_unsorted: root.join("java.unsorted.tsv"),
            java_sorted: root.join("java.sorted.tsv"),
            rust_unsorted: root.join("rust.unsorted.tsv"),
            rust_sorted: root.join("rust.sorted.tsv"),
            buckets: ChunkRowBuckets::new(chunk_tiles),
            keep_files: matches!(
                std::env::var("VARDICT_E2E_SWEEP_KEEP_SPOOL"),
                Ok(value) if value == "1" || value.eq_ignore_ascii_case("true")
            ),
            root,
        })
    }

    fn spool_java_tsv_chunk(&self, tag: &str, chrom: &str, config: &str) -> io::Result<RowStats> {
        let path = java_tsv_path(tag, chrom, config);
        let decoder = zstd::stream::read::Decoder::new(File::open(&path)?)?;
        let mut reader = BufReader::new(decoder);
        let mut writer = BufWriter::new(File::create(&self.java_unsorted)?);
        let mut stats = RowStats {
            tiles: self.buckets.tiles.len(),
            ..RowStats::default()
        };
        let mut region_index = None;
        let mut line = String::new();

        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            strip_line_ending(&mut line);
            if line.trim().is_empty() {
                continue;
            }

            let region = if let Some(region_index) = region_index {
                tab_field(&line, region_index).ok_or_else(|| {
                    invalid_data(format!(
                        "Missing Region column {region_index} in {} row: {line}",
                        path.display()
                    ))
                })?
            } else {
                let columns: Vec<&str> = line.split('\t').collect();
                if let Some(index) = columns
                    .iter()
                    .position(|field| field.eq_ignore_ascii_case("Region"))
                {
                    region_index = Some(index);
                    continue;
                }
                let index = detect_region_column(&columns).ok_or_else(|| {
                    invalid_data(format!(
                        "Could not locate Region column in {} from row: {line}",
                        path.display()
                    ))
                })?;
                region_index = Some(index);
                columns[index]
            };

            if self.buckets.tile_index_for_region(region)?.is_some() {
                write_spool_record(&mut writer, region, &line)?;
                stats.rows += 1;
                stats.row_bytes += line.len();
            }
        }

        writer.flush()?;
        sort_spool_file(&self.java_unsorted, &self.java_sorted, &self.root)?;
        Ok(stats)
    }

    fn open_rust_writer(&self) -> io::Result<BufWriter<File>> {
        File::create(&self.rust_unsorted).map(BufWriter::new)
    }

    fn sort_rust(&self) -> io::Result<()> {
        sort_spool_file(&self.rust_unsorted, &self.rust_sorted, &self.root)
    }

    fn diff_sorted(&self, config: &str) -> io::Result<Vec<TileMismatch>> {
        let mut java = SortedSpoolReader::new(&self.java_sorted)?;
        let mut rust = SortedSpoolReader::new(&self.rust_sorted)?;
        let mut java_group = java.next_group()?;
        let mut rust_group = rust.next_group()?;
        let mut failures = Vec::new();

        while failures.len() < MAX_FAILURES {
            match (java_group.take(), rust_group.take()) {
                (Some((java_key, java_rows)), Some((rust_key, rust_rows))) => {
                    match java_key.cmp(&rust_key) {
                        std::cmp::Ordering::Equal => {
                            if java_rows != rust_rows {
                                failures.push(TileMismatch {
                                    config: config.to_string(),
                                    key: java_key,
                                    java: java_rows,
                                    rust: rust_rows,
                                });
                            }
                            java_group = java.next_group()?;
                            rust_group = rust.next_group()?;
                        }
                        std::cmp::Ordering::Less => {
                            failures.push(TileMismatch {
                                config: config.to_string(),
                                key: java_key,
                                java: java_rows,
                                rust: Vec::new(),
                            });
                            java_group = java.next_group()?;
                            rust_group = Some((rust_key, rust_rows));
                        }
                        std::cmp::Ordering::Greater => {
                            failures.push(TileMismatch {
                                config: config.to_string(),
                                key: rust_key,
                                java: Vec::new(),
                                rust: rust_rows,
                            });
                            java_group = Some((java_key, java_rows));
                            rust_group = rust.next_group()?;
                        }
                    }
                }
                (Some((java_key, java_rows)), None) => {
                    failures.push(TileMismatch {
                        config: config.to_string(),
                        key: java_key,
                        java: java_rows,
                        rust: Vec::new(),
                    });
                    java_group = java.next_group()?;
                }
                (None, Some((rust_key, rust_rows))) => {
                    failures.push(TileMismatch {
                        config: config.to_string(),
                        key: rust_key,
                        java: Vec::new(),
                        rust: rust_rows,
                    });
                    rust_group = rust.next_group()?;
                }
                (None, None) => break,
            }
        }

        Ok(failures)
    }
}

impl Drop for ChunkSpool {
    fn drop(&mut self) {
        if !self.keep_files {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

struct SortedSpoolReader {
    reader: BufReader<File>,
    pending: Option<String>,
}

impl SortedSpoolReader {
    fn new(path: &Path) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::new(File::open(path)?),
            pending: None,
        })
    }

    fn next_group(&mut self) -> io::Result<Option<(TileKey, Vec<Row>)>> {
        let mut line = match self.pending.take() {
            Some(line) => line,
            None => {
                let mut line = String::new();
                if self.reader.read_line(&mut line)? == 0 {
                    return Ok(None);
                }
                strip_line_ending(&mut line);
                line
            }
        };

        let (key, row) = split_spool_record(&line)?;
        let key = key.to_string();
        let tile = parse_region_column_value(&key)?;
        let mut rows = vec![row.to_string().into_boxed_str()];

        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                break;
            }
            strip_line_ending(&mut line);
            let (next_key, next_row) = split_spool_record(&line)?;
            if next_key == key {
                rows.push(next_row.to_string().into_boxed_str());
            } else {
                self.pending = Some(line);
                break;
            }
        }

        Ok(Some((tile, rows)))
    }
}

struct RustChunkSpool {
    writer: BufWriter<File>,
    buckets: ChunkRowBuckets,
    region_index: Option<usize>,
    stats: RowStats,
    error: Option<String>,
}

impl RustChunkSpool {
    fn new(spool: &ChunkSpool) -> io::Result<Self> {
        Ok(Self {
            writer: spool.open_rust_writer()?,
            buckets: ChunkRowBuckets::new(&spool.buckets.tiles),
            region_index: None,
            stats: RowStats {
                tiles: spool.buckets.tiles.len(),
                ..RowStats::default()
            },
            error: None,
        })
    }

    fn push_owned_line(&mut self, line: String) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.try_push_owned_line(&line) {
            self.error = Some(error.to_string());
        }
    }

    fn try_push_owned_line(&mut self, line: &str) -> io::Result<()> {
        if line.trim().is_empty() {
            return Ok(());
        }

        let region = if let Some(region_index) = self.region_index {
            tab_field(line, region_index).ok_or_else(|| {
                invalid_data(format!(
                    "Missing Region column {region_index} in Rust output row: {line}"
                ))
            })?
        } else {
            let columns: Vec<&str> = line.split('\t').collect();
            let region_index = detect_region_column(&columns).ok_or_else(|| {
                invalid_data(format!(
                    "Could not locate Region column in Rust output row: {line}"
                ))
            })?;
            self.region_index = Some(region_index);
            columns[region_index]
        };

        if self.buckets.tile_index_for_region(region)?.is_some() {
            write_spool_record(&mut self.writer, region, line)?;
            self.stats.rows += 1;
            self.stats.row_bytes += line.len();
        }
        Ok(())
    }

    fn finish(mut self) -> io::Result<RowStats> {
        self.writer.flush()?;
        if let Some(error) = self.error {
            Err(invalid_data(error))
        } else {
            Ok(self.stats)
        }
    }
}

fn spool_root(tag: &str, chrom: &str, ordinal: usize) -> PathBuf {
    let counter = SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = std::env::var_os("VARDICT_E2E_SWEEP_SPOOL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tmp/parity_e2e_sweep_spool"));
    base.join(format!(
        "{}-{}-{}-{}-{}-{}",
        std::process::id(),
        heartbeat_timestamp(),
        counter,
        heartbeat_escape(tag),
        heartbeat_escape(chrom),
        ordinal
    ))
}

fn write_spool_record(writer: &mut BufWriter<File>, region: &str, row: &str) -> io::Result<()> {
    writer.write_all(region.as_bytes())?;
    writer.write_all(b"\t")?;
    writer.write_all(row.as_bytes())?;
    writer.write_all(b"\n")
}

fn sort_spool_file(input: &Path, output: &Path, temp_dir: &Path) -> io::Result<()> {
    let output_status = Command::new("sort")
        .env("LC_ALL", "C")
        .env("TMPDIR", temp_dir)
        .arg(input)
        .arg("-o")
        .arg(output)
        .status()?;
    if output_status.success() {
        Ok(())
    } else {
        Err(invalid_data(format!(
            "sort failed for {} with status {output_status}",
            input.display()
        )))
    }
}

fn split_spool_record(line: &str) -> io::Result<(&str, &str)> {
    split_once_byte(line, b'\t')
        .ok_or_else(|| invalid_data(format!("Invalid spool record without tab: {line}")))
}

struct RustChunkRows {
    rows_by_tile: ChunkRowBuckets,
    region_index: Option<usize>,
    error: Option<String>,
}

impl RustChunkRows {
    fn new(tiles: &[TileKey]) -> Self {
        Self {
            rows_by_tile: ChunkRowBuckets::new(tiles),
            region_index: None,
            error: None,
        }
    }

    fn push_owned_line(&mut self, line: String) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.try_push_owned_line(line) {
            self.error = Some(error.to_string());
        }
    }

    fn try_push_owned_line(&mut self, line: String) -> io::Result<()> {
        if line.trim().is_empty() {
            return Ok(());
        }

        let row_index = {
            let region = if let Some(region_index) = self.region_index {
                tab_field(&line, region_index).ok_or_else(|| {
                    invalid_data(format!(
                        "Missing Region column {region_index} in Rust output row: {line}"
                    ))
                })?
            } else {
                let columns: Vec<&str> = line.split('\t').collect();
                let region_index = detect_region_column(&columns).ok_or_else(|| {
                    invalid_data(format!(
                        "Could not locate Region column in Rust output row: {line}"
                    ))
                })?;
                self.region_index = Some(region_index);
                columns[region_index]
            };
            self.rows_by_tile.tile_index_for_region(region)?
        };

        if let Some(row_index) = row_index {
            self.rows_by_tile.push_owned_at(row_index, line);
        }
        Ok(())
    }

    fn finish(self) -> io::Result<RowsByTile> {
        if let Some(error) = self.error {
            Err(invalid_data(error))
        } else {
            Ok(self.rows_by_tile.into_btree_map())
        }
    }
}

fn run_rust_chunk(
    context: &TagContext,
    tiles: &[TileKey],
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
) -> io::Result<RowsByTile> {
    let regions = build_regions(tiles)?;
    let _guard = init_sweep_scope(
        context.scope_config.clone(),
        context.chr_lengths.clone(),
        &context.sample,
    );
    let rows = Arc::new(Mutex::new(RustChunkRows::new(tiles)));
    let sink_rows = rows.clone();
    let simple_mode = SimpleMode::new(vec![regions], context.reference_resource.clone());
    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::OwnedLineSink(Arc::new(
        move |line| {
            let mut rows = sink_rows.lock().unwrap_or_else(|error| error.into_inner());
            rows.push_owned_line(line);
        },
    )));
    let _budget_guard = super::common::thread_budget()
        .acquire(super::common::config_budget_cost(&context.scope_config));
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-simple-start",
        None,
    );
    if let Some(thread_count) = super::common::configured_thread_count(context.scope_config.threads)
    {
        simple_mode.parallel(thread_count);
    } else {
        simple_mode.not_parallel();
    }
    emit_chunk_heartbeat(context, trial_name, chrom, ordinal, "rust-simple-end", None);

    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Err);
    let rows = match Arc::try_unwrap(rows) {
        Ok(rows) => rows.into_inner().unwrap_or_else(|error| error.into_inner()),
        Err(rows) => {
            let mut rows = rows.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::replace(&mut *rows, RustChunkRows::new(&[]))
        }
    };
    rows.finish()
}

fn run_rust_chunk_to_spool(
    context: &TagContext,
    tiles: &[TileKey],
    trial_name: &str,
    chrom: &str,
    ordinal: usize,
    spool: &ChunkSpool,
) -> io::Result<RowStats> {
    let regions = build_regions(tiles)?;
    let _guard = init_sweep_scope(
        context.scope_config.clone(),
        context.chr_lengths.clone(),
        &context.sample,
    );
    let rows = Arc::new(Mutex::new(RustChunkSpool::new(spool)?));
    let sink_rows = rows.clone();
    let simple_mode = SimpleMode::new(vec![regions], context.reference_resource.clone());
    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::OwnedLineSink(Arc::new(
        move |line| {
            let mut rows = sink_rows.lock().unwrap_or_else(|error| error.into_inner());
            rows.push_owned_line(line);
        },
    )));
    let _budget_guard = super::common::thread_budget()
        .acquire(super::common::config_budget_cost(&context.scope_config));
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-simple-start",
        Some("mode=disk-spool"),
    );
    if let Some(thread_count) = super::common::configured_thread_count(context.scope_config.threads)
    {
        simple_mode.parallel(thread_count);
    } else {
        simple_mode.not_parallel();
    }
    emit_chunk_heartbeat(
        context,
        trial_name,
        chrom,
        ordinal,
        "rust-simple-end",
        Some("mode=disk-spool"),
    );

    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Err);
    let rows = match Arc::try_unwrap(rows) {
        Ok(rows) => rows.into_inner().unwrap_or_else(|error| error.into_inner()),
        Err(rows) => {
            let mut rows = rows.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::replace(&mut *rows, RustChunkSpool::new(spool)?)
        }
    };
    let stats = rows.finish()?;
    spool.sort_rust()?;
    Ok(stats)
}

fn diff_chunk(java: &RowsByTile, rust: &RowsByTile, config: &str) -> Vec<TileMismatch> {
    let mut failures = Vec::new();
    let keys: BTreeSet<_> = java.keys().chain(rust.keys()).cloned().collect();

    for key in keys {
        let mut java_rows = java.get(&key).cloned().unwrap_or_default();
        let mut rust_rows = rust.get(&key).cloned().unwrap_or_default();
        java_rows.sort();
        rust_rows.sort();
        if java_rows != rust_rows {
            failures.push(TileMismatch {
                config: config.to_string(),
                key,
                java: java_rows,
                rust: rust_rows,
            });
        }
        if failures.len() >= MAX_FAILURES {
            break;
        }
    }

    failures
}

fn format_report(failures: &[TileMismatch]) -> String {
    let mut report = format!(
        "E2E sweep mismatches: {} tile(s) differ; showing up to {MAX_FAILURES}.",
        failures.len()
    );

    for (index, failure) in failures.iter().take(MAX_FAILURES).enumerate() {
        report.push_str(&format!(
            "\n\n{}. config={} tile={}:{}-{}\nJava rows:\n{}\nRust rows:\n{}",
            index + 1,
            failure.config,
            failure.key.chrom,
            failure.key.start,
            failure.key.end,
            format_rows(&failure.java),
            format_rows(&failure.rust),
        ));
    }

    report
}

fn build_regions(tiles: &[TileKey]) -> io::Result<Vec<Region>> {
    tiles
        .iter()
        .map(|tile| {
            let start = i32::try_from(tile.start).map_err(|_| {
                invalid_data(format!("Tile start does not fit in i32: {}", tile.start))
            })?;
            let end = i32::try_from(tile.end)
                .map_err(|_| invalid_data(format!("Tile end does not fit in i32: {}", tile.end)))?;
            Ok(Region::new(
                tile.chrom.clone(),
                start,
                end,
                tile.chrom.clone(),
            ))
        })
        .collect()
}

fn sweep_config(config_name: &str, bam_path: &str, ref_path: &str) -> Configuration {
    let mut config = if config_name == "default" {
        Configuration::default()
    } else {
        super::common::config_preset(config_name)
    };
    config.bam = Some(BamNames::new(bam_path));
    config.fasta = ref_path.to_string();
    config.sample_name = Some(sample_name_for_bam(Path::new(bam_path)));
    config
}

fn init_sweep_scope(
    config: Configuration,
    chr_lengths: HashMap<String, i32>,
    sample: &str,
) -> SweepScopeGuard {
    GlobalReadOnlyScope::clear_thread_local();
    GlobalReadOnlyScope::init_thread_local(
        config,
        chr_lengths,
        sample,
        None,
        None,
        HashMap::new(),
        HashMap::new(),
    );
    SweepScopeGuard
}

fn tab_field(line: &str, index: usize) -> Option<&str> {
    let mut field_start = 0;
    let mut field_index = 0;
    for (position, byte) in line.bytes().enumerate() {
        if byte == b'\t' {
            if field_index == index {
                return Some(&line[field_start..position]);
            }
            field_index += 1;
            field_start = position + 1;
        }
    }
    (field_index == index).then(|| &line[field_start..])
}

fn strip_line_ending(line: &mut String) {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
}

pub(crate) fn detect_region_column(columns: &[&str]) -> Option<usize> {
    columns
        .iter()
        .position(|field| parse_tile_key(field).is_some())
}

#[allow(dead_code)]
pub(crate) fn parse_region_column_value(value: &str) -> io::Result<TileKey> {
    parse_tile_key(value)
        .ok_or_else(|| invalid_data(format!("Invalid Region column value: {value}")))
}

pub(crate) fn parse_tile_key(value: &str) -> Option<TileKey> {
    let (chrom, start, end) = parse_tile_key_parts(value)?;
    Some(TileKey {
        chrom: chrom.to_string(),
        start,
        end,
    })
}

fn looks_like_tile_key(value: &str) -> bool {
    parse_tile_key_parts(value).is_some()
}

fn parse_tile_key_parts(value: &str) -> Option<(&str, u32, u32)> {
    let (chrom, range) = split_once_byte(value, b':')?;
    let (start, end) = split_once_byte(range, b'-')?;
    Some((chrom, start.parse().ok()?, end.parse().ok()?))
}

fn split_once_byte(value: &str, needle: u8) -> Option<(&str, &str)> {
    let position = value.bytes().position(|byte| byte == needle)?;
    Some((&value[..position], &value[position + 1..]))
}

pub(crate) fn live_vardictjava_commit() -> Result<String, String> {
    let output = Command::new("git")
        .args(["-C", "VarDictJava", "rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("Failed to run git for VarDictJava commit: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Failed to resolve VarDictJava commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let commit = String::from_utf8(output.stdout)
        .map_err(|error| format!("VarDictJava commit output was not valid UTF-8: {error}"))?;
    Ok(commit.trim().to_string())
}

fn compute_bed_fingerprint(tag: &str) -> io::Result<Value> {
    let bed_root = sweep_bed_root().join(tag);
    let mut bed_files = Vec::new();
    for entry in fs::read_dir(&bed_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("bed") {
            bed_files.push(path);
        }
    }
    bed_files.sort();

    let mut hasher = Sha256::new();
    for path in bed_files {
        let mut file = File::open(&path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        hasher.update(&bytes);
    }

    Ok(Value::String(format!("{:x}", hasher.finalize())))
}

fn compute_bam_stat(tag: &str) -> io::Result<Value> {
    let (bam_path, _) = bam_paths_for_tag(tag);
    let metadata = fs::metadata(bam_path)?;
    let modified = metadata
        .modified()
        .map_err(io::Error::other)?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();

    Ok(serde_json::json!([
        {
            "path": bam_path,
            "size": metadata.len(),
            "mtime_unix": modified,
        }
    ]))
}

fn compute_reference_fingerprint(tag: &str) -> io::Result<Value> {
    let (_, ref_path) = bam_paths_for_tag(tag);
    let fai_path = format!("{ref_path}.fai");
    let mut file = File::open(&fai_path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(Value::String(format!("{:x}", hasher.finalize())))
}

fn compute_generator_flags_hash(config: &str, tag: &str) -> Value {
    let sweep_bed_root = sweep_bed_root();
    let normalized = format!(
        "--output-only --config {config} --tags {tag} --sweep-bed-root {}",
        sweep_bed_root.display()
    );
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    Value::String(format!("{:x}", hasher.finalize()))
}

pub(crate) fn chrom_sort_key(chrom: &str) -> (u8, u32, &str) {
    match chrom.parse::<u32>() {
        Ok(value) => (0, value, ""),
        Err(_) => (1, 0, chrom),
    }
}

fn handle_missing_cache(tag: &str, reason: &str) {
    if is_ci() {
        panic!("E2E sweep cache validation failed for {tag}: {reason}");
    }

    eprintln!("Skipping parity_e2e_sweep for {tag}: {reason}");
}

fn is_ci() -> bool {
    matches!(std::env::var("CI"), Ok(value) if value == "true")
}

fn format_rows(rows: &[Row]) -> String {
    if rows.is_empty() {
        "<empty>".to_string()
    } else {
        let total_len = rows.iter().map(|row| row.len()).sum::<usize>() + rows.len() - 1;
        let mut output = String::with_capacity(total_len);
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                output.push('\n');
            }
            output.push_str(row);
        }
        output
    }
}

pub(crate) fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
