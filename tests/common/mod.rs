use std::collections::{HashMap, HashSet};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use std::sync::Arc;
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::reference::{Reference, ReferenceResource};
use vardict_rs::scope::{GlobalReadOnlyScope, Scope, VariantPrinter};

#[allow(dead_code)]
pub fn load_region_config() -> Vec<(String, PathBuf, PathBuf)> {
    let tsv = std::fs::read_to_string("testdata/parity_regions.tsv")
        .expect("testdata/parity_regions.tsv not found");

    let regions: Vec<_> = tsv.lines()
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
        .collect();

    match std::env::var("PARITY_REGION_INDEX") {
        Err(std::env::VarError::NotPresent) => regions,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("PARITY_REGION_INDEX must be valid UTF-8")
        }
        Ok(value) => {
            let index = value.parse::<usize>().unwrap_or_else(|_| {
                panic!("PARITY_REGION_INDEX must be a non-negative integer, got: {value}")
            });
            assert!(
                index < regions.len(),
                "PARITY_REGION_INDEX={index} out of range (0..{})",
                regions.len()
            );
            vec![regions[index].clone()]
        }
    }
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

/// Load a single region's golden data from a v2 archive.
/// The archive format is: {chrom}\t{start}\t{end}\t{data}\n per line, zstd-compressed.
/// Scans line-by-line until it finds the matching region.
#[allow(dead_code)]
pub fn load_v2_archive_region(archive_path: &std::path::Path, target_region: &str) -> String {
    let (chrom, range) = target_region
        .split_once(':')
        .unwrap_or_else(|| panic!("Invalid region format: {target_region}"));
    let (start, end) = range
        .split_once('-')
        .unwrap_or_else(|| panic!("Invalid region range: {target_region}"));
    let target_key = format!("{chrom}\t{start}\t{end}\t");

    let file = File::open(archive_path)
        .unwrap_or_else(|error| panic!("Failed to open {}: {error}", archive_path.display()));
    let decoder = zstd::stream::read::Decoder::new(file)
        .unwrap_or_else(|error| panic!("Failed to decode {}: {error}", archive_path.display()));
    let reader = std::io::BufReader::new(decoder);

    use std::io::BufRead;

    for line in reader.lines() {
        let line = line
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", archive_path.display()));
        if line.starts_with(&target_key) {
            return line[target_key.len()..].to_string();
        }
    }

    panic!(
        "Region {target_region} not found in archive {}",
        archive_path.display()
    );
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
pub fn e2e_fixture_base() -> PathBuf {
    std::env::var_os("VARDICT_E2E_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tmp/e2e_fixtures"))
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VardictImpl {
    Rust,
    Java,
    Both,
}

#[allow(dead_code)]
pub fn resolve_impl() -> VardictImpl {
    match std::env::var("VARDICT_IMPL") {
        Err(std::env::VarError::NotPresent) => VardictImpl::Rust,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("Unknown VARDICT_IMPL: value was not valid UTF-8")
        }
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "" | "rust" => VardictImpl::Rust,
            "java" => VardictImpl::Java,
            "both" => VardictImpl::Both,
            _ => panic!(
                "Unknown VARDICT_IMPL '{value}'. Expected one of: rust, java, both"
            ),
        },
    }
}

