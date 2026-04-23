#[test]
#[ignore = "Sweep gate: E2E somatic parity for wes_il_pair (~4.88M tiles, 14 configs). Run via: cargo test --profile debug-release --test parity_e2e_sweep_somatic wes_il_pair_sweep:: -- --include-ignored --test-threads=1. Requires tmp/sweep_fixtures/output/ cache (somatic entries); regenerate via bash scripts/gen_e2e_sweep_golden.sh --somatic --tags wes_il_pair --force. See tests/parity_e2e_sweep_somatic.rs for env vars."]
fn parity_e2e_sweep_somatic_wes_il_pair() {
    super::somatic_common::run_pair("wes_il_pair");
}