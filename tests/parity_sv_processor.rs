mod common;

#[test]
#[ignore = "Rust SVProcessor not yet implemented"]
fn parity_sv_processor_all_regions() {
    let regions = common::load_region_config();

    for (region, _bam, _ref_path) in &regions {
        let golden = common::load_golden_data("sv_processor", region);
        assert!(
            !golden.is_empty(),
            "golden fixture for {region} should not be empty"
        );
    }
}
