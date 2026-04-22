# Parity Work - Hybrid Test Setup Handoff

## Scope
The parity test surface now uses a two-binary hybrid split: [tests/parity_config_e2e.rs](tests/parity_config_e2e.rs) holds 46 tests total (44 push tests plus 2 housekeeping checks), while [tests/parity_config_e2e_cells.rs](tests/parity_config_e2e_cells.rs) holds 4,400 ignored cell-level tests under libtest-mimic with `harness = false`. Use Binary A for fast push-surface iteration and Binary B for full cell coverage or single-cell triage.

## Environment Setup
Use the parity shell setup below before local runs. Binary B should also run with `unset PARITY_REGION_INDEX`, and local work should leave `VARDICT_CELL_SHARD` unset.

```bash
source /home/eck/software/miniconda3/lib/python3.12/site-packages/conda/shell/etc/profile.d/conda.sh
conda activate rust_build_env
export LIBCLANG_PATH="$CONDA_PREFIX/lib"
export VARDICT_IMPL=rust
export RAYON_NUM_THREADS=10
unset PARITY_REGION_INDEX
```

## Thread Budget
**HARD CAP 10 TOTAL**

- Never exceed 10 OS threads combined across rayon, libtest, and any external process.
- Binary A should normally use `--test-threads=4`; those 44 push tests may each spawn 2 worker threads internally, so keep the test runner capped conservatively.
- Binary B uses `--test-threads=10`; libtest-mimic honors the flag for the cell runner.
- If Binary A runs alongside an external process such as `samtools`, reduce `--test-threads` by the external process count.
- Treat `RAYON_NUM_THREADS=10` as a ceiling, not a target; individual `--test-threads` values must stay at or below it.

## Running Parity Tests
Binary A is the fast push-surface loop. Binary B covers the full ignored cell surface and single-cell diagnosis.

Fast iteration, Binary A, about 53 seconds:

```bash
cargo test --profile debug-release --test parity_config_e2e -- --test-threads=4
```

Full cell surface, Binary B, about 980 seconds. Keep `unset PARITY_REGION_INDEX` in the shell before this run:

```bash
cargo test --profile debug-release --test parity_config_e2e_cells -- --ignored --test-threads=10
```

Single-cell triage, Binary B, usually seconds:

```bash
cargo test --profile debug-release --test parity_config_e2e_cells parity_config_e2e_cell_<slug>_r<NNN> -- --ignored --exact
```

The single-cell workflow is documented in [.github/skills/config-e2e-diagnosis/SKILL.md](.github/skills/config-e2e-diagnosis/SKILL.md).

## Baseline Discipline
The known-failure baseline lives in [testdata/expected_failing_cells.txt](testdata/expected_failing_cells.txt), and [scripts/config_e2e_surface_gate.sh](scripts/config_e2e_surface_gate.sh) enforces exact equality against it. Any change to that file requires review. If a cell is fixed, remove it from the baseline in the same commit as the code change.

## Pre-Commit Checklist
Before committing parity changes, run the full surface gate locally or rely on CI to run the same gate:

```bash
scripts/config_e2e_surface_gate.sh
```

That gate is about 17 minutes locally. The minimum local check before a parity commit is Binary A green plus:

```bash
scripts/check_ignored_tests.sh
```

## Allowlist And Ignored Handling
The ignored-test allowlist in [scripts/ignored_tests_allowlist.txt](scripts/ignored_tests_allowlist.txt) includes `prefix:parity_config_e2e_cell_`. Do not add exact per-cell entries for Binary B tests.

## Shard Variable
`VARDICT_CELL_SHARD=0/1` is the default single-shard setting that still runs all 4,400 cells. Multi-shard WGS plumbing exists but is deferred; leave `VARDICT_CELL_SHARD` unset for normal local work. Shard scaling and wall-clock context are summarized in [docs/sweep-parity.md](docs/sweep-parity.md).

## Known Failure Profile
There are 34 deterministic failing cells, all in BIAS configs and no latent non-BIAS failures. The exact breakdown is `pw_007=5, t1_14=5, t2_07=8, t3_04=3, t3_08=13`.

## Cross-References
- [.github/skills/config-e2e-diagnosis/SKILL.md](.github/skills/config-e2e-diagnosis/SKILL.md) for single-cell diagnosis workflow.
- [docs/sweep-parity.md](docs/sweep-parity.md) for wall-clock expectations and shard-scaling notes.
- [copilot-office/missions](copilot-office/missions) for related mission notes when parity work is being handed off across agents.