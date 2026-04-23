//! Determinism parity tests. Run with `--test-threads=1` to stay under the
//! 10-worker test-harness ceiling (N=8 workers + 1 test-runner thread = 9).

mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const THREAD_COUNTS: [usize; 4] = [1, 2, 4, 8];
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

#[test]
fn parity_parallel_determinism_default_th1() {
    assert_parallel_matches_serial("default", 1);
}

#[test]
fn parity_parallel_determinism_default_th2() {
    assert_parallel_matches_serial("default", 2);
}

#[test]
fn parity_parallel_determinism_default_th4() {
    assert_parallel_matches_serial("default", 4);
}

#[test]
fn parity_parallel_determinism_default_th8() {
    assert_parallel_matches_serial("default", 8);
}

#[test]
fn parity_parallel_determinism_t1_14_th1() {
    assert_parallel_matches_serial("T1-14", 1);
}

#[test]
fn parity_parallel_determinism_t1_14_th2() {
    assert_parallel_matches_serial("T1-14", 2);
}

#[test]
fn parity_parallel_determinism_t1_14_th4() {
    assert_parallel_matches_serial("T1-14", 4);
}

#[test]
fn parity_parallel_determinism_t1_14_th8() {
    assert_parallel_matches_serial("T1-14", 8);
}

fn assert_parallel_matches_serial(config_name: &str, thread_count: usize) {
    assert!(
        THREAD_COUNTS.contains(&thread_count),
        "unsupported thread count {thread_count}; expected one of {THREAD_COUNTS:?}"
    );

    let fixture = fastest_region_fixture();
    let config_flags = config_flags(config_name);
    let case_name = format!("{config_name} x th={thread_count}");

    let started = Instant::now();
    let serial = run_vardict_subprocess(&fixture, &config_flags, 1);
    let parallel = run_vardict_subprocess(&fixture, &config_flags, thread_count);
    assert_bytes_equal(&serial, &parallel, &case_name, &fixture.region);

    eprintln!(
        "determinism case passed: {case_name} region={} elapsed={:?}",
        fixture.region,
        started.elapsed()
    );
}

#[derive(Clone, Debug)]
struct FixtureRegion {
    region: String,
    bam_path: PathBuf,
    reference_path: PathBuf,
}

fn fastest_region_fixture() -> FixtureRegion {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    common::load_region_config()
        .into_iter()
        .min_by_key(|(region, _, _)| region_span_len(region))
        .map(|(region, bam_path, reference_path)| FixtureRegion {
            region,
            bam_path: manifest_dir.join(bam_path),
            reference_path: manifest_dir.join(reference_path),
        })
        .unwrap_or_else(|| panic!("testdata/parity_regions.tsv did not contain any regions"))
}

fn region_span_len(region: &str) -> i32 {
    let (_chr, range) = region
        .split_once(':')
        .unwrap_or_else(|| panic!("invalid parity region format: {region}"));
    let (start, end) = range
        .split_once('-')
        .unwrap_or_else(|| panic!("invalid parity region range: {region}"));
    let start = start
        .parse::<i32>()
        .unwrap_or_else(|error| panic!("invalid region start in {region}: {error}"));
    let end = end
        .parse::<i32>()
        .unwrap_or_else(|error| panic!("invalid region end in {region}: {error}"));
    end - start + 1
}

fn config_flags(config_name: &str) -> Vec<String> {
    match config_name {
        "default" => Vec::new(),
        other => common::config_preset_java_flags(other),
    }
}

fn run_vardict_subprocess(
    fixture: &FixtureRegion,
    config_flags: &[String],
    thread_count: usize,
) -> Vec<u8> {
    let bam = path_as_str(&fixture.bam_path, "BAM", &fixture.region);
    let reference = path_as_str(&fixture.reference_path, "reference", &fixture.region);

    let mut command = Command::new(env!("CARGO_BIN_EXE_vardict_rs"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("-G")
        .arg(reference)
        .arg("-b")
        .arg(bam)
        .arg("-N")
        .arg("test_sample")
        .args(config_flags)
        .arg("--th")
        .arg(thread_count.to_string())
        .arg("-R")
        .arg(&fixture.region)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = run_command_with_timeout(
        command,
        RUN_TIMEOUT,
        &format!(
            "running vardict_rs for region {} config {config_flags:?} threads={thread_count}",
            fixture.region
        ),
    );

    assert!(
        output.status.success(),
        "vardict_rs failed for region {} config {config_flags:?} threads={thread_count} with exit status {}\nSTDERR:\n{}\nSTDOUT:\n{}",
        fixture.region,
        output.status,
        String::from_utf8_lossy(&output.stderr).trim(),
        String::from_utf8_lossy(&output.stdout).trim(),
    );

    output.stdout
}

fn path_as_str<'a>(path: &'a Path, kind: &str, region: &str) -> &'a str {
    path.to_str().unwrap_or_else(|| {
        panic!(
            "{kind} path for region {region} was not valid UTF-8: {}",
            path.display()
        )
    })
}

fn run_command_with_timeout(mut command: Command, timeout: Duration, description: &str) -> Output {
    let mut child = command.spawn().unwrap_or_else(|error| {
        panic!("Failed to start {description}: {error}")
    });

    let stdout_handle = child.stdout.take().map(spawn_output_reader);
    let stderr_handle = child.stderr.take().map(spawn_output_reader);
    let started = Instant::now();

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(50)),
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

fn assert_bytes_equal(expected: &[u8], actual: &[u8], case_name: &str, region: &str) {
    if actual == expected {
        return;
    }

    let offset = expected
        .iter()
        .zip(actual.iter())
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(actual.len()));

    let expected_line = line_at_offset(expected, offset);
    let actual_line = line_at_offset(actual, offset);
    let line_number = first_diverging_line_number(expected, actual);

    panic!(
        "Parallel determinism mismatch for {case_name} in region {region}\nFirst divergent byte offset: {offset}\nFirst divergent line: {line_number}\nExpected line: {:?}\nActual line: {:?}\nExpected bytes: {}\nActual bytes: {}",
        String::from_utf8_lossy(expected_line),
        String::from_utf8_lossy(actual_line),
        hex_dump(expected_line),
        hex_dump(actual_line),
    );
}

fn first_diverging_line_number(expected: &[u8], actual: &[u8]) -> usize {
    let expected_lines: Vec<&[u8]> = expected.split_inclusive(|byte| *byte == b'\n').collect();
    let actual_lines: Vec<&[u8]> = actual.split_inclusive(|byte| *byte == b'\n').collect();

    expected_lines
        .iter()
        .zip(actual_lines.iter())
        .position(|(left, right)| left != right)
        .map(|index| index + 1)
        .unwrap_or_else(|| expected_lines.len().min(actual_lines.len()) + 1)
}

fn line_at_offset(bytes: &[u8], offset: usize) -> &[u8] {
    let start = bytes[..offset.min(bytes.len())]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = bytes[offset.min(bytes.len())..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|index| offset.min(bytes.len()) + index)
        .unwrap_or(bytes.len());
    &bytes[start..end]
}

fn hex_dump(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}