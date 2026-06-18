---
name: production-bench
description: >-
  Production benchmark of VarDictJava (vdj) vs VarDict-rs (vdr): head-to-head resource usage
  (wall / user CPU / sys CPU / peak RSS) AND output parity, on coverage-derived BEDs at the
  production thread count (-th 8). Use this WHENEVER the user wants to compare vdj and vdr
  performance, measure VarDict-rs vs VarDictJava production time, check "resource usage and
  parity", run a "production bench", see how vdr compares to vdj on a real workload, or
  benchmark both tools head-to-head. Standalone (not part of the parity workflow); produces a
  decision-ready REPORT.md. Parity failure invalidates a cell's perf numbers.
---

# Production bench — vdj vs vdr

Answer one question reproducibly: **how does VarDict-rs (vdr) compare to VarDictJava (vdj) on a
production-shaped workload, in resource usage and output parity?** Build BEDs from each BAM set's real
coverage, run *both* tools with identical flags at `-th 8`, and report a Java-vs-Rust table plus a
byte-identical parity verdict. A perf number is only valid when that cell's output is parity-clean.

This skill is **read-only on the codebase** — it only runs the two binaries and writes under
`opt/claude/tmp/production-bench/`. It never edits source and never commits.

## Fixed matrix

**Workloads (2):**
| key | lane | reference | BAM(s) | note |
|---|---|---|---|---|
| `germline` | germline WGS | `testdata/hs37d5.fa` | `NA12878...low_coverage...bam` | genomecov+run limited to chr1 (`--region 1`) to stay tractable |
| `somatic`  | somatic WES pair | `testdata/GRCh38.d1.vd1.fa` | `WES_IL_T_1...bam` \| `WES_IL_N_1...bam` | paired (`-b "T|N"`); covered region = tumor∩normal |

**Presets (4)** — flags from `vardict_rs2/scripts/config_presets.tsv`:
| preset | flags | vdr CLI support |
|---|---|---|
| `T1-01` | *(default, none)* | ✅ |
| `T1-02` | `-f 0.005 -r 1 -q 15` | ✅ |
| `T1-06` | `-f 0.001 -r 1 -q 20 -m 12` | ✅ |
| `CM-NOSV` | `-U` | ✅ (vdr's CLI now exposes the full VarDictJava option set incl. `-U`) |

**Metrics:** wall + user CPU + sys CPU + peak RSS, `-th 8` only, via `/usr/bin/time -v`, median of N runs
(1 warmup discarded). Reported as Java, Rust, and Rust/Java ratio (< 1.00x = Rust faster/leaner).

## How to run

Everything runs inside the `vdr` conda env (provides `samtools`/`bedtools` and matches the vdr build).

1. **Build vdr at HEAD** (the binary under test):
   ```bash
   source /home/eck/software/miniconda3/etc/profile.d/conda.sh && conda activate vdr
   export LIBCLANG_PATH=$CONDA_PREFIX/lib
   export BINDGEN_EXTRA_CLANG_ARGS="-I$CONDA_PREFIX/lib/clang/22/include"
   cargo build --profile debug-release --bin vardict_rs   # run from opt/claude
   ```

2. **Smoke** (cheap, ~1 min — germline tiny region, validates the whole pipeline):
   ```bash
   bash .claude/skills/production-bench/scripts/run_matrix.sh --smoke
   ```

3. **Full matrix** (user-invoked; minutes per cell × 2 workloads × supported presets × 2 tools × N runs):
   ```bash
   bash .claude/skills/production-bench/scripts/run_matrix.sh --runs 3 --budget-mb 20
   ```
   Tune scope with `--workloads "germline somatic"`, `--presets "T1-01 T1-06"`, `--budget-mb N`.

Read the result at `opt/claude/tmp/production-bench/REPORT.md`.

## What the scripts do

- **`make_cov_bed.sh`** — `bedtools genomecov -bg → merge → makewindows -w 700`, then annotate each
  window with mean coverage and **systematically sample across the coverage distribution** down to a size
  budget (`--budget-mb`, default 20 Mb) so a `-th 8` run finishes in minutes while still reflecting the
  real coverage mix. Two BAMs → intersection (somatic). Cached per workload+budget; reused on re-run.
- **`run_matrix.sh`** — resolves each workload, ensures its BED, and for every preset runs vdj then vdr
  with matching flags (`-G ref -b … -N … -th 8 -z 1 <preset> -c 1 -S 2 -E 3 -s 2 -e 3 BED`) under
  `/usr/bin/time -v` for N+1 runs (warmup discarded). **Both tools are given `-z 1`** (zero-based BED) to
  match bedtools' 0-based windows; vdr now supports `-z` (and the full VDJ option set), so the flags are
  identical on both sides. vdr uses `-th` (normalized to `--th`). Captures `uptime` at start.
- **`report.py`** — medians the time logs, checks parity (byte-identical; sorted-identical fallback for
  thread-order), and writes `REPORT.md`. Parity mismatch/missing → cell **INVALID** (perf numbers not to
  be trusted). Flags "idle re-measure owed" when start loadavg > 2.

## Interpreting results

- **Parity must be IDENTICAL** for a cell's numbers to count. vdr is a faithful byte-identical port, so a
  DIFFER result signals a real regression (or a flag/version skew) — investigate before trusting timing.
- **Load matters.** A/B on this box has historically been noisy under the codex sweep (loadavg ~9). If the
  report flags load, re-run on an idle box before drawing conclusions.
- The benchmarked vdr binary is the **`debug-release`** profile (the project's standard). A PGO /
  target-cpu=native "production" build would be a separate, faster artifact — note that when reporting.

## Guardrails

- Build, run, and write **only** inside `vardict_rs_claude` (the skill dir + `opt/claude/tmp/`).
  `vardict_rs2` and `VarDictJava` (sibling source) are read-only; the 4 preset rows are copied in, so
  there is no runtime dependency on the sibling.
- `vdr` conda env for the build and for samtools/bedtools.
- Report-only: never edit source, never commit/push.
