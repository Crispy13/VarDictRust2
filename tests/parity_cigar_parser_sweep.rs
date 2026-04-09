mod common;

#[test]
#[ignore = "Sweep gate: CigarParser full-sweep parity"]
fn parity_cigar_parser_sweep() {
    common::check_sweep_manifest();
    let regions = common::load_sweep_region_config();

    let mut failures = Vec::new();
    let mut tested = 0;
    let mut skipped = 0;

    for (region, _bam, _ref_path) in &regions {
        let path = common::sweep_fixture_path("cigar_parser", region);
        if !path.exists() {
            skipped += 1;
            continue;
        }

        tested += 1;
        let golden = common::load_sweep_golden_data("cigar_parser", region);
        if golden.is_empty() {
            failures.push(format!("{region}: empty golden data"));
        }
    }

    eprintln!(
        "parity_cigar_parser_sweep: tested={tested}, skipped={skipped}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_cigar_parser_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );

    assert!(
        tested > 0,
        "No sweep fixtures found for cigar_parser. Run: scripts/sweep_fixtures.sh"
    );
}
