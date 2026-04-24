mod common;

use std::sync::{Arc, Mutex};

use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

const PUSH_INDICES: &[usize] = &[0, 1, 2, 3, 4, 35, 36, 37, 70, 71];

#[test]
#[ignore = "requires generated tmp/e2e_fixtures goldens not checked into the repo"]
fn parity_e2e_push() {
    run_e2e_suite(
        Some(PUSH_INDICES),
        "bash scripts/gen_e2e_golden_tsv.sh --push-only --config default",
    );
}

#[test]
#[ignore = "Nightly E2E - see parity_e2e_push for fast gate"]
fn parity_e2e_all() {
    run_e2e_suite(None, "bash scripts/gen_e2e_golden_tsv.sh --config default");
}

fn run_e2e_suite(indices: Option<&[usize]>, regeneration_command: &str) {
    let regions = common::load_region_config();
    let fixture_base = common::e2e_fixture_base();
    let implementation = common::resolve_impl();
    let java_flags = Vec::new();

    for (_region_index, region_str, bam_path, ref_path) in select_regions(&regions, indices) {
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
        let expected = common::load_golden_tsv(
            &fixture_base,
            &region_str,
            Some("default"),
            regeneration_command,
        );

        match implementation {
            common::VardictImpl::Rust => {
                let actual = run_simple_mode_region(&region_str, bam_str, ref_str, chr_lengths);
                common::assert_tsv_parity(&actual, &expected, &region_str);
            }
            common::VardictImpl::Java => {
                let actual = common::run_java_region(&region_str, bam_str, ref_str, &java_flags);
                common::assert_tsv_parity(&actual, &expected, &region_str);
            }
            common::VardictImpl::Both => {
                let rust_actual =
                    run_simple_mode_region(&region_str, bam_str, ref_str, chr_lengths);
                common::assert_tsv_parity(&rust_actual, &expected, &region_str);

                let java_actual =
                    common::run_java_region(&region_str, bam_str, ref_str, &java_flags);
                common::assert_tsv_parity(&java_actual, &expected, &region_str);
            }
        }
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
        take_captured_output(&captured)
    };

    output
}

fn take_captured_output(buffer: &Arc<Mutex<String>>) -> String {
    let mut output = buffer.lock().unwrap_or_else(|error| error.into_inner());
    std::mem::take(&mut *output)
}