#[allow(dead_code)]
pub fn java_binary_path() -> PathBuf {
    const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

    let vardict_dir = project_root().join("VarDictJava");
    let binary = vardict_dir.join("build/install/VarDict/bin/VarDict");
    if is_executable_file(&binary) {
        return binary;
    }

    let mut build = Command::new("./gradlew");
    build
        .current_dir(&vardict_dir)
        .args(["installDist", "-q"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_command_with_timeout(
        build,
        BUILD_TIMEOUT,
        "building VarDictJava with ./gradlew installDist -q",
    );

    assert!(
        output.status.success(),
        "Failed to build VarDictJava with ./gradlew installDist -q\nSTDOUT:\n{}\nSTDERR:\n{}",
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim(),
    );
    assert!(
        is_executable_file(&binary),
        "VarDictJava launcher still missing after build at {}. Build manually with: (cd {} && ./gradlew installDist -q)",
        binary.display(),
        vardict_dir.display(),
    );

    binary
}

#[allow(dead_code)]
pub fn run_java_region(region_str: &str, bam: &str, ref_path: &str, extra_flags: &[String]) -> String {
    const RUN_TIMEOUT: Duration = Duration::from_secs(120);

    let java_bin = java_binary_path();
    let mut command = Command::new(&java_bin);
    command
        .arg("-G")
        .arg(ref_path)
        .arg("-b")
        .arg(bam)
        .arg("-N")
        .arg("test_sample")
        .arg("-th")
        .arg("1")
        .args(extra_flags)
        .arg("-R")
        .arg(region_str)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = run_command_with_timeout(
        command,
        RUN_TIMEOUT,
        &format!("running VarDictJava for region {region_str}"),
    );
    assert!(
        output.status.success(),
        "VarDictJava failed for region {region_str} with exit status {}\nSTDERR:\n{}\nSTDOUT:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim(),
        String::from_utf8_lossy(&output.stdout).trim(),
    );

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(dead_code)]
pub fn config_preset_java_flags(config_name: &str) -> Vec<String> {
    if config_name == "default" {
        return Vec::new();
    }

    let available = load_config_presets_raw_tsv();
    available
        .into_iter()
        .find_map(|(name, flags)| {
            (name == config_name).then(|| {
                flags
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_else(|| {
            panic!(
                "Unknown config preset: {config_name}. Available presets: {}",
                CONFIG_PRESETS.join(", ")
            )
        })
}

#[allow(dead_code)]
pub fn safe_region_name(region: &str) -> String {
    region.replace(':', "_").replace('-', "_")
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        return path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn run_command_with_timeout(mut command: Command, timeout: Duration, description: &str) -> Output {
    let mut child = command.spawn().unwrap_or_else(|error| {
        panic!("Failed to start {description}: {error}")
    });

    let stdout_handle = child.stdout.take().map(spawn_output_reader);
    let stderr_handle = child.stderr.take().map(spawn_output_reader);
    let start = Instant::now();

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_output_reader(stdout_handle);
                let stderr = join_output_reader(stderr_handle);
                panic!(
                    "Timed out after {}s while {description}\nSTDOUT:\n{}\nSTDERR:\n{}",
                    timeout.as_secs(),
                    String::from_utf8_lossy(&stdout).trim(),
                    String::from_utf8_lossy(&stderr).trim(),
                );
            }
            Err(error) => panic!("Failed while waiting for {description}: {error}"),
        }
    };

    Output {
        status,
        stdout: join_output_reader(stdout_handle),
        stderr: join_output_reader(stderr_handle),
    }
}

fn spawn_output_reader<R>(mut reader: R) -> thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .unwrap_or_else(|error| panic!("Failed reading child process output: {error}"));
        bytes
    })
}

fn join_output_reader(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .map(|reader| {
            reader
                .join()
                .unwrap_or_else(|_| panic!("Failed joining child process output reader"))
        })
        .unwrap_or_default()
}

#[allow(dead_code)]
pub fn load_golden_tsv(
    fixture_base: &Path,
    region_str: &str,
    config_subdir: Option<&str>,
    regen_cmd: &str,
) -> String {
    let safe_region = safe_region_name(region_str);
    let path = match config_subdir {
        Some(name) => fixture_base.join(name).join(format!("{safe_region}.tsv")),
        None => fixture_base.join(format!("{safe_region}.tsv")),
    };

    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "Missing E2E golden for region {region_str} at {}: {error}. Regenerate with: {regen_cmd}",
            path.display()
        )
    })
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
#[allow(deprecated)]
#[deprecated(note = "Use v2 archive functions instead")]
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
#[allow(deprecated)]
#[deprecated(note = "Use v2 archive functions instead")]
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
#[allow(deprecated)]
#[deprecated(note = "Use v2 archive functions instead")]
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
pub fn assert_tsv_parity(rust_output: &str, golden: &str, region: &str) {
    let mut expected_lines: Vec<&str> = golden.lines().collect();
    let mut actual_lines: Vec<&str> = rust_output.lines().collect();
    expected_lines.sort_unstable();
    actual_lines.sort_unstable();

    if actual_lines == expected_lines {
        return;
    }

    let first_diff = expected_lines
        .iter()
        .zip(actual_lines.iter())
        .position(|(expected_line, actual_line)| expected_line != actual_line)
        .unwrap_or_else(|| expected_lines.len().min(actual_lines.len()));

    let expected_line = expected_lines.get(first_diff).copied().unwrap_or("");
    let actual_line = actual_lines.get(first_diff).copied().unwrap_or("");
    let mut message = format!(
        "E2E TSV mismatch for region {region}\nFirst divergent sorted line index: {first_diff}\nGolden: {}\nActual: {}",
        escape_snippet(expected_line),
        escape_snippet(actual_line),
    );

    if whitespace_only_difference(expected_line, actual_line) {
        message.push_str(&format!(
            "\nGolden bytes: {}\nActual bytes: {}",
            hex_dump(expected_line.as_bytes()),
            hex_dump(actual_line.as_bytes()),
        ));
    }

    panic!("{message}");
}

