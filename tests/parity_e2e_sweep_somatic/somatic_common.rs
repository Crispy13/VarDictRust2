//! Somatic full-BAM E2E parity harness against cached Java TSV shards.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;
use vardict_rs::prelude::HashMap;

use serde_json::{Value, json};
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::modes::SomaticMode;
use vardict_rs::patterns::SAMPLE_PATTERN2;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

const SOMATIC_COLUMNS_NO_FISHER: usize = 55;
const SOMATIC_COLUMNS_FISHER: usize = 61;
const SOMATIC_REGION_COLUMN_NO_FISHER: usize = 48;
const SOMATIC_REGION_COLUMN_FISHER: usize = 52;

pub const SOMATIC_BAM_PAIR_MAP: &[(&str, &str, &str, &str)] = &[(
    "wes_il_pair",
    "testdata/WES_IL_T_1.bwa.dedup.bam",
    "testdata/WES_IL_N_1.bwa.dedup.bam",
    "testdata/GRCh38.d1.vd1.fa",
)];

#[derive(Clone, Debug)]
struct TileMismatch {
    config: String,
    key: super::r2_common::TileKey,
    java: Vec<String>,
    rust: Vec<String>,
}

struct SweepScopeGuard;

impl Drop for SweepScopeGuard {
    fn drop(&mut self) {
        GlobalReadOnlyScope::clear();
    }
}

#[derive(Clone, Debug)]
struct BedRootSelection {
    logical_root: String,
    path: PathBuf,
}

pub fn somatic_pair_lookup(tag: &str) -> (&'static str, &'static str, &'static str) {
    SOMATIC_BAM_PAIR_MAP
        .iter()
        .find_map(|(candidate, tumor, normal, reference)| {
            (*candidate == tag).then_some((*tumor, *normal, *reference))
        })
        .unwrap_or_else(|| panic!("Unknown somatic pair tag: {tag}"))
}

pub fn sample_name_for_pair(tumor: &str, normal: &str) -> String {
    let raw_bam = format!("{tumor}|{normal}");
    if let Some(captures) = SAMPLE_PATTERN2.captures(&raw_bam) {
        if let Some(sample) = captures.get(1) {
            return sample.as_str().to_string();
        }
    }

    Path::new(tumor)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| panic!("Tumor BAM path has no file stem: {tumor}"))
}

pub fn run_pair(tag: &str) {
    let (tumor, normal, reference) = somatic_pair_lookup(tag);
    let tumor_path = PathBuf::from(tumor);
    let normal_path = PathBuf::from(normal);
    let reference_path = PathBuf::from(reference);
    let sample = sample_name_for_pair(tumor, normal);
    let config = active_config();

    let bed_root = match check_somatic_manifest(&config, tag) {
        Ok(bed_root) => bed_root,
        Err(error) => {
            handle_missing_cache(tag, &error);
            return;
        }
    };

    let chroms = discover_chroms(tag, &config)
        .unwrap_or_else(|error| panic!("Failed to discover sweep chromosomes for {tag}: {error}"));

    for chrom in &chroms {
        let java_path = java_tsv_path(tag, chrom, &config);
        if !java_path.is_file() {
            handle_missing_cache(
                tag,
                &format!(
                    "Missing Java TSV cache for {tag}/{chrom} at {}",
                    java_path.display()
                ),
            );
            return;
        }
    }

    let shard = super::r2_common::parse_shard_env();
    let mut failures = Vec::new();

    for chrom in chroms {
        let tiles = load_tiles_for_chrom(&bed_root.path, tag, &chrom).unwrap_or_else(|error| {
            panic!("Failed to load sweep BED tiles for {tag}/{chrom}: {error}")
        });

        for (ordinal, window) in tiles.chunks(super::r2_common::CHUNK_SIZE).enumerate() {
            if let Some((index, total)) = shard {
                if super::r2_common::chunk_id(tag, &chrom, ordinal as u64) % total != index {
                    continue;
                }
            }

            let java =
                load_java_tsv_chunk_somatic(tag, &chrom, &config, window).unwrap_or_else(|error| {
                    panic!("Failed to load cached Java TSV chunk for {tag}/{chrom}: {error}")
                });
            let rust = run_rust_chunk_somatic(
                &tumor_path,
                &normal_path,
                &reference_path,
                &sample,
                &config,
                window,
            )
            .unwrap_or_else(|error| panic!("Failed to run Rust chunk for {tag}/{chrom}: {error}"));

            let java_presorted =
                has_presorted_java_fixture(tag, &chrom, &config).unwrap_or_else(|error| {
                    panic!(
                        "Failed to inspect cached Java TSV provenance for {tag}/{chrom}: {error}"
                    )
                });
            failures.extend(diff_chunk_somatic(&java, &rust, &config, java_presorted));
            if failures.len() >= super::r2_common::MAX_FAILURES {
                panic!("{}", format_report(&failures));
            }
        }
    }

    if !failures.is_empty() {
        panic!("{}", format_report(&failures));
    }
}

