# VarDict-rs Parity Scope

**Status:** Authoritative. This document defines the exact surface of behavior for which
VarDict-rs claims byte-identical parity with VarDictJava.

**Last updated:** This commit. Updates require a corresponding commit to
[scripts/config_presets.tsv](scripts/config_presets.tsv) or
[tests/common/mod.rs](tests/common/mod.rs) that changes what is covered, verified by
[scripts/check_preset_drift.sh](scripts/check_preset_drift.sh).

---

## What "100% parity" means, operationally

**Byte-identical output**: for every covered `(mode, config, lane, chromosome, region)`
tuple, the Rust binary's stdout (and stderr where asserted) must match Java's stdout
byte-for-byte.

No statistical agreement. No column-set agreement. No "mostly matches". Exact byte
equality. The parity harness under [tests/](tests/) compares with `assert_eq!` on byte
slices.

Non-goals are listed below. Behavior outside this scope is explicitly unclaimed.

---

## Covered surface

### Modes
- **SimpleMode** — germline single-BAM variant calling.
- **SomaticMode** — tumor/normal paired calling.

### Execution model
- **Single-threaded by default, plus bounded `CM-TH4` coverage.** The parity matrix
  now includes `CM-TH4` (`-th 4`) for SimpleMode and the in-process config/sweep
  harnesses dispatch the real Rust `parallel()` path behind a harness-level thread
  budget. Broader thread counts, broader execution modes, and unbounded multi-thread
  sweeps remain out of scope — see
  [docs/handoff-multithreading.md](docs/handoff-multithreading.md).

### Input format
- **BAM** (indexed, `.bai` present).
- CRAM is out of scope.

### Reference
- **GRCh37/hg19** via `testdata/hs37d5.fa`.
- **GRCh38** via `testdata/GRCh38.d1.vd1.fa` (somatic lane only).

### Flag surface
The 45 presets in [scripts/config_presets.tsv](scripts/config_presets.tsv) cover the
following flags. Any flag not in this list is unclaimed:

| Flag | Axis | Coverage |
|------|------|----------|
| `-f` | Allele-frequency threshold | T1-02, T1-03, T1-05, T1-06, T1-07 + T2/T3/PW combinations |
| `-r` | Minimum variant reads | T1-08 + T2/T3/PW combinations |
| `-q` | Minimum base quality | T1-03, T1-09, T1-10 + T2/T3/PW combinations |
| `-m` | Mismatch limit | T1-04, T1-11, T1-12 + T2/T3/PW combinations |
| `-X` | Vext (realignment window) | T1-04, T1-13 + T2/T3/PW combinations |
| `-B` | Bias-read requirement | T1-05, T1-14 + T2/T3/PW combinations |
| `-th` | SimpleMode worker count | `CM-TH4` only (`-th 4`) |

Additional dedicated presets exercise execution-model and call-mode flags:
`--fisher` (CM-FISHER), `-th 4` (CM-TH4), `-p` (CM-PILEUP), `-U` (CM-NOSV),
`-k 0` (CM-NOREAL), `--chimeric` (CM-CHIMERIC), `-Q 30` (CM-MAPQ30).
Somatic-only flags `-M`, `-V`, `-I` are exercised by the somatic default config on
the `wes_il_pair` tag.

### Sweep lanes
| Tag | Mode | Reference | BAM(s) |
|-----|------|-----------|--------|
| `hg002_agilent_v5` | Simple | hs37d5 | HG002 Agilent v5 exome |
| `na12878_chrom20_exome` | Simple | hs37d5 | NA12878 chrom20 exome |
| `na12878_low_coverage` | Simple | hs37d5 | NA12878 whole-genome low-coverage |
| `wes_il_pair` | Somatic | GRCh38 | WES tumor/normal pair |

### Preset tiers
- **T1-01..T1-14** (14 rows) — single-axis threshold variation.
- **T2-\***, **T3-\*** — dual- and three-to-five-axis threshold combinations
  (7 rows each after the CM-* swap; tier-column totals are listed below).
- **PW-000..PW-009** (10 rows) — pairwise interaction coverage across 6 threshold
  flags.
- **CM-\*** (7 rows, named `CM-FISHER`, `CM-TH4`, `CM-PILEUP`, `CM-NOSV`,
  `CM-NOREAL`, `CM-CHIMERIC`, `CM-MAPQ30`) — call-mode and bounded multi-thread
  presets exercising distinct execution branches. The current tier-column
  distribution is T1=14 / T2=11 / T3=10 / PW=10.

**Total: 45 rows.**

Drift between the TSV, the `CONFIG_PRESETS` constant in
[tests/common/mod.rs](tests/common/mod.rs), and the
[tiered-config-test skill](.github/skills/tiered-config-test/SKILL.md) is enforced by
`scripts/check_preset_drift.sh`, wired into
[.github/workflows/parity.yml](.github/workflows/parity.yml).

