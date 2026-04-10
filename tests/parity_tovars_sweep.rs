mod common;

use vardict_rs::data::RealignedVariationData;
use vardict_rs::mods::to_vars_builder;
use vardict_rs::reference::ReferenceResource;

const MAX_FAILURES: usize = 10;

fn reference_fetch_region(
    region: &vardict_rs::data::Region,
    data: &RealignedVariationData,
) -> vardict_rs::data::Region {
    let min_position = data
        .non_insertion_variants
        .keys()
        .chain(data.insertion_variants.keys())
        .copied()
        .min()
        .unwrap_or(region.start)
        .min(region.start);
    let max_position = data
        .non_insertion_variants
        .keys()
        .chain(data.insertion_variants.keys())
        .copied()
        .max()
        .unwrap_or(region.end)
        .max(region.end);

    vardict_rs::data::Region::new(&region.chr, min_position, max_position, "")
}

#[test]
#[ignore = "Sweep gate: ToVarsBuilder full-sweep parity"]
fn parity_tovars_sweep() {
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

    for (region_str, _bam_path, ref_path) in &regions {
        let sv_path = common::sweep_fixture_path("sv_processor", region_str);
        let path = common::sweep_fixture_path("tovars", region_str);
        if !sv_path.exists() || !path.exists() {
            skipped += 1;
            continue;
        }

        tested += 1;
        if tested % 1000 == 0 {
            eprintln!(
                "  [tovars] progress: {tested}/{total} tested, {} failures, {skipped} skipped",
                failures.len()
            );
        }

        let _guard = common::init_test_scope();
        let region = common::parse_region(region_str);

        let reference_resource = ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths.clone(),
            false,
        );

        let sv_golden = common::load_sweep_golden_data("sv_processor", region_str);
        let data: RealignedVariationData = serde_json::from_str(&sv_golden).unwrap_or_else(|error| {
            panic!("Failed to deserialize sv_processor golden for {region_str}: {error}")
        });

        let fetch_region = reference_fetch_region(&region, &data);
        let reference = reference_resource
            .get_reference(&fetch_region)
            .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));
        let ref_map = &reference.reference_sequences;

        let max_read_length = data.max_read_length.unwrap_or(0);
        let ref_coverage = &data.ref_coverage;
        let insertion_variants = &data.insertion_variants;
        let mut non_insertion_variants = data.non_insertion_variants;
        let duprate = data.duprate;

        let result = to_vars_builder::process(
            max_read_length,
            &region,
            ref_map,
            ref_coverage,
            insertion_variants,
            &mut non_insertion_variants,
            duprate,
        );

        let result_json = serde_json::to_string(&result)
            .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

        if let Some(message) = common::assert_sweep_module_parity("tovars", region_str, &result_json)
        {
            failures.push(message);
            if failures.len() >= MAX_FAILURES {
                eprintln!("  [tovars] Reached {MAX_FAILURES} failures, stopping early");
                break;
            }
        }
    }

    eprintln!(
        "parity_tovars_sweep: tested={tested}, skipped={skipped}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_tovars_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No sweep fixtures found for tovars. Run: scripts/sweep_fixtures.sh"
    );
}
