mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use vardict_rs::config::Configuration;
use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

/// A small region subset to keep config-variation runtime bounded.
const CONFIG_INDICES: &[usize] = &[0, 1, 2];

#[derive(Clone, Copy)]
struct ConfigPreset {
    name: &'static str,
    regen_flag: &'static str,
    apply: fn(&mut Configuration),
}

const PRESETS: &[ConfigPreset] = &[
    ConfigPreset {
        name: "high_freq",
        regen_flag: "--config high_freq",
        apply: |cfg| cfg.freq = 0.05,
    },
    ConfigPreset {
        name: "low_qual",
        regen_flag: "--config low_qual",
        apply: |cfg| cfg.goodq = 15.0,
    },
    ConfigPreset {
        name: "strict_bias",
        regen_flag: "--config strict_bias",
        apply: |cfg| cfg.min_bias_reads = 5,
    },
];

#[test]
#[ignore = "Requires config-variant goldens: scripts/gen_e2e_golden_tsv.sh --push-only --config <preset> for each preset"]
fn parity_e2e_config_variations() {
    let regions = common::load_region_config();
    let fixture_base = common::e2e_fixture_base();

    for preset in PRESETS {
        for &index in CONFIG_INDICES {
            let (region_str, bam_path, ref_path) = regions.get(index).unwrap_or_else(|| {
                panic!(
                    "Requested region index {index} but testdata/parity_regions.tsv has only {} rows",
                    regions.len()
                )
            });
            let bam_str = bam_path.to_str().expect("BAM path not UTF-8");
            let ref_str = ref_path.to_str().expect("Reference path not UTF-8");
            let fai_path = format!("{ref_str}.fai");
            let chr_lengths = common::load_chr_lengths(&fai_path);

            let actual =
                run_with_preset(preset, region_str, bam_str, ref_str, chr_lengths.clone());
            let golden_path = fixture_base
                .join(preset.name)
                .join(format!("{}.tsv", safe_region_name(region_str)));
            let expected = load_golden_tsv(&golden_path, region_str, preset);

            assert_tsv_parity(preset.name, index, region_str, &actual, &expected);
        }
    }
}

fn run_with_preset(
    preset: &ConfigPreset,
    region_str: &str,
    bam_path: &str,
    ref_path: &str,
    chr_lengths: std::collections::HashMap<String, i32>,
) -> String {
    let _guard = common::init_test_scope_with_bam_config(
        bam_path,
        ref_path,
        chr_lengths.clone(),
        preset.apply,
    );
    let mut region = common::parse_region(region_str);
    region.gene = region.chr.clone();

    let reference_resource = ReferenceResource::new(ref_path, 1200, 0, chr_lengths, false);
    let simple_mode = SimpleMode::new(vec![vec![region]], reference_resource);
    let captured = Arc::new(Mutex::new(String::new()));
    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Buffer(captured.clone()));
    simple_mode.not_parallel();
    let out = {
        let mut guard = captured.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    GlobalReadOnlyScope::clear();
    out
}

fn load_golden_tsv(path: &Path, region_str: &str, preset: &ConfigPreset) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "Missing config-variant golden for preset '{}' region {region_str} at {}: {error}.\nRegenerate with: bash scripts/gen_e2e_golden_tsv.sh --push-only {}",
            preset.name,
            path.display(),
            preset.regen_flag,
        )
    })
}

fn safe_region_name(region: &str) -> String {
    region.replace(':', "_").replace('-', "_")
}

fn assert_tsv_parity(
    preset_name: &str,
    region_index: usize,
    region_str: &str,
    actual: &str,
    expected: &str,
) {
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
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| expected_lines.len().min(actual_lines.len()));

    let expected_line = expected_lines.get(first_diff).copied().unwrap_or("");
    let actual_line = actual_lines.get(first_diff).copied().unwrap_or("");
    panic!(
        "E2E config-variant TSV mismatch: preset '{preset_name}' region {region_index} ({region_str})\nFirst divergent sorted line: {first_diff}\nGolden: {:?}\nActual: {:?}",
        expected_line, actual_line,
    );
}

// Silence unused warning for PathBuf import under conditional paths.
#[allow(dead_code)]
fn _touch_pathbuf() -> PathBuf {
    PathBuf::new()
}
