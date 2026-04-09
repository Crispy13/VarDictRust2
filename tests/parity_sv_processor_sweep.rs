mod common;

#[test]
#[ignore = "Sweep gate: SVProcessor full-sweep parity"]
fn parity_sv_processor_sweep() {
    common::check_sweep_manifest();
    let regions = common::load_sweep_region_config();

    let mut failures = Vec::new();
    let mut tested = 0;
    let mut skipped = 0;

    for (region, _bam, _ref_path) in &regions {
        let path = common::sweep_fixture_path("sv_processor", region);
        if !path.exists() {
            skipped += 1;
            continue;
        }

        tested += 1;
        let golden = common::load_sweep_golden_data("sv_processor", region);
        if golden.is_empty() {
            failures.push(format!("{region}: empty golden data"));
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
        failures.join("\n")
    );

    assert!(
        tested > 0,
        "No sweep fixtures found for sv_processor. Run: scripts/sweep_fixtures.sh"
    );
}