fn check_somatic_manifest(config: &str, tag: &str) -> Result<BedRootSelection, String> {
    let manifest_path = super::r2_common::sweep_fixture_root().join("manifest.json");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("Failed to read {}: {error}", manifest_path.display()))?;
    let manifest_json: Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("Failed to parse {}: {error}", manifest_path.display()))?;

    let live_commit = super::r2_common::live_vardictjava_commit()?;
    let manifest_commit = manifest_json
        .get("vardictjava_commit")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing vardictjava_commit in {}", manifest_path.display()))?;
    if manifest_commit != live_commit {
        return Err(format!(
            "Sweep cache commit mismatch: manifest={manifest_commit}, live={live_commit}"
        ));
    }

    let cache_key = format!("{config}:somatic:{tag}");
    let cache_entry = manifest_json
        .get("cache_entries")
        .and_then(Value::as_object)
        .and_then(|entries| entries.get(&cache_key))
        .ok_or_else(|| {
            format!(
                "Missing cache_entries[{cache_key}] in {}; regenerate the somatic E2E sweep cache",
                manifest_path.display()
            )
        })?;
    let expected_bed_sha = cache_entry
        .get("bed_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("Missing bed_sha256 in somatic cache entry"))?;
    let bed_root = resolve_bed_root(tag, expected_bed_sha).map_err(|error| error.to_string())?;

    super::r2_common::compare_manifest_field(
        cache_entry,
        "bed_sha256",
        &compute_bed_sha256(&bed_root.path, tag)
            .map(Value::String)
            .map_err(|error| error.to_string())?,
    )?;
    super::r2_common::compare_manifest_field(
        cache_entry,
        "bam_stat",
        &compute_bam_stat(tag).map_err(|error| error.to_string())?,
    )?;
    super::r2_common::compare_manifest_field(
        cache_entry,
        "reference_sha256",
        &Value::String(compute_reference_sha256(tag).map_err(|error| error.to_string())?),
    )?;
    super::r2_common::compare_manifest_field(
        cache_entry,
        "generator_flags_hash",
        &Value::String(
            compute_generator_flags_hash(config, tag, &bed_root.logical_root)
                .map_err(|error| error.to_string())?,
        ),
    )?;
    super::r2_common::compare_manifest_field(
        cache_entry,
        "vardictjava_commit",
        &Value::String(live_commit),
    )?;

    Ok(bed_root)
}

fn active_config() -> String {
    std::env::var("VARDICT_E2E_SWEEP_SOMATIC_CONFIG")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

fn config_path_segment(config: &str) -> PathBuf {
    if config == "default" {
        PathBuf::new()
    } else {
        PathBuf::from(config)
    }
}

fn discover_chroms(tag: &str, config: &str) -> io::Result<Vec<String>> {
    let root = super::r2_common::sweep_fixture_root()
        .join("output")
        .join(config_path_segment(config));
    let mut chroms = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let chrom = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| {
                super::r2_common::invalid_data(format!(
                    "Invalid chrom dir name: {}",
                    path.display()
                ))
            })?;
        let tsv_path = path.join(format!("{tag}_{chrom}.tsv.zst"));
        if tsv_path.is_file() {
            chroms.push(chrom);
        }
    }

    chroms.sort_by(|left, right| {
        super::r2_common::chrom_sort_key(left).cmp(&super::r2_common::chrom_sort_key(right))
    });
    Ok(chroms)
}

fn load_tiles_for_chrom(
    bed_root: &Path,
    tag: &str,
    chrom: &str,
) -> io::Result<Vec<super::r2_common::TileKey>> {
    let bed_path = bed_root.join(tag).join(format!("{chrom}.bed"));
    let reader = BufReader::new(File::open(&bed_path)?);
    let mut tiles = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 3 {
            return Err(super::r2_common::invalid_data(format!(
                "Expected at least 3 BED columns in {}: {line}",
                bed_path.display()
            )));
        }
        let start = fields[1].parse::<u32>().map_err(|error| {
            super::r2_common::invalid_data(format!(
                "Invalid BED start in {}: {error}",
                bed_path.display()
            ))
        })?;
        let end = fields[2].parse::<u32>().map_err(|error| {
            super::r2_common::invalid_data(format!(
                "Invalid BED end in {}: {error}",
                bed_path.display()
            ))
        })?;

        tiles.push(super::r2_common::TileKey {
            chrom: fields[0].to_string(),
            start,
            end,
        });
    }

    Ok(tiles)
}

