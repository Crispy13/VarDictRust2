#[test]
#[ignore = "Sweep gate: full-BAM E2E parity for hg002. Run via: cargo test --profile debug-release --test parity_e2e_sweep parity_e2e_sweep_hg002 -- --include-ignored --test-threads=1. Requires tmp/sweep_fixtures/output cache; regenerate via: bash scripts/gen_e2e_sweep_golden.sh"]
fn parity_e2e_sweep_hg002() {
    super::sweep_common::run_tag("hg002");
}