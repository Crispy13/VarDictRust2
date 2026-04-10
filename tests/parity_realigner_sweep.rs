mod common;

use std::collections::HashSet;
use std::sync::Arc;

use vardict_rs::data::VariationData;
use vardict_rs::mods::variation_realigner;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;

#[test]
#[ignore = "Sweep gate: Realigner full-sweep parity"]
fn parity_realigner_sweep() {
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
        let cp_path = common::sweep_fixture_path("cigar_parser", region_str);
        let path = common::sweep_fixture_path("realigner", region_str);
        if !cp_path.exists() || !path.exists() {
            skipped += 1;
            continue;
        }

        tested += 1;
        if tested % 1000 == 0 {
            eprintln!(
                "  [realigner] progress: {tested}/{total} tested, {} failures, {skipped} skipped",
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
        let reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));
        let reference = Arc::new(reference);

        let cp_golden = common::load_sweep_golden_data("cigar_parser", region_str);
        let variation_data: VariationData = serde_json::from_str(&cp_golden).unwrap_or_else(|error| {
            panic!("Failed to deserialize cigar_parser golden for {region_str}: {error}")
        });

        let bam_str = bam_path.to_str().unwrap();
        let scope = Scope::new(
            bam_str,
            region.clone(),
            reference,
            reference_resource,
            variation_data.max_read_length.unwrap_or(0),
            HashSet::new(),
            VariantPrinter::Out,
            variation_data,
        );

        let result_scope = variation_realigner::process(scope);
        let result_json = serde_json::to_string(&result_scope.data)
            .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

        if let Some(message) = common::assert_sweep_module_parity("realigner", region_str, &result_json)
        {
            failures.push(message);
            if failures.len() >= MAX_FAILURES {
                eprintln!("  [realigner] Reached {MAX_FAILURES} failures, stopping early");
                break;
            }
        }
    }

    eprintln!(
        "parity_realigner_sweep: tested={tested}, skipped={skipped}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_realigner_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No sweep fixtures found for realigner. Run: scripts/sweep_fixtures.sh"
    );
}
