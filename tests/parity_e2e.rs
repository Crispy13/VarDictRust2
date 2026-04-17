mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

const PUSH_INDICES: &[usize] = &[0, 1, 2, 3, 4, 35, 36, 37, 70, 71];

#[test]
fn parity_e2e_push() {
    run_e2e_suite(
        Some(PUSH_INDICES),
        "bash scripts/gen_e2e_golden_tsv.sh --push-only",
    );
}

#[test]
#[ignore = "Nightly E2E - see parity_e2e_push for fast gate"]
fn parity_e2e_all() {
    run_e2e_suite(None, "bash scripts/gen_e2e_golden_tsv.sh");
}

fn run_e2e_suite(indices: Option<&[usize]>, regeneration_command: &str) {
    let regions = common::load_region_config();
    let fixture_base = common::e2e_fixture_base();

    for (region_index, region_str, bam_path, ref_path) in select_regions(&regions, indices) {
        let bam_str = bam_path.to_str().unwrap_or_else(|| {
            panic!(
                "BAM path for region {region_str} was not valid UTF-8: {}",
                bam_path.display()
            )
        });
        let ref_str = ref_path.to_str().unwrap_or_else(|| {
            panic!(
                "Reference path for region {region_str} was not valid UTF-8: {}",
                ref_path.display()
            )
        });
        let fai_path = format!("{ref_str}.fai");
        let chr_lengths = common::load_chr_lengths(&fai_path);
        let actual = run_simple_mode_region(&region_str, bam_str, ref_str, chr_lengths);
        let golden_path = fixture_base.join(format!("{}.tsv", safe_region_name(&region_str)));
        let expected = load_golden_tsv(&golden_path, &region_str, regeneration_command);

        assert_tsv_parity(region_index, &region_str, &actual, &expected);
    }
}

fn select_regions(
    regions: &[(String, std::path::PathBuf, std::path::PathBuf)],
    indices: Option<&[usize]>,
) -> Vec<(usize, String, std::path::PathBuf, std::path::PathBuf)> {
    match indices {
        Some(indices) => indices
            .iter()
            .map(|&index| {
                let (region, bam_path, ref_path) = regions.get(index).unwrap_or_else(|| {
                    panic!(
                        "Requested region index {index} but testdata/parity_regions.tsv has only {} rows",
                        regions.len()
                    )
                });
                (index, region.clone(), bam_path.clone(), ref_path.clone())
            })
            .collect(),
        None => regions
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, (region, bam_path, ref_path))| (index, region, bam_path, ref_path))
            .collect(),
    }
}

fn run_simple_mode_region(
    region_str: &str,
    bam_path: &str,
    ref_path: &str,
    chr_lengths: std::collections::HashMap<String, i32>,
) -> String {
    let output = {
        let _guard = common::init_test_scope_with_bam(bam_path, ref_path, chr_lengths.clone());
        let mut region = common::parse_region(region_str);

        // Java `-R chr:start-end` emits the chromosome string in the gene column.
        region.gene = region.chr.clone();

        let reference_resource = ReferenceResource::new(ref_path, 1200, 0, chr_lengths, false);
        let simple_mode = SimpleMode::new(vec![vec![region]], reference_resource);
        let captured = Arc::new(Mutex::new(String::new()));
        GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Buffer(captured.clone()));
        simple_mode.not_parallel();
        let captured = take_captured_output(&captured);

        GlobalReadOnlyScope::clear();
        captured
    };

    output
}

fn take_captured_output(buffer: &Arc<Mutex<String>>) -> String {
    let mut output = buffer.lock().unwrap_or_else(|error| error.into_inner());
    std::mem::take(&mut *output)
}

fn load_golden_tsv(path: &Path, region_str: &str, regeneration_command: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "Missing E2E golden for region {region_str} at {}: {error}. Regenerate with: {regeneration_command}",
            path.display()
        )
    })
}

fn safe_region_name(region: &str) -> String {
    region.replace(':', "_").replace('-', "_")
}

fn assert_tsv_parity(region_index: usize, region_str: &str, actual: &str, expected: &str) {
    let mut expected_lines: Vec<&str> = expected.lines().collect();
    let mut actual_lines: Vec<&str> = actual.lines().collect();
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
        "E2E TSV mismatch for region index {region_index} ({region_str})\nFirst divergent sorted line index: {first_diff}\nGolden: {}\nActual: {}",
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
