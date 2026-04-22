// parity_e2e_sweep: full-BAM e2e parity vs cached Java TSV.
// Phase 0a fixed sample naming to use the BAM basename, not `test_sample`.
// Regenerate goldens with: bash scripts/gen_e2e_sweep_golden.sh
// Shard runs with: VARDICT_E2E_SWEEP_SHARD=i/N cargo test --profile debug-release --test parity_e2e_sweep -- --include-ignored --test-threads=1
// Scope configs with: VARDICT_E2E_SWEEP_CONFIG=<config>
// Uses GlobalReadOnlyScope and must run with --test-threads=1.
#[path = "common/mod.rs"]
mod common;

#[path = "parity_e2e_sweep/common.rs"]
mod sweep_common;

#[path = "parity_e2e_sweep/hg002_sweep.rs"]
mod hg002_sweep;

#[path = "parity_e2e_sweep/na12878_exome_sweep.rs"]
mod na12878_exome_sweep;

#[path = "parity_e2e_sweep/na12878_lowcov_sweep.rs"]
mod na12878_lowcov_sweep;