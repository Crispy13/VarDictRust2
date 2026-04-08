mod common;

#[test]
#[ignore = "Rust ToVars not yet implemented"]
fn parity_tovars_all_regions() {
    let regions = common::load_region_config();

    for (region, _bam, _ref_path) in &regions {
        let golden = common::load_golden_data("tovars", region);
        assert!(
            !golden.is_empty(),
            "golden fixture for {region} should not be empty"
        );
    }
}