fn escape_snippet(line: &str) -> String {
    format!("{:?}", line)
}

fn whitespace_only_difference(left: &str, right: &str) -> bool {
    left != right
        && left
            .chars()
            .filter(|ch| !ch.is_ascii_whitespace())
            .collect::<String>()
            == right
                .chars()
                .filter(|ch| !ch.is_ascii_whitespace())
                .collect::<String>()
}

fn hex_dump(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[allow(dead_code)]
#[allow(deprecated)]
#[deprecated(note = "Use v2 archive functions instead")]
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

#[allow(dead_code)]
pub const CONFIG_PRESETS: &[&str] = &[
    "default",
    "sensitive",
    "strict",
    "mismatch_tolerant",
    "low_bias",
    "clinical_wgs",
];

fn load_config_presets_raw_tsv() -> Vec<(String, String)> {
    let preset_path = project_root().join("scripts/config_presets.tsv");
    let tsv = std::fs::read_to_string(&preset_path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", preset_path.display()));

    tsv.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(
                fields.len(),
                3,
                "expected 3 tab-separated fields in {}: {line}",
                preset_path.display()
            );
            (fields[0].to_string(), fields[1].to_string())
        })
        .collect()
}

#[allow(dead_code)]
pub fn load_config_presets_tsv() -> Vec<(String, HashMap<String, String>)> {
    load_config_presets_raw_tsv()
        .into_iter()
        .map(|(name, flags)| (name, parse_java_flags(&flags)))
        .collect()
}

fn parse_java_flags(flags: &str) -> HashMap<String, String> {
    let tokens: Vec<&str> = flags.split_whitespace().collect();
    assert_eq!(
        tokens.len() % 2,
        0,
        "Expected flag/value pairs in config preset flags: {flags}"
    );

    let mut parsed = HashMap::new();
    for pair in tokens.chunks(2) {
        let flag = pair[0];
        let value = pair[1];
        assert!(flag.starts_with('-'), "Invalid flag token in preset flags: {flag}");
        let previous = parsed.insert(flag.to_string(), value.to_string());
        assert!(
            previous.is_none(),
            "Duplicate flag {flag} in config preset flags: {flags}"
        );
    }

    parsed
}

#[allow(dead_code)]
pub fn config_preset(name: &str) -> Configuration {
    let mut config = Configuration::default();

    match name {
        "default" => {}
        "sensitive" => {
            config.freq = 0.005;
            config.minr = 1;
            config.goodq = 15.0;
        }
        "strict" => {
            config.freq = 0.05;
            config.minr = 4;
            config.goodq = 30.0;
        }
        "mismatch_tolerant" => {
            config.mismatch = 15;
            config.vext = 5;
        }
        "low_bias" => {
            config.min_bias_reads = 1;
            config.freq = 0.02;
        }
        "clinical_wgs" => {
            config.freq = 0.001;
            config.minr = 1;
            config.goodq = 20.0;
            config.mismatch = 12;
        }
        _ => panic!("Unknown config preset: {name}"),
    }

    config
}

// ── Step 2.2: init_test_scope ───────────────────────────────────────────────

static SCOPE_MUTEX: Mutex<()> = Mutex::new(());

#[allow(dead_code)]
pub fn init_test_scope(chr_lengths: HashMap<String, i32>) -> MutexGuard<'static, ()> {
    let guard = SCOPE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    GlobalReadOnlyScope::clear();
    GlobalReadOnlyScope::init(
        Configuration::default(),
        chr_lengths,
        "test_sample",
        None,
        None,
        HashMap::new(),
        HashMap::new(),
    );
    guard
}

