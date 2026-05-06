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
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;

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

#[derive(Clone, Debug)]
struct TileMismatch {
    config: String,
    key: TileKey,
    java: Vec<String>,
    rust: Vec<String>,
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
    let result = catch_unwind(AssertUnwindSafe(|| {
        if FAILURE_COUNT.load(Ordering::Relaxed) >= MAX_FAILURES {
            eprintln!("{trial_name}: skipped after MAX_FAILURES cap ({MAX_FAILURES})");
            return Ok(());
        }

        if let Err(error) = validate_tag_cache(&context) {
            if is_ci() {
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
            return Ok(());
        }

        let window = &tiles[start..end];
        let java = load_java_tsv_chunk(&context.tag, &chrom, &context.config_name, window)
            .map_err(|error| {
                format!(
                    "Failed to load cached Java TSV chunk for {}/{chrom} chunk {ordinal}: {error}",
                    context.tag
                )
            })?;
        let rust = run_rust_chunk(&context, window).map_err(|error| {
            format!(
                "Failed to run Rust chunk for {}/{chrom} chunk {ordinal}: {error}",
                context.tag
            )
        })?;
        let failures = diff_chunk(&java, &rust, &context.config_name);
        if failures.is_empty() {
            return Ok(());
        }

        fail_or_skip_after_cap(&trial_name, format_report(&failures))
    }));

    match result {
        Ok(result) => result,
        Err(payload) => {
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

fn fail_or_skip_after_cap(trial_name: &str, message: String) -> Result<(), String> {
    let previous = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed);
    if previous >= MAX_FAILURES {
        eprintln!("{trial_name}: skipped after MAX_FAILURES cap ({MAX_FAILURES})");
        return Ok(());
    }

    Err(message)
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

fn load_java_tsv_chunk(
    tag: &str,
    chrom: &str,
    config: &str,
    chunk_tiles: &[TileKey],
) -> io::Result<BTreeMap<TileKey, Vec<String>>> {
    let path = java_tsv_path(tag, chrom, config);
    let decoder = zstd::stream::read::Decoder::new(File::open(&path)?)?;
    let reader = BufReader::new(decoder);
    let mut rows_by_tile = empty_tile_map(chunk_tiles);
    let mut region_index = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let columns: Vec<&str> = line.split('\t').collect();
        if region_index.is_none() {
            if let Some(index) = columns
                .iter()
                .position(|field| field.eq_ignore_ascii_case("Region"))
            {
                region_index = Some(index);
                continue;
            }
            region_index = Some(detect_region_column(&columns).ok_or_else(|| {
                invalid_data(format!(
                    "Could not locate Region column in {} from row: {line}",
                    path.display()
                ))
            })?);
        }

        let tile = parse_region_column_value(columns[region_index.unwrap()])?;
        if let Some(rows) = rows_by_tile.get_mut(&tile) {
            rows.push(line);
        }
    }

    Ok(rows_by_tile)
}

fn run_rust_chunk(
    context: &TagContext,
    tiles: &[TileKey],
) -> io::Result<BTreeMap<TileKey, Vec<String>>> {
    let regions = build_regions(tiles)?;
    let _guard = init_sweep_scope(
        context.scope_config.clone(),
        context.chr_lengths.clone(),
        &context.sample,
    );
    let captured = Arc::new(Mutex::new(String::new()));
    let simple_mode = SimpleMode::new(vec![regions], context.reference_resource.clone());
    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Buffer(captured.clone()));
    simple_mode.not_parallel();

    let output = super::common::take_captured_output(&captured);
    let mut rows_by_tile = empty_tile_map(tiles);
    if output.trim().is_empty() {
        return Ok(rows_by_tile);
    }

    let mut region_index = None;
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let columns: Vec<&str> = line.split('\t').collect();
        if region_index.is_none() {
            region_index = Some(detect_region_column(&columns).ok_or_else(|| {
                invalid_data(format!(
                    "Could not locate Region column in Rust output row: {line}"
                ))
            })?);
        }

        let tile = parse_region_column_value(columns[region_index.unwrap()])?;
        if let Some(rows) = rows_by_tile.get_mut(&tile) {
            rows.push(line.to_string());
        }
    }

    Ok(rows_by_tile)
}

fn diff_chunk(
    java: &BTreeMap<TileKey, Vec<String>>,
    rust: &BTreeMap<TileKey, Vec<String>>,
    config: &str,
) -> Vec<TileMismatch> {
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

fn empty_tile_map(chunk_tiles: &[TileKey]) -> BTreeMap<TileKey, Vec<String>> {
    chunk_tiles
        .iter()
        .cloned()
        .map(|tile| (tile, Vec::new()))
        .collect()
}

pub(crate) fn detect_region_column(columns: &[&str]) -> Option<usize> {
    columns
        .iter()
        .position(|field| parse_tile_key(field).is_some())
}

pub(crate) fn parse_region_column_value(value: &str) -> io::Result<TileKey> {
    parse_tile_key(value)
        .ok_or_else(|| invalid_data(format!("Invalid Region column value: {value}")))
}

pub(crate) fn parse_tile_key(value: &str) -> Option<TileKey> {
    let (chrom, range) = value.split_once(':')?;
    let (start, end) = range.split_once('-')?;
    Some(TileKey {
        chrom: chrom.to_string(),
        start: start.parse().ok()?,
        end: end.parse().ok()?,
    })
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

fn format_rows(rows: &[String]) -> String {
    if rows.is_empty() {
        "<empty>".to_string()
    } else {
        rows.join("\n")
    }
}

pub(crate) fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