---

## Explicitly out of scope (non-goals)

These are acknowledged gaps. They do **not** count against the parity claim.

### Modes
- **AmpliconMode** — no handoff doc yet; Rust has `AmpliconMode` struct but no parity
  harness.
- **SplicingMode** — not implemented in Rust. Java has
  [SplicingMode.java](VarDictJava/src/main/java/com/astrazeneca/vardict/modes/SplicingMode.java).

### Execution
- **Broader multi-threaded execution** beyond the fixed `CM-TH4` SimpleMode coverage.
  Broader thread counts, unbounded harness concurrency, and broader mode acceptance
  remain out of scope. Status and follow-up plan:
  [docs/handoff-multithreading.md](docs/handoff-multithreading.md).
- **Distributed / multi-process execution** — not in Java either.

### Input
- **CRAM input** — not supported in Rust.
- **Stream stdin BAM** — not tested.

### Flag surface (unclaimed)
Any flag not in the coverage table above — including but not limited to `-E`, `-G`,
`-z`, `-S`, `-R`, `-a`, `-o`, `-O`, `-s`, `-t`, `-g`, `-L`, `-P`, `-N` — is unclaimed.
Individual flags may happen to work; none are asserted byte-identical.

### Platform
- **Non-Linux targets** (macOS, Windows) — build may succeed but no parity assertions
  on those platforms.

### Output format
- **VCF output** via `-v` — unclaimed. Primary output format is tab-separated text.

---

## Verification gates

A claim of "100% parity" is substantiated by the following CI gates, **all green**:

| Gate | Workflow | Test target |
|------|----------|-------------|
| Module-level JSONL parity | `parity.yml` | `parity_*.rs` binaries (cigar, realigner, tovars, etc.) |
| E2E surface gate | `parity.yml` | `scripts/config_e2e_surface_gate.sh` |
| E2E config cells | `parity.yml` | `parity_config_e2e_cells` |
| E2E full sweep (single-BAM) | `sweep.yml` nightly | `parity_e2e_sweep` with `--include-ignored` on all 3 single-BAM tags |
| E2E full sweep (somatic) | `sweep.yml` nightly | `parity_e2e_sweep_somatic` with `--include-ignored` on `wes_il_pair` |
| Preset drift gate | `parity.yml` pre-test | `scripts/check_preset_drift.sh` |

Full parity claim requires all rows × all covered chromosomes × all 45 presets to be
green in at least one `sweep.yml` nightly run. Partial coverage (e.g., smoke tier
only) does **not** support the full claim.

### Canonical E2E Evidence

The config E2E workflow distinguishes between diagnostic first-failure artifacts and a
completed full-matrix parity claim:

- A fail-fast red artifact is diagnostic evidence only. It may drive mismatch isolation,
  but it is not a completed parity checkpoint.
- A completed full-scope artifact must record the same declared matrix it set out to run,
  with `tested_cell_count == planned_cell_count`, `tested_pair_count == planned_pair_count`,
  `halted_early == false`, and `warning_summary.readiness_impact.status == ready`.
- Repair handoff from a red artifact requires an exact live freshness replay of the
  failing preset, tag, region or tile, fixture source, sweep BED root, and test-thread
  context before the workflow may proceed.
- When workflow documents and live artifacts disagree, prefer the freshest acknowledged
  artifact set. Stale mission summaries do not override newer executable evidence.

---

## Scope changes

Adding a new mode, new input format, or new flag to the claimed surface requires:

1. Parity harness addition (new test binary or new test function under
   `parity_e2e_sweep` / `parity_e2e_sweep_somatic`).
2. Golden fixture generation via `scripts/gen_e2e_sweep_golden.sh`.
3. TSV + `CONFIG_PRESETS` + skill-doc update (enforced by
   `scripts/check_preset_drift.sh`).
4. This document updated.
5. CI nightly green for 3 consecutive runs before the claim expands.

Removing from scope requires: document update + explicit note in the commit message
explaining why. Removed scope should remain in this doc's "Previously covered" section
(not yet present; add if/when removal happens).

---

## Related documents

- [docs/handoff-multithreading.md](docs/handoff-multithreading.md) — port plan for
  multi-threaded execution.
- [.github/skills/tiered-config-test/SKILL.md](.github/skills/tiered-config-test/SKILL.md) — preset tier execution policy.
- [.github/skills/workflow-management/SKILL.md](.github/skills/workflow-management/SKILL.md) — process for changing test infrastructure.
- [/memories/repo/preset-redundancy-audit.md](../memories/repo/preset-redundancy-audit.md) — redundant-row analysis for future CM-* swap.
- [Goal.md](Goal.md) — project goal narrative.
