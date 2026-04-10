use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};
use std::{fs::File, io::Read, path::PathBuf};

use std::sync::Arc;
use vardict_rs::config::Configuration;
use vardict_rs::data::Region;
use vardict_rs::reference::{Reference, ReferenceResource};
use vardict_rs::scope::{GlobalReadOnlyScope, Scope, VariantPrinter};

#[allow(dead_code)]
pub fn load_region_config() -> Vec<(String, PathBuf, PathBuf)> {
    let tsv = std::fs::read_to_string("testdata/parity_regions.tsv")
        .expect("testdata/parity_regions.tsv not found");

    tsv.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(
                fields.len(),
                3,
                "expected 3 fields in parity_regions.tsv: {line}"
            );
            (
                fields[0].to_string(),
                PathBuf::from(fields[1]),
                PathBuf::from(fields[2]),
            )
        })
        .collect()
}

#[allow(dead_code)]
pub fn golden_fixture_path(module: &str, region: &str) -> PathBuf {
    let safe_region = region.replace(':', "_").replace('-', "_");
    let filename = format!("{module}_{safe_region}.jsonl.zst");
    PathBuf::from("testdata/fixtures")
        .join(module)
        .join(filename)
}

#[allow(dead_code)]
pub fn load_golden_data(module: &str, region: &str) -> String {
    let path = golden_fixture_path(module, region);
    let file = File::open(&path)
        .unwrap_or_else(|error| panic!("Failed to open {}: {error}", path.display()));
    let mut decoder = zstd::stream::read::Decoder::new(file)
        .unwrap_or_else(|error| panic!("Failed to decode {}: {error}", path.display()));
    let mut content = String::new();
    decoder
        .read_to_string(&mut content)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
    let lines: Vec<&str> = content.lines().collect();

    assert!(
        lines.len() >= 2,
        "Fixture {} should have at least 2 lines",
        path.display()
    );

    lines[1].to_string()
}

#[allow(dead_code)]
pub fn sweep_fixture_base() -> PathBuf {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tmp/sweep_fixtures"));

    assert!(
        base.is_dir(),
        "Sweep fixture directory not found at {}. Generate with: scripts/sweep_fixtures.sh",
        base.display()
    );

    base
}

#[allow(dead_code)]
pub fn check_sweep_manifest() {
    let base = sweep_fixture_base();
    let manifest_path = base.join("manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "Failed to read {}: {error}. Regenerate with: scripts/sweep_fixtures.sh",
            manifest_path.display()
        )
    });
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest)
        .unwrap_or_else(|error| panic!("Failed to parse {}: {error}", manifest_path.display()));
    let manifest_commit = manifest_json
        .get("vardictjava_commit")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("Missing vardictjava_commit in {}", manifest_path.display()));
    let output = std::process::Command::new("git")
        .args(["-C", "VarDictJava", "rev-parse", "HEAD"])
        .output()
        .unwrap_or_else(|error| panic!("Failed to run git for VarDictJava commit: {error}"));

    assert!(
        output.status.success(),
        "Failed to resolve VarDictJava commit: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let live_commit = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("VarDictJava commit output was not valid UTF-8: {error}"));

    assert_eq!(
        manifest_commit,
        live_commit.trim(),
        "Sweep fixtures are stale. Regenerate with: scripts/sweep_fixtures.sh"
    );
}

#[allow(dead_code)]
pub fn load_sweep_region_config() -> Vec<(String, PathBuf, PathBuf)> {
    let base = sweep_fixture_base();
    let tsv_path = base.join("regions.tsv");
    let tsv = std::fs::read_to_string(&tsv_path).unwrap_or_else(|error| {
        panic!(
            "Failed to read {}: {error}. Generate with: scripts/sweep_fixtures.sh",
            tsv_path.display()
        )
    });

    tsv.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(
                fields.len(),
                3,
                "expected 3 fields in {}: {line}",
                tsv_path.display()
            );
            (
                fields[0].to_string(),
                PathBuf::from(fields[1]),
                PathBuf::from(fields[2]),
            )
        })
        .collect()
}

#[allow(dead_code)]
pub fn sweep_fixture_path(module: &str, region: &str) -> PathBuf {
    let (chrom, range) = region
        .split_once(':')
        .unwrap_or_else(|| panic!("Invalid sweep region format: {region}"));
    let (start, end) = range
        .split_once('-')
        .unwrap_or_else(|| panic!("Invalid sweep region range: {region}"));
    let filename = format!("{module}_{chrom}_{start}_{end}.jsonl.zst");

    sweep_fixture_base().join(module).join(chrom).join(filename)
}

