mod common;

#[test]
#[ignore = "Rust CigarParser not yet implemented"]
fn parity_cigar_parser_all_regions() {
    let regions = common::load_region_config();

    for (region, _bam, _ref_path) in &regions {
        let golden = common::load_golden_data("cigar_parser", region);
        assert!(
            !golden.is_empty(),
            "golden fixture for {region} should not be empty"
        );
    }
}