#[allow(dead_code)]
pub fn init_test_scope_with_bam(
    bam_path: &str,
    ref_path: &str,
    chr_lengths: HashMap<String, i32>,
) -> MutexGuard<'static, ()> {
    let guard = SCOPE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    GlobalReadOnlyScope::clear();

    let mut config = Configuration::default();
    config.bam = Some(BamNames::new(bam_path));
    config.fasta = ref_path.to_string();

    GlobalReadOnlyScope::init(
        config,
        chr_lengths,
        "test_sample",
        None,
        None,
        HashMap::new(),
        HashMap::new(),
    );

    guard
}

#[allow(dead_code)]
pub fn init_test_scope_with_config(
    mut config: Configuration,
    bam_path: &str,
    reference: &str,
    chr_lengths: HashMap<String, i32>,
) -> MutexGuard<'static, ()> {
    let guard = SCOPE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    GlobalReadOnlyScope::clear();

    config.bam = Some(BamNames::new(bam_path));
    config.fasta = reference.to_string();

    GlobalReadOnlyScope::init(
        config,
        chr_lengths,
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

#[allow(dead_code)]
pub const BAM_TAG_MAP: &[(&str, &str, &str)] = &[
    (
        "na12878_exome",
        "testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam",
        "testdata/hs37d5.fa",
    ),
    (
        "hg002",
        "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
        "testdata/hs37d5.fa",
    ),
    (
        "na12878_lowcov",
        "testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam",
        "testdata/hs37d5.fa",
    ),
];

#[allow(dead_code)]
pub fn bam_tag_lookup(tag: &str) -> (&'static str, &'static str) {
    BAM_TAG_MAP
        .iter()
        .find_map(|(candidate, bam_path, ref_path)| {
            (*candidate == tag).then_some((*bam_path, *ref_path))
        })
        .unwrap_or_else(|| panic!("Unknown BAM tag: {tag}"))
}

fn v2_chrom_sort_key(chrom: &str) -> (u8, i32, &str) {
    match chrom.parse::<i32>() {
        Ok(number) => (0, number, ""),
        Err(_) => (1, 0, chrom),
    }
}

#[allow(dead_code)]
pub fn discover_v2_archives(
    base: &std::path::Path,
    module: &str,
) -> Vec<(String, String, PathBuf)> {
    let module_root = base.join("v2").join(module);
    assert!(
        module_root.is_dir(),
        "V2 archive directory not found: {}",
        module_root.display()
    );

    let mut archives = Vec::new();
    let entries = std::fs::read_dir(&module_root)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", module_root.display()));

    for entry in entries {
        let entry = entry
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", module_root.display()));
        let tag_path = entry.path();
        if !tag_path.is_dir() {
            continue;
        }

        let bam_tag = entry
            .file_name()
            .to_str()
            .unwrap_or_else(|| panic!("Invalid BAM tag directory: {}", tag_path.display()))
            .to_string();
        let tag_entries = std::fs::read_dir(&tag_path)
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", tag_path.display()));

        for archive_entry in tag_entries {
            let archive_entry = archive_entry
                .unwrap_or_else(|error| panic!("Failed to read {}: {error}", tag_path.display()));
            let archive_path = archive_entry.path();
            if !archive_path.is_file() {
                continue;
            }

            let file_name = archive_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_else(|| panic!("Invalid archive name: {}", archive_path.display()));
            let chrom = match file_name.strip_suffix(".jsonl.zst") {
                Some(chrom) => chrom,
                None => continue,
            };

            archives.push((bam_tag.clone(), chrom.to_string(), archive_path));
        }
    }

    archives.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| v2_chrom_sort_key(&left.1).cmp(&v2_chrom_sort_key(&right.1)))
    });

    // Shard-scope filtering: reduce to a single BAM tag for fast CI sweeps
    if let Ok(scope) = std::env::var("VARDICT_SWEEP_SHARD_SCOPE") {
        if !scope.is_empty() {
            let target_tag = if scope == "1" {
                "na12878_exome".to_string()
            } else {
                scope
            };
            archives.retain(|(bam_tag, _, _)| bam_tag == &target_tag);
            if archives.is_empty() {
                panic!(
                    "VARDICT_SWEEP_SHARD_SCOPE={target_tag} matched zero archives for module '{module}'. \
                     Available tags: check v2/{module}/ directory."
                );
            }
            eprintln!(
                "  [shard-scope] filtered to {target_tag}: {} archive(s) for module '{module}'",
                archives.len()
            );
        }
    }

    archives
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2ArchiveLine {
    pub chrom: String,
    pub start: i32,
    pub end: i32,
    pub data: String,
}