#[allow(dead_code)]
pub fn load_sweep_golden_data(module: &str, region: &str) -> String {
    let path = sweep_fixture_path(module, region);
    let file = File::open(&path)
        .unwrap_or_else(|error| panic!("Failed to open {}: {error}", path.display()));
    let mut decoder = zstd::stream::read::Decoder::new(file)
        .unwrap_or_else(|error| panic!("Failed to decode {}: {error}", path.display()));
    let mut content = String::new();
    decoder
        .read_to_string(&mut content)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
    let lines: Vec<&str> = content.lines().collect();

    assert!(
        lines.len() >= 2,
        "Fixture {} should have at least 2 lines",
        path.display()
    );

    lines[1].to_string()
}

// ── Step 2.1: assert_module_parity ──────────────────────────────────────────

#[allow(dead_code)]
pub fn assert_module_parity(module: &str, region: &str, actual_json: &str) {
    let golden = load_golden_data(module, region);
    if actual_json == golden {
        return;
    }
    // Find first divergent byte offset
    let offset = actual_json
        .bytes()
        .zip(golden.bytes())
        .position(|(a, g)| a != g)
        .unwrap_or(actual_json.len().min(golden.len()));

    let window = 80usize;
    let half = window / 2;
    let golden_start = offset.saturating_sub(half);
    let golden_end = (offset + half).min(golden.len());
    let actual_start = offset.saturating_sub(half);
    let actual_end = (offset + half).min(actual_json.len());

    panic!(
        "Parity mismatch for module={module}, region={region}\n\
         First divergence at byte offset {offset}\n\
         Golden[{golden_start}..{golden_end}]: {:?}\n\
         Actual[{actual_start}..{actual_end}]: {:?}",
        &golden[golden_start..golden_end],
        &actual_json[actual_start..actual_end],
    );
}

#[allow(dead_code)]
pub fn assert_sweep_module_parity(module: &str, region: &str, actual_json: &str) -> Option<String> {
    let golden = load_sweep_golden_data(module, region);
    if actual_json == golden {
        return None;
    }

    let offset = actual_json
        .bytes()
        .zip(golden.bytes())
        .position(|(a, g)| a != g)
        .unwrap_or(actual_json.len().min(golden.len()));

    let window = 80usize;
    let half = window / 2;
    let golden_start = offset.saturating_sub(half);
    let golden_end = (offset + half).min(golden.len());
    let actual_start = offset.saturating_sub(half);
    let actual_end = (offset + half).min(actual_json.len());

    Some(format!(
        "module={module}, region={region}\n\
         First divergence at byte offset {offset}\n\
         Golden[{golden_start}..{golden_end}]: {:?}\n\
         Actual[{actual_start}..{actual_end}]: {:?}",
        &golden[golden_start..golden_end],
        &actual_json[actual_start..actual_end],
    ))
}

#[allow(dead_code)]
pub fn load_chr_lengths(fai_path: &str) -> HashMap<String, i32> {
    let content = std::fs::read_to_string(fai_path)
        .unwrap_or_else(|error| panic!("Failed to read FAI file {fai_path}: {error}"));

    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let chr = fields[0].to_string();
            let len: i32 = fields[1].parse().unwrap_or(0);
            (chr, len)
        })
        .collect()
}

// ── Step 2.2: init_test_scope ───────────────────────────────────────────────

static SCOPE_MUTEX: Mutex<()> = Mutex::new(());

#[allow(dead_code)]
pub fn init_test_scope() -> MutexGuard<'static, ()> {
    let guard = SCOPE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    GlobalReadOnlyScope::clear();
    GlobalReadOnlyScope::init(
        Configuration::default(),
        HashMap::new(),
        "test_sample",
        None,
        None,
        HashMap::new(),
        HashMap::new(),
    );
    guard
}

// ── Step 2.3: build_scope_for_test ──────────────────────────────────────────

#[allow(dead_code)]
pub fn build_scope_for_test<D>(
    region: &Region,
    bam_path: &str,
    ref_path: &str,
    data: D,
) -> Scope<D> {
    let reference = Arc::new(Reference::new());
    let reference_resource = Arc::new(ReferenceResource::new(
        ref_path,
        0,
        0,
        HashMap::new(),
        false,
    ));
    Scope::new(
        bam_path,
        region.clone(),
        reference,
        reference_resource,
        0,
        HashSet::new(),
        VariantPrinter::Out,
        data,
    )
}

// ── Step 2.4: parse_region ──────────────────────────────────────────────────

#[allow(dead_code)]
pub fn parse_region(region_str: &str) -> Region {
    let (chr, range) = region_str
        .split_once(':')
        .unwrap_or_else(|| panic!("Invalid region format (expected chr:start-end): {region_str}"));
    let (start_str, end_str) = range
        .split_once('-')
        .unwrap_or_else(|| panic!("Invalid region range (expected start-end): {region_str}"));
    let start: i32 = start_str
        .parse()
        .unwrap_or_else(|e| panic!("Invalid start in region {region_str}: {e}"));
    let end: i32 = end_str
        .parse()
        .unwrap_or_else(|e| panic!("Invalid end in region {region_str}: {e}"));
    Region::new(chr, start, end, "")
}
