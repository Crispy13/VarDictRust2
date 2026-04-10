mod common;

use std::collections::HashMap;
use std::sync::Arc;

use vardict_rs::data::{RealignedVariationData, Sclip, VariationMap};
use vardict_rs::mods::structural_variants_processor;
use vardict_rs::reference::ReferenceResource;

const MAX_FAILURES: usize = 10;

#[test]
#[ignore = "Sweep gate: SVProcessor full-sweep parity"]
fn parity_sv_processor_sweep() {
    common::check_sweep_manifest();
    let regions = common::load_sweep_region_config();
    let total = regions.len();

    let first_ref = regions
        .first()
        .map(|(_, _, ref_path)| ref_path)
        .unwrap_or_else(|| panic!("No sweep regions found in tmp/sweep_fixtures/regions.tsv"));
    let fai_path = format!("{}.fai", first_ref.display());
    let chr_lengths = common::load_chr_lengths(&fai_path);

    let mut failures = Vec::new();
    let mut tested = 0usize;
    let mut skipped = 0usize;

    for (region_str, bam_path, ref_path) in &regions {
        let realigner_path = common::sweep_fixture_path("realigner", region_str);
        let path = common::sweep_fixture_path("sv_processor", region_str);
        if !realigner_path.exists() || !path.exists() {
            skipped += 1;
            continue;
        }

        tested += 1;
        if tested % 1000 == 0 {
            eprintln!(
                "  [sv_processor] progress: {tested}/{total} tested, {} failures, {skipped} skipped",
                failures.len()
            );
        }

        let _guard = common::init_test_scope();
        let region = common::parse_region(region_str);

        let reference_resource = Arc::new(ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths.clone(),
            false,
        ));
        let mut reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));

        let realigner_golden = common::load_sweep_golden_data("realigner", region_str);
        let mut data: RealignedVariationData = serde_json::from_str(&realigner_golden).unwrap_or_else(|error| {
            panic!("Failed to deserialize realigner golden for {region_str}: {error}")
        });

        let bam_str = bam_path.to_str().unwrap();
        let bams: Option<Vec<String>> = Some(vec![bam_str.to_string()]);
        let splice: Option<std::collections::BTreeSet<String>> = None;
        let mut prev_non_insertion_variants: HashMap<i32, VariationMap> = HashMap::new();
        let mut prev_ref_coverage: HashMap<i32, i32> = HashMap::new();
        let mut prev_soft_clips_3_end: HashMap<i32, Sclip> = HashMap::new();
        let mut prev_soft_clips_5_end: HashMap<i32, Sclip> = HashMap::new();
        let prev_reference_sequences: HashMap<i32, u8> = HashMap::new();

        structural_variants_processor::process(
            &mut data,
            &mut reference,
            &reference_resource,
            &region,
            &bams,
            &splice,
            &mut prev_non_insertion_variants,
            &mut prev_ref_coverage,
            &mut prev_soft_clips_3_end,
            &mut prev_soft_clips_5_end,
            &prev_reference_sequences,
            "",
            0,
        );

        let result_json = serde_json::to_string(&data)
            .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

        if let Some(message) =
            common::assert_sweep_module_parity("sv_processor", region_str, &result_json)
        {
            failures.push(message);
            if failures.len() >= MAX_FAILURES {
                eprintln!("  [sv_processor] Reached {MAX_FAILURES} failures, stopping early");
                break;
            }
        }
    }

    eprintln!(
        "parity_sv_processor_sweep: tested={tested}, skipped={skipped}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_sv_processor_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No sweep fixtures found for sv_processor. Run: scripts/sweep_fixtures.sh"
    );
}