impl V2ArchiveLine {
    #[allow(dead_code)]
    pub fn region_str(&self) -> String {
        format!("{}:{}-{}", self.chrom, self.start, self.end)
    }
}

#[allow(dead_code)]
pub struct V2ArchiveReader {
    archive_path: PathBuf,
    reader: std::io::BufReader<zstd::stream::read::Decoder<'static, std::io::BufReader<File>>>,
    line_buf: String,
    line_number: usize,
}

impl V2ArchiveReader {
    #[allow(dead_code)]
    pub fn new(archive_path: &std::path::Path) -> Self {
        let file = File::open(archive_path)
            .unwrap_or_else(|error| panic!("Failed to open {}: {error}", archive_path.display()));
        let decoder = zstd::stream::read::Decoder::new(file)
            .unwrap_or_else(|error| panic!("Failed to decode {}: {error}", archive_path.display()));

        Self {
            archive_path: archive_path.to_path_buf(),
            reader: std::io::BufReader::new(decoder),
            line_buf: String::new(),
            line_number: 0,
        }
    }
}

impl Iterator for V2ArchiveReader {
    type Item = V2ArchiveLine;

    fn next(&mut self) -> Option<Self::Item> {
        self.line_buf.clear();
        let next_line_number = self.line_number + 1;
        let bytes_read = std::io::BufRead::read_line(&mut self.reader, &mut self.line_buf)
            .unwrap_or_else(|error| {
                panic!(
                    "Failed to read {} at line {}: {error}",
                    self.archive_path.display(),
                    next_line_number
                )
            });
        if bytes_read == 0 {
            return None;
        }
        self.line_number = next_line_number;

        let line = self.line_buf.trim_end_matches('\n').trim_end_matches('\r');
        let mut fields = line.splitn(4, '\t');
        let chrom = fields.next().unwrap_or_else(|| {
            panic!(
                "Failed to parse {} line {}: missing chrom",
                self.archive_path.display(),
                self.line_number
            )
        });
        let start_str = fields.next().unwrap_or_else(|| {
            panic!(
                "Failed to parse {} line {}: missing start",
                self.archive_path.display(),
                self.line_number
            )
        });
        let end_str = fields.next().unwrap_or_else(|| {
            panic!(
                "Failed to parse {} line {}: missing end",
                self.archive_path.display(),
                self.line_number
            )
        });
        let data = fields.next().unwrap_or_else(|| {
            panic!(
                "Failed to parse {} line {}: missing data",
                self.archive_path.display(),
                self.line_number
            )
        });

        let start = start_str.parse::<i32>().unwrap_or_else(|error| {
            panic!(
                "Failed to parse {} line {}: invalid start {:?}: {error}",
                self.archive_path.display(),
                self.line_number,
                start_str
            )
        });
        let end = end_str.parse::<i32>().unwrap_or_else(|error| {
            panic!(
                "Failed to parse {} line {}: invalid end {:?}: {error}",
                self.archive_path.display(),
                self.line_number,
                end_str
            )
        });

        Some(V2ArchiveLine {
            chrom: chrom.to_string(),
            start,
            end,
            data: data.to_string(),
        })
    }
}

#[allow(dead_code)]
pub fn assert_v2_module_parity(
    module: &str,
    region_str: &str,
    actual_json: &str,
    golden_json: &str,
) -> Option<String> {
    if actual_json == golden_json {
        return None;
    }

    let offset = actual_json
        .bytes()
        .zip(golden_json.bytes())
        .position(|(actual, golden)| actual != golden)
        .unwrap_or(actual_json.len().min(golden_json.len()));

    let window = 80usize;
    let half = window / 2;
    let golden_start = offset.saturating_sub(half);
    let golden_end = (offset + half).min(golden_json.len());
    let actual_start = offset.saturating_sub(half);
    let actual_end = (offset + half).min(actual_json.len());

    Some(format!(
        "module={module}, region={region_str}\n\
         First divergence at byte offset {offset}\n\
         Golden[{golden_start}..{golden_end}]: {:?}\n\
         Actual[{actual_start}..{actual_end}]: {:?}",
        &golden_json[golden_start..golden_end],
        &actual_json[actual_start..actual_end],
    ))
}
