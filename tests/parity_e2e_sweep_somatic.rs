//! Full-BAM somatic end-to-end parity harness against cached Java TSV shards.
//!
//! This binary mirrors the R2 sweep aggregator for tumor/normal SomaticMode runs and compares
//! Rust output against cached Java TSV under the configured fixture root (default
//! `tmp/sweep_fixtures/output/`; override the parent with the
//! `VARDICT_E2E_SWEEP_FIXTURE_ROOT` environment variable) using a per-tile multiset compare.
//!
//! Prerequisites:
//! - The fixture root (default `tmp/sweep_fixtures/`, override via
//!   `VARDICT_E2E_SWEEP_FIXTURE_ROOT`) must contain `output/` with somatic cache shards and
//!   `manifest.json`.
//! - The somatic cache is generated against `testdata/GRCh38.d1.vd1.fa`.
//! - Sample naming follows Java's SAMPLE_PATTERN2 applied to the raw `-b tumor|normal` string.
//!
//! Environment:
//! - `VARDICT_E2E_SWEEP_FIXTURE_ROOT=<path>` overrides the fixture-root parent (default `tmp/sweep_fixtures`).
//! - `VARDICT_E2E_SWEEP_SOMATIC_CONFIG=<name>` selects the cache layout; defaults to `default`.
//! - `VARDICT_E2E_SWEEP_SHARD=i/N` optionally runs only one shard of the tile set.
//! - `VARDICT_E2E_SWEEP_BED_ROOT=<path>` optionally overrides sweep BED root discovery.
//! - `CI=true` converts missing-cache handling into a hard panic instead of a local skip.
//!
//! Run with:
//! `cargo test --profile debug-release --test parity_e2e_sweep_somatic -- --include-ignored --test-threads=1`
//!
//! This harness must run with `--test-threads=1` because it initializes GlobalReadOnlyScope.

#[path = "common/mod.rs"]
mod common;

#[allow(dead_code)]
#[path = "parity_e2e_sweep/common.rs"]
mod r2_common;

#[path = "parity_e2e_sweep_somatic/somatic_common.rs"]
mod somatic_common;

#[path = "parity_e2e_sweep_somatic/wes_il_pair_sweep.rs"]
mod wes_il_pair_sweep;
