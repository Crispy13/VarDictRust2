mod common;

#[test]
#[ignore = "Rust Realigner not yet implemented"]
fn parity_realigner_all_regions() {
    let regions = common::load_region_config();

    for (region, _bam, _ref_path) in &regions {
        let golden = common::load_golden_data("realigner", region);
        assert!(
            !golden.is_empty(),
            "golden fixture for {region} should not be empty"
        );
    }
}
