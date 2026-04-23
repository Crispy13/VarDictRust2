//! Full-BAM end-to-end parity harness against cached Java TSV shards.
//!
//! This module streams large BAM-backed regions in chunked tile windows, dispatches by BAM tag,
//! and compares Rust output against the pre-generated Java cache under
//! `tmp/sweep_fixtures/output/`.
//!
//! Prerequisites:
//! - `tmp/sweep_fixtures/output/` must exist and contain a valid `manifest.json`.
//! - Regenerate the cache with `bash scripts/gen_e2e_sweep_golden.sh` when fixtures are missing
//!   or stale.
//!
//! Environment:
//! - `VARDICT_E2E_SWEEP_CONFIG=<name>` selects the cache layout; defaults to `default`.
//! - `VARDICT_E2E_SWEEP_SHARD=i/N` optionally runs only one shard of the tile set.
//! - `CI=true` converts missing-cache handling into a hard panic instead of a local skip.
//!
//! Run with:
//! `cargo test --profile debug-release --test parity_e2e_sweep -- --include-ignored --test-threads=1`
//!
//! To add a new BAM tag:
//! 1. Add the lookup entry in `tests/common/mod.rs::bam_tag_lookup`.
//! 2. Add a `<tag>_sweep.rs` stub plus its `#[path]` line in `tests/parity_e2e_sweep.rs`.
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
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde_json::Value;
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

pub const MAX_FAILURES: usize = 10;
pub const CHUNK_SIZE: usize = 20_000;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TileKey {
    chrom: String,
    start: u32,
    end: u32,
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
        GlobalReadOnlyScope::clear();
    }
}

