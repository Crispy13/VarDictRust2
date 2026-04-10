mod common;

use std::collections::HashMap;
use vardict_rs::data::RealignedVariationData;
use vardict_rs::mods::to_vars_builder;
use vardict_rs::reference::ReferenceResource;

/// Load chromosome lengths from a .fai file into a HashMap.
fn load_chr_lengths(fai_path: &str) -> HashMap<String, i32> {
    let content = std::fs::read_to_string(fai_path)
        .unwrap_or_else(|e| panic!("Failed to read FAI file {fai_path}: {e}"));
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
fn parity_tovars_all_regions() {
    let regions = common::load_region_config();

    for (region_str, _bam_path, ref_path) in &regions {
        let _guard = common::init_test_scope();

        let region = common::parse_region(region_str);
        let fai_path = format!("{}.fai", ref_path.display());
        let chr_lengths = load_chr_lengths(&fai_path);

        let reference_resource = ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths,
            false,
        );

        // Load sv_processor golden as input
        let sv_golden = common::load_golden_data("sv_processor", region_str);
        let data: RealignedVariationData = serde_json::from_str(&sv_golden)
            .unwrap_or_else(|e| panic!("Failed to deserialize sv_processor golden for {region_str}: {e}"));

        // Match the shared Java pipeline reference more closely: load the full span of
        // positions present in SVProcessor output, not only the nominal test region.
        let fetch_region = reference_fetch_region(&region, &data);
        let reference = reference_resource
            .get_reference(&fetch_region)
            .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
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
            .unwrap_or_else(|e| panic!("Failed to serialize tovars output for {region_str}: {e}"));

        common::assert_module_parity("tovars", region_str, &result_json);
    }
}
