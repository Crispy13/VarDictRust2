# Sweep Parity

## Overview

The sweep parity gate has two tiers:

- Tier 1: fast gate over 100 sampled regions using committed fixtures in `testdata/fixtures/`.
- Tier 2: promotion gate over the full sampled BED sweep using ephemeral fixtures in `tmp/sweep_fixtures/`.

Tier 1 is for routine validation. Tier 2 is for broader parity coverage before promotion when you need much higher confidence.

## Prerequisites

Activate the Rust build environment before running scripts or tests:

```bash
conda activate vdr && export LIBCLANG_PATH="$CONDA_PREFIX/lib"
```

Required tools:

- `bedtools` for sweep BED generation:

```bash
conda install -c bioconda bedtools
```

- VarDictJava built and runnable from `VarDictJava/build/install/VarDict/bin/VarDict`.
  If needed:

```bash
(cd VarDictJava && ./gradlew installDist)
```

## Workflow

```bash
# Step 1: Generate sweep BEDs for each BAM
scripts/gen_sweep_bed.sh --min-interval 700 testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam na12878_exome
scripts/gen_sweep_bed.sh --min-interval 700 testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam hg002
scripts/gen_sweep_bed.sh --min-interval 700 testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam na12878_lowcov

# Step 2: Generate sweep fixtures
scripts/sweep_fixtures.sh

# Step 3: A-A determinism spot-check
scripts/sweep_aa_check.sh

# Step 4: Run sweep parity tests
cargo test --profile debug-release -- --include-ignored --skip parity_config_e2e_cell_ parity_cigar_parser_sweep
cargo test --profile debug-release -- --include-ignored --skip parity_config_e2e_cell_ parity_realigner_sweep
cargo test --profile debug-release -- --include-ignored --skip parity_config_e2e_cell_ parity_sv_processor_sweep
cargo test --profile debug-release -- --include-ignored --skip parity_config_e2e_cell_ parity_tovars_sweep
```

Notes:

- `scripts/gen_sweep_bed.sh` writes per-chromosome BED files under `tmp/sweep_beds/<bam_tag>/`.
- `scripts/sweep_fixtures.sh` reads those BEDs and emits JSONL fixtures under `tmp/sweep_fixtures/` by default.
- `scripts/sweep_aa_check.sh` samples 100 deterministic regions from `tmp/sweep_beds/` and runs VarDictJava twice per region to catch nondeterminism before Rust-vs-Java parity runs.
- `scripts/sweep_fixtures.sh --bam-tags na12878_exome,hg002` limits fixture generation to selected BAM tags.
- `scripts/sweep_fixtures.sh --dry-run` prints the planned work without invoking VarDictJava.

## Scale

With `--min-interval 700`, the current `na12878_exome` sweep produces about 39,484 tiles. Other BAMs differ, but full-sweep runs are substantially larger than the 100-region Tier 1 gate.

## Stale Fixtures

`tmp/sweep_fixtures/manifest.json` records the `vardictjava_commit` used to generate the sweep fixtures. The sweep tests compare that value against `git -C VarDictJava rev-parse HEAD` and fail fast if they do not match.

For config E2E sweep gates, `scripts/e2e_sweep_gate.py` also validates each staged cached Java TSV against the paired `*.chunks.json` metadata before the cargo parity phase. Staged TSV and sidecar entries may be symlinks; the gate resolves them from the workspace-root staging path before invoking `zstd` or reading JSON so validation stays CWD-safe while preserving both the staged path and resolved target in diagnostics. Missing TSVs, missing fingerprints, incompatible backfilled sidecars, unreadable staged payloads, payload fingerprint mismatches, and provenance compatibility warnings such as `mismatch_generator_flags` or `mismatch_bed_sha256` are all cache-readiness warnings; they require cache refresh or provenance repair and must not be routed as Rust repair evidence.

When `scripts/sweep_fixtures_parallel.py --output-only` is used to refresh TSVs, an expected non-empty BED selection now fails if it finishes without a complete matching TSV plus sidecar. A selection whose BED files contain no intervals is reported explicitly as an empty scope so it can be distinguished from a bad no-op refresh.

If the tests report stale sweep fixtures, regenerate them:

```bash
scripts/sweep_fixtures.sh
```

## Fixture Directory Override

The sweep tests read fixtures from `tmp/sweep_fixtures/` by default. Override that location with `VARDICT_SWEEP_FIXTURE_DIR`:

```bash
export VARDICT_SWEEP_FIXTURE_DIR=/path/to/sweep_fixtures
```

Point it at a directory with the same layout, including `manifest.json` and the module subdirectories.

## Troubleshooting

- `ERROR: bedtools is not available in PATH`
  Install it in the active conda env with `conda install -c bioconda bedtools`.

- `ERROR: sweep BED directory not found` or `ERROR: no sweep BED regions found`
  Run the three `scripts/gen_sweep_bed.sh` commands first, then rerun `scripts/sweep_fixtures.sh` or `scripts/sweep_aa_check.sh`.

- `ERROR: VarDictJava binary not found`
  Build VarDictJava with `(cd VarDictJava && ./gradlew installDist)`. The sweep scripts try to do this automatically, so persistent failures usually mean the Gradle build itself is broken.

- `Sweep fixture directory not found` in Rust tests
  Generate fixtures with `scripts/sweep_fixtures.sh`, or set `VARDICT_SWEEP_FIXTURE_DIR` to an existing sweep fixture root.

- `Sweep fixtures are stale. Regenerate with: scripts/sweep_fixtures.sh`
  The checked-out VarDictJava commit changed after fixture generation. Regenerate the sweep fixtures.

- `scripts/sweep_aa_check.sh` reports run failures or diffs
  Treat that as a VarDictJava determinism issue first; fix the A-A failure before trusting any Rust-vs-Java sweep result.

## Config E2E Wall-Clock Expectations

| Job | Tests / Cells | Threads | Wall |
|-----|---------------|---------|------|
| Binary A (push) | 45 tests | 4 | ~53 s |
| Binary B (full matrix) | 4,500 cells | 10 | ~980 s |
| Surface gate (parity.yml push) | push + matrix | 4 / 10 | ~17 min (dominated by the ~16 min full matrix) |

## VARDICT_CELL_SHARD Scaling

- Single-shard `0/1` is the default and is a no-op today; one job runs all 4,500 cells.
- To fan out to N shards in CI, configure `strategy.matrix.shard: ["0/N", "1/N", ..., "(N-1)/N"]` with `env: VARDICT_CELL_SHARD: ${{ matrix.shard }}`.
- Each shard runs `ceil(4500/N)` cells; shards are independent and can run in parallel.

```yaml
strategy:
  matrix:
    shard: ["0/4", "1/4", "2/4", "3/4"]
env:
  VARDICT_CELL_SHARD: ${{ matrix.shard }}
```