pub fn run_tag(tag: &str) {
    let (bam_path, ref_path) = bam_paths_for_tag(tag);
    let bam = PathBuf::from(bam_path);
    let ref_path = PathBuf::from(ref_path);
    let sample = sample_name_for_bam(&bam);
    let config = active_config();

    if let Err(error) = check_e2e_sweep_manifest(&config, tag) {
        handle_missing_cache(tag, &error);
        return;
    }

    let chroms = discover_chroms(tag)
        .unwrap_or_else(|error| panic!("Failed to discover sweep chromosomes for {tag}: {error}"));

    for chrom in &chroms {
        let java_path = java_tsv_path(tag, chrom, &config);
        if !java_path.is_file() {
            handle_missing_cache(
                tag,
                &format!("Missing Java TSV cache for {tag}/{chrom} at {}", java_path.display()),
            );
            return;
        }
    }

    let shard = parse_shard_env();
    let mut failures = Vec::new();

    for chrom in chroms {
        let tiles = load_tiles_for_chrom(tag, &chrom)
            .unwrap_or_else(|error| panic!("Failed to load sweep BED tiles for {tag}/{chrom}: {error}"));

        for (ordinal, window) in tiles.chunks(CHUNK_SIZE).enumerate() {
            if let Some((index, total)) = shard {
                if chunk_id(tag, &chrom, ordinal as u64) % total != index {
                    continue;
                }
            }

            let java = load_java_tsv_chunk(tag, &chrom, &config, window).unwrap_or_else(|error| {
                panic!("Failed to load cached Java TSV chunk for {tag}/{chrom}: {error}")
            });
            let rust = run_rust_chunk(&bam, &ref_path, &sample, &config, window).unwrap_or_else(
                |error| panic!("Failed to run Rust chunk for {tag}/{chrom}: {error}"),
            );

            failures.extend(diff_chunk(&java, &rust, &config));
            if failures.len() >= MAX_FAILURES {
                panic!("{}", format_report(&failures));
            }
        }
    }

    if !failures.is_empty() {
        panic!("{}", format_report(&failures));
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
    let manifest_path = Path::new("tmp/sweep_fixtures/manifest.json");
    let manifest = fs::read_to_string(manifest_path)
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

fn compare_manifest_field(entry: &Value, field: &str, expected: &Value) -> Result<(), String> {
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

fn parse_shard_env() -> Option<(u64, u64)> {
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
    assert!(total > 0, "VARDICT_E2E_SWEEP_SHARD must have N > 0: {trimmed}");
    assert!(index < total, "VARDICT_E2E_SWEEP_SHARD index must be < N: {trimmed}");
    Some((index, total))
}

fn chunk_id(tag: &str, chrom: &str, ordinal: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    (tag, chrom, ordinal).hash(&mut hasher);
    hasher.finish()
}

fn discover_chroms(tag: &str) -> io::Result<Vec<String>> {
    let root = Path::new("tmp/sweep_beds").join(tag);
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
    let bed_path = Path::new("tmp/sweep_beds").join(tag).join(format!("{chrom}.bed"));
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
            invalid_data(format!("Invalid BED start in {}: {error}", bed_path.display()))
        })?;
        let end = fields[2].parse::<u32>().map_err(|error| {
            invalid_data(format!("Invalid BED end in {}: {error}", bed_path.display()))
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
    Path::new("tmp/sweep_fixtures/output")
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
            if let Some(index) = columns.iter().position(|field| field.eq_ignore_ascii_case("Region")) {
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
    bam_path: &Path,
    ref_path: &Path,
    sample: &str,
    config_name: &str,
    tiles: &[TileKey],
) -> io::Result<BTreeMap<TileKey, Vec<String>>> {
    let bam_path_string = bam_path.to_string_lossy().into_owned();
    let ref_path_string = ref_path.to_string_lossy().into_owned();
    let fai_path = format!("{}.fai", ref_path.display());
    let chr_lengths = super::common::load_chr_lengths(&fai_path);
    let regions = build_regions(tiles)?;
    let reference_resource = ReferenceResource::new(
        ref_path_string.clone(),
        1200,
        0,
        chr_lengths.clone(),
        false,
    );
    let config = sweep_config(config_name, &bam_path_string, &ref_path_string);
    let _guard = init_sweep_scope(config, chr_lengths, sample);
    let captured = Arc::new(Mutex::new(String::new()));
    let simple_mode = SimpleMode::new(vec![regions], reference_resource);
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
                invalid_data(format!("Could not locate Region column in Rust output row: {line}"))
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
        let java_rows = java.get(&key).cloned().unwrap_or_default();
        let rust_rows = rust.get(&key).cloned().unwrap_or_default();
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
            let end = i32::try_from(tile.end).map_err(|_| {
                invalid_data(format!("Tile end does not fit in i32: {}", tile.end))
            })?;
            Ok(Region::new(tile.chrom.clone(), start, end, tile.chrom.clone()))
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
    GlobalReadOnlyScope::clear();
    GlobalReadOnlyScope::init(
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

fn detect_region_column(columns: &[&str]) -> Option<usize> {
    columns
        .iter()
        .position(|field| parse_tile_key(field).is_some())
}

fn parse_region_column_value(value: &str) -> io::Result<TileKey> {
    parse_tile_key(value)
        .ok_or_else(|| invalid_data(format!("Invalid Region column value: {value}")))
}

fn parse_tile_key(value: &str) -> Option<TileKey> {
    let (chrom, range) = value.split_once(':')?;
    let (start, end) = range.split_once('-')?;
    Some(TileKey {
        chrom: chrom.to_string(),
        start: start.parse().ok()?,
        end: end.parse().ok()?,
    })
}

fn live_vardictjava_commit() -> Result<String, String> {
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
    let bed_root = Path::new("tmp/sweep_beds").join(tag);
    let mut bed_files = Vec::new();
    for entry in fs::read_dir(&bed_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("bed") {
            bed_files.push(path);
        }
    }
    bed_files.sort();

    // No hash crate is available in dev-dependencies, so use a stable std hasher
    // as a workspace-local cache fingerprint for this scaffold phase.
    let mut hasher = DefaultHasher::new();
    for path in bed_files {
        let mut file = File::open(&path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        bytes.hash(&mut hasher);
    }

    Ok(Value::String(format!("{:016x}", hasher.finish())))
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

    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(Value::String(format!("{:016x}", hasher.finish())))
}

fn compute_generator_flags_hash(config: &str, tag: &str) -> Value {
    let normalized = format!(
        "--output-only --config {config} --tags {tag} --sweep-bed-root tmp/sweep_beds"
    );
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    Value::String(format!("{:016x}", hasher.finish()))
}

fn chrom_sort_key(chrom: &str) -> (u8, u32, &str) {
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

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}