fn java_tsv_path(tag: &str, chrom: &str, config: &str) -> PathBuf {
    super::r2_common::sweep_fixture_root()
        .join("output")
        .join(config_path_segment(config))
        .join(chrom)
        .join(format!("{tag}_{chrom}.tsv.zst"))
}

fn java_chunks_path(tag: &str, chrom: &str, config: &str) -> PathBuf {
    super::r2_common::sweep_fixture_root()
        .join("output")
        .join(config_path_segment(config))
        .join(chrom)
        .join(format!("{tag}_{chrom}.chunks.json"))
}

fn has_presorted_java_fixture(tag: &str, chrom: &str, config: &str) -> io::Result<bool> {
    let path = java_chunks_path(tag, chrom, config);
    let payload: Value = serde_json::from_reader(File::open(&path)?).map_err(|error| {
        super::r2_common::invalid_data(format!(
            "Failed to parse fixture provenance {}: {error}",
            path.display()
        ))
    })?;
    let Some(output_order) = payload.get("output_order") else {
        return Ok(false);
    };
    let mode = output_order.get("mode").and_then(Value::as_str);
    let key = output_order.get("key").and_then(Value::as_str);
    let lc_all = output_order.get("lc_all").and_then(Value::as_str);
    if mode == Some("sorted") && key == Some("Region<TAB>row") && lc_all == Some("C") {
        Ok(true)
    } else {
        Err(super::r2_common::invalid_data(format!(
            "Fixture {} has incompatible output_order provenance; expected mode=sorted key=Region<TAB>row lc_all=C",
            path.display()
        )))
    }
}

fn load_java_tsv_chunk_somatic(
    tag: &str,
    chrom: &str,
    config: &str,
    chunk_tiles: &[super::r2_common::TileKey],
) -> io::Result<BTreeMap<super::r2_common::TileKey, Vec<String>>> {
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
            if let Some(index) = detect_region_column_somatic(&columns) {
                region_index = Some(index);
                if columns[index].eq_ignore_ascii_case("Region")
                    || columns[index].eq_ignore_ascii_case("Seg")
                {
                    continue;
                }
            } else {
                return Err(super::r2_common::invalid_data(format!(
                    "Could not locate Region column in {} from row: {line}",
                    path.display()
                )));
            }
        }

        let tile = super::r2_common::parse_region_column_value(columns[region_index.unwrap()])?;
        if let Some(rows) = rows_by_tile.get_mut(&tile) {
            rows.push(line);
        }
    }

    Ok(rows_by_tile)
}

fn run_rust_chunk_somatic(
    tumor_path: &Path,
    normal_path: &Path,
    reference_path: &Path,
    sample: &str,
    config_name: &str,
    tiles: &[super::r2_common::TileKey],
) -> io::Result<BTreeMap<super::r2_common::TileKey, Vec<String>>> {
    let tumor_path_string = tumor_path.to_string_lossy().into_owned();
    let normal_path_string = normal_path.to_string_lossy().into_owned();
    let reference_path_string = reference_path.to_string_lossy().into_owned();
    let fai_path = format!("{}.fai", reference_path.display());
    let chr_lengths = super::common::load_chr_lengths(&fai_path);
    let config = sweep_config(
        config_name,
        &tumor_path_string,
        &normal_path_string,
        &reference_path_string,
        sample,
    );
    let regions = build_regions(tiles, config.number_nucleotide_to_extend)?;
    let reference_resource = ReferenceResource::new(
        reference_path_string.clone(),
        config.reference_extension,
        config.number_nucleotide_to_extend,
        chr_lengths.clone(),
        false,
    );
    let _guard = init_sweep_scope(config, chr_lengths, sample);
    let captured = Arc::new(Mutex::new(String::new()));
    let somatic_mode = SomaticMode::new(vec![regions], reference_resource);
    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Buffer(captured.clone()));
    somatic_mode.not_parallel();

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
            region_index = Some(detect_region_column_somatic(&columns).ok_or_else(|| {
                super::r2_common::invalid_data(format!(
                    "Could not locate Region column in Rust output row: {line}"
                ))
            })?);
            if columns[region_index.unwrap()].eq_ignore_ascii_case("Region")
                || columns[region_index.unwrap()].eq_ignore_ascii_case("Seg")
            {
                continue;
            }
        }

        let tile = super::r2_common::parse_region_column_value(columns[region_index.unwrap()])?;
        if let Some(rows) = rows_by_tile.get_mut(&tile) {
            rows.push(line.to_string());
        }
    }

    Ok(rows_by_tile)
}

fn diff_chunk_somatic(
    java: &BTreeMap<super::r2_common::TileKey, Vec<String>>,
    rust: &BTreeMap<super::r2_common::TileKey, Vec<String>>,
    config: &str,
    java_presorted: bool,
) -> Vec<TileMismatch> {
    let mut failures = Vec::new();
    let keys: BTreeSet<_> = java.keys().chain(rust.keys()).cloned().collect();

    for key in keys {
        let mut java_rows = java.get(&key).cloned().unwrap_or_default();
        let mut rust_rows = rust.get(&key).cloned().unwrap_or_default();
        if !java_presorted {
            java_rows.sort();
        }
        rust_rows.sort();

        if java_rows != rust_rows {
            failures.push(TileMismatch {
                config: config.to_string(),
                key,
                java: java_rows,
                rust: rust_rows,
            });
        }
        if failures.len() >= super::r2_common::MAX_FAILURES {
            break;
        }
    }

    failures
}

fn build_regions(
    tiles: &[super::r2_common::TileKey],
    extend: i32,
) -> io::Result<Vec<Region>> {
    tiles
        .iter()
        .map(|tile| {
            let start = i32::try_from(tile.start).map_err(|_| {
                super::r2_common::invalid_data(format!(
                    "Tile start does not fit in i32: {}",
                    tile.start
                ))
            })?;
            let end = i32::try_from(tile.end).map_err(|_| {
                super::r2_common::invalid_data(format!(
                    "Tile end does not fit in i32: {}",
                    tile.end
                ))
            })?;
            Ok(Region::new(
                tile.chrom.clone(),
                start - extend,
                end + extend,
                tile.chrom.clone(),
            ))
        })
        .collect()
}

fn sweep_config(
    config_name: &str,
    tumor_path: &str,
    normal_path: &str,
    reference_path: &str,
    sample: &str,
) -> Configuration {
    let mut config = if config_name == "default" {
        Configuration::default()
    } else {
        super::common::config_preset(config_name)
    };
    config.bam = Some(BamNames::new(format!("{tumor_path}|{normal_path}")));
    config.fasta = reference_path.to_string();
    config.sample_name = Some(sample.to_string());
    config
}

fn init_sweep_scope(
    config: Configuration,
    chr_lengths: HashMap<String, i32>,
    sample: &str,
) -> SweepScopeGuard {
    let (adaptor_forward, adaptor_reverse) = super::r2_common::build_adaptor_maps(&config.adaptor);
    GlobalReadOnlyScope::clear();
    GlobalReadOnlyScope::init(
        config,
        chr_lengths,
        sample,
        None,
        None,
        adaptor_forward,
        adaptor_reverse,
    );
    SweepScopeGuard
}

fn empty_tile_map(
    chunk_tiles: &[super::r2_common::TileKey],
) -> BTreeMap<super::r2_common::TileKey, Vec<String>> {
    chunk_tiles
        .iter()
        .cloned()
        .map(|tile| (tile, Vec::new()))
        .collect()
}

fn detect_region_column_somatic(columns: &[&str]) -> Option<usize> {
    if let Some(index) = columns
        .iter()
        .position(|field| field.eq_ignore_ascii_case("Region") || field.eq_ignore_ascii_case("Seg"))
    {
        return Some(index);
    }

    let preferred = match columns.len() {
        SOMATIC_COLUMNS_NO_FISHER => Some(SOMATIC_REGION_COLUMN_NO_FISHER),
        SOMATIC_COLUMNS_FISHER => Some(SOMATIC_REGION_COLUMN_FISHER),
        _ => None,
    };

    if let Some(index) = preferred {
        if columns
            .get(index)
            .and_then(|value| super::r2_common::parse_tile_key(value))
            .is_some()
        {
            return Some(index);
        }
    }

    super::r2_common::detect_region_column(columns)
}

fn compute_bed_sha256(bed_root: &Path, tag: &str) -> io::Result<String> {
    let bed_root = bed_root.join(tag);
    let mut bed_files = Vec::new();

    for entry in fs::read_dir(&bed_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("bed") {
            bed_files.push(path);
        }
    }

    bed_files.sort();
    sha256_concat_files(&bed_files)
}

fn compute_bam_stat(tag: &str) -> io::Result<Value> {
    let (tumor, normal, _) = somatic_pair_lookup(tag);
    Ok(json!([
        bam_stat_entry(tumor, "tumor")?,
        bam_stat_entry(normal, "normal")?,
    ]))
}

fn compute_reference_sha256(tag: &str) -> io::Result<String> {
    let (_, _, reference) = somatic_pair_lookup(tag);
    let fai_path = PathBuf::from(format!("{reference}.fai"));
    let target = if fai_path.is_file() {
        fai_path
    } else {
        PathBuf::from(reference)
    };
    sha256_file(&target)
}

fn compute_generator_flags_hash(config: &str, tag: &str, bed_root: &str) -> io::Result<String> {
    let logical_flags = format!(
        "--output-only --config {config} --pair-tags {tag} --tags --sweep-bed-root {bed_root}"
    );
    sha256_bytes(logical_flags.as_bytes())
}

fn resolve_bed_root(tag: &str, expected_bed_sha: &str) -> io::Result<BedRootSelection> {
    if let Ok(root) = std::env::var("VARDICT_E2E_SWEEP_BED_ROOT") {
        let path = PathBuf::from(root.trim());
        let logical_root = path.to_string_lossy().into_owned();
        return Ok(BedRootSelection { logical_root, path });
    }

    let mut candidates = Vec::new();
    for logical_root in candidate_bed_roots()? {
        let path = PathBuf::from(&logical_root);
        let tag_root = path.join(tag);
        if !tag_root.is_dir() {
            continue;
        }
        let digest = compute_bed_sha256(&path, tag)?;
        if digest == expected_bed_sha {
            candidates.push(BedRootSelection { logical_root, path });
        }
    }

    candidates.sort_by(|left, right| left.logical_root.cmp(&right.logical_root));
    candidates.into_iter().next().ok_or_else(|| {
        io::Error::other(format!(
            "No sweep BED root matched manifest hash for {tag}; checked tmp/* candidates"
        ))
    })
}

fn candidate_bed_roots() -> io::Result<Vec<String>> {
    let mut roots = vec![String::from("tmp/sweep_beds")];

    if Path::new("tmp").is_dir() {
        for entry in fs::read_dir("tmp")? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let logical_root = path.to_string_lossy().into_owned();
            if logical_root == "tmp/sweep_beds" {
                continue;
            }
            roots.push(logical_root);
        }
    }

    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn bam_stat_entry(path: &str, role: &str) -> io::Result<Value> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .map_err(io::Error::other)?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    Ok(json!({
        "path": path,
        "size": metadata.len(),
        "mtime_unix": modified,
        "role": role,
    }))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    sha256_concat_files(&[path.to_path_buf()])
}

fn sha256_concat_files(paths: &[PathBuf]) -> io::Result<String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Failed to open sha256sum stdin"))?;
        for path in paths {
            let mut file = File::open(path)?;
            io::copy(&mut file, &mut stdin)?;
        }
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "sha256sum failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    parse_sha256sum_output(&output.stdout)
}

fn sha256_bytes(bytes: &[u8]) -> io::Result<String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Failed to open sha256sum stdin"))?;
        stdin.write_all(bytes)?;
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "sha256sum failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    parse_sha256sum_output(&output.stdout)
}

fn parse_sha256sum_output(stdout: &[u8]) -> io::Result<String> {
    let digest = String::from_utf8(stdout.to_vec())
        .map_err(|error| io::Error::other(format!("sha256sum output was not UTF-8: {error}")))?;
    digest
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| io::Error::other("sha256sum output was empty"))
}

fn handle_missing_cache(tag: &str, reason: &str) {
    if matches!(std::env::var("CI"), Ok(value) if value == "true") {
        panic!("E2E sweep cache validation failed for {tag}: {reason}");
    }

    eprintln!("Skipping parity_e2e_sweep_somatic for {tag}: {reason}");
}

fn format_report(failures: &[TileMismatch]) -> String {
    let mut report = format!(
        "Somatic E2E sweep mismatches: {} tile(s) differ; showing up to {}.",
        failures.len(),
        super::r2_common::MAX_FAILURES,
    );

    for (index, failure) in failures
        .iter()
        .take(super::r2_common::MAX_FAILURES)
        .enumerate()
    {
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

fn format_rows(rows: &[String]) -> String {
    if rows.is_empty() {
        "<empty>".to_string()
    } else {
        rows.join("\n")
    }
}
