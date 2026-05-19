---
name: perf-optimization-cycle
description: "Run a complete VarDict-rs performance optimization round. Use when the user asks for the next optimization trial/round, says profile first, requests perf-optimization followed by logic-parity-audit, or wants a full cycle of profiling, hotspot selection, scoped Rust optimization, benchmark comparison against Java/VDJ, validation, logic audit, report writing, and commit after a successful no-issue round."
---

# Perf Optimization Cycle

Use this skill for the full optimization workflow. It coordinates the existing detailed skills:

1. `perf-optimization` for measurement, profiling, hotspot analysis, implementation, and performance validation.
2. `logic-parity-audit` after any accepted source change.

The required order is:

```text
profile -> choose one hotspot -> implement one scoped optimization -> validate -> benchmark -> logic audit -> report
```

Do not skip profiling. Do not do speculative optimization from intuition.

## Phase 0: Worktree And Scope

Before profiling:

- Confirm current repo is `/home/eck/workspace/vardict_rs2-2`.
- Run `git status --short --branch`.
- If unrelated user changes exist, do not overwrite them.
- If previous optimization changes are uncommitted, clarify whether the user wants them committed before the next round unless the user already explicitly said to continue.
- Use `tmp/perf_opt_roundNN/` or a similarly clear `tmp/` directory for artifacts.
- Confirm or reuse the negotiated parity thread count before running parity work. This applies to parity checks, parity fixture generation, parity sweep helpers, and logic-audit validation commands. It is not a VarDict runtime behavior change and does not apply to profiling/benchmark commands except where those commands are also parity checks:
  - if the current session has no negotiated parity thread count, ask the user before starting large parity work
  - use `--test-threads=<negotiated>` for parity cargo tests, unless the parity harness requires `--test-threads=1`
  - use `--workers <negotiated>` or fewer for parity fixture/sweep generation helpers
  - use `CARGO_BUILD_JOBS=<negotiated>` when cargo is invoked only to support parity work
  - keep profiling commands single-threaded when required by `perf-optimization` (`--th 1`, `taskset -c 0`, DHAT single-thread traces)

## Phase 1: Profile First

Use the `perf-optimization` skill.

Required baseline/profile artifacts:

- Rust matrix on the fixed panel, unless the user explicitly requests a smaller trial.
- A flamegraph for the workload being optimized.
- At least one of:
  - `perf stat`
  - DHAT
  - cachegrind
  - `/usr/bin/time -v`

For broad optimization rounds, use panel v1:

- Panel file: `tmp/perf_panels/config_matrix_regions_v1.txt`
- Region source: `/home/eck/workspace/vardict_rs2/testdata/parity_regions.tsv`
- Matrix size: 44 configs x 50 regions = 2,200 cells
- CPU pinning: `taskset -c 0`
- Rust harness: `tmp/perf_opt_round33/harness`

Build harness:

```bash
source /home/eck/software/miniconda3/etc/profile.d/conda.sh
conda activate rust_build_env
unset CFLAGS CXXFLAGS CPPFLAGS LDFLAGS ADDR2LINE CXXFILT
export LIBCLANG_PATH="$CONDA_PREFIX/lib"
cargo build --manifest-path tmp/perf_opt_round33/harness/Cargo.toml --profile debug-release
```

Run Rust matrix:

```bash
mkdir -p tmp/perf_opt_roundNN
export PANEL_FILE=tmp/perf_panels/config_matrix_regions_v1.txt
/usr/bin/time -v taskset -c 0 \
  tmp/perf_opt_round33/harness/target/debug-release/vardict_perf_harness \
  matrix tmp/perf_opt_roundNN/matrix_before.tsv \
  2> tmp/perf_opt_roundNN/matrix_before_time.txt
```

Generate flamegraph with `cargo flamegraph` when possible. Keep `ADDR2LINE` unset for better symbol behavior in this environment:

```bash
unset ADDR2LINE CXXFILT
taskset -c 0 cargo flamegraph \
  --manifest-path tmp/perf_opt_round33/harness/Cargo.toml \
  --profile debug-release \
  -p vardict_perf_harness \
  -o /home/eck/workspace/vardict_rs2-2/tmp/perf_opt_roundNN/flamegraph.svg \
  -- matrix /home/eck/workspace/vardict_rs2-2/tmp/perf_opt_roundNN/matrix_profile.tsv
```

If profiling the full panel is too expensive, use a repeated-cell loop or a known slow matrix cell, but explicitly say the profile is narrower than the benchmark panel.

## Phase 2: Choose One Hotspot

Select exactly one optimization target per round unless the user asks otherwise.

Prioritize:

- Profile-confirmed CPU or allocation hotspots.
- Low-risk ownership/borrowing changes.
- Data structure or cache changes that match Java behavior.
- Changes whose output behavior can be checked by matrix byte/status comparison.

Do not optimize:

- Caller sites not visible in the profile.
- Test harness-only code unless the goal is benchmark workflow speed.
- Multiple unrelated modules in one round.

State the chosen target and why before editing.

## Phase 3: Implement Scoped Optimization

Implementation rules:

- Keep the patch small.
- Preserve Java output order and filtering semantics.
- Prefer changes that move Rust closer to Java reference semantics.
- Do not change public behavior unless the user explicitly accepts the risk.
- Add comments only for non-obvious lifecycle or parity-sensitive behavior.

Common accepted patterns from previous rounds:

- Replace deep clones with shared immutable ownership or borrowing.
- Cache read-only external resources when Java already does so.
- Reuse buffers or readers when lifecycle semantics are clear.
- Clone only narrow fields when owned values are required.

## Phase 4: Validate Correctness

Minimum validation after source changes:

```bash
CARGO_BUILD_JOBS=4 cargo build --profile debug-release --bin vardict_rs
CARGO_BUILD_JOBS=4 cargo test --profile debug-release --lib <touched_module_or_keyword> -- --test-threads=4
CARGO_BUILD_JOBS=4 cargo test --profile debug-release --lib -- --test-threads=4
git diff --check
```

Run broader integration tests when local data supports it:

```bash
CARGO_BUILD_JOBS=4 cargo test --profile debug-release -- --skip parity_config_e2e_cell_ --test-threads=4
```

If integration tests fail because local `testdata/parity_regions.tsv` is missing or because of a known unrelated preset assertion, record the exact blocker in the report instead of hiding it.

## Phase 5: Benchmark After

Re-run the same Rust matrix used before:

```bash
export PANEL_FILE=tmp/perf_panels/config_matrix_regions_v1.txt
/usr/bin/time -v taskset -c 0 \
  tmp/perf_opt_round33/harness/target/debug-release/vardict_perf_harness \
  matrix tmp/perf_opt_roundNN/matrix_after.tsv \
  2> tmp/perf_opt_roundNN/matrix_after_time.txt
```

Compare:

- row count
- status count
- output bytes by `(config, region_index)`
- summed cell time
- wall time
- peak RSS
- slowest cells
- biggest improvements/regressions

Zero output byte/status mismatches are required for an accepted optimization.

## Phase 6: Compare Against Java / VDJ

Do not re-run Java every round.

Use reusable Java baseline unless panel/testdata/Java binary/VarDictJava commit changed:

- Java matrix: `tmp/perf_matrix_panel_v1_20260518/java_matrix.tsv`
- Java time/RSS: `tmp/perf_matrix_panel_v1_20260518/java_matrix_time.txt`
- Java commit: `4e362c0ccdae9bba4e378e9f40b15e85ed875d8e`

Generate `java_vs_rust.tsv` for the after matrix:

```bash
python - <<'PY'
import csv
from pathlib import Path

java_path = Path("tmp/perf_matrix_panel_v1_20260518/java_matrix.tsv")
rust_path = Path("tmp/perf_opt_roundNN/matrix_after.tsv")
out_path = Path("tmp/perf_opt_roundNN/java_vs_rust.tsv")

java_rows = list(csv.DictReader(java_path.open(), delimiter="\t"))
rust_rows = list(csv.DictReader(rust_path.open(), delimiter="\t"))
rust = {(r["config"], r["region_index"]): r for r in rust_rows}

with out_path.open("w", newline="") as handle:
    writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
    writer.writerow([
        "config", "tier", "region_index", "region",
        "java_elapsed_s", "rust_elapsed_s", "speedup_java_over_rust",
        "java_output_bytes", "rust_output_bytes", "status_match", "bytes_match",
    ])
    for j in java_rows:
        r = rust[(j["config"], j["region_index"])]
        jt = float(j["elapsed_s"])
        rt = float(r["elapsed_s"])
        writer.writerow([
            j["config"], j["tier"], j["region_index"], j["region"],
            f"{jt:.6f}", f"{rt:.6f}", f"{jt / rt:.6f}" if rt else "",
            j["output_bytes"], r["output_bytes"],
            str(j["status"] == r["status"]).lower(),
            str(j["output_bytes"] == r["output_bytes"]).lower(),
        ])
print(out_path)
PY
```

Report the process-model caveat:

- Java baseline invokes the Java CLI once per cell.
- Rust matrix uses an in-process harness.

## Phase 7: Logic Parity Audit

Use `logic-parity-audit` after any accepted source change.

Audit scope must match touched logic:

- For global/shared state changes: audit Java singleton/resource semantics and Rust lifecycle.
- For CIGAR or parser changes: audit corresponding Java module methods and branch order.
- For cache changes: audit Java cache/lifecycle behavior and Rust equivalent.

Write audit report under:

```text
tmp/logic-parity-audit/<ModuleOrChange>-YYYY-MM-DD.md
```

Report:

- Java files read
- Rust files read
- method/change groups audited
- verified / needs review / failed
- test and matrix evidence
- any integration-test blockers

## Phase 8: Round Report

Write:

```text
tmp/perf_opt_roundNN/report.md
```

Include:

- objective and chosen hotspot
- before/after profile artifacts
- code files changed
- tests run and results
- Rust before vs after metrics
- Java/VDJ vs Rust-after metrics
- output/status mismatch count
- slowest cells after
- flamegraph interpretation
- DHAT/perf stat changes if collected
- logic audit report path
- commit status

Keep `tmp/` artifacts untracked unless the user explicitly asks to commit them.

## Phase 9: Commit Policy

Commit only after a successful optimization round with validated correctness, benchmark improvement, and completed logic parity audit.

Before commit:

- `git status --short`
- `git diff --check`
- confirm only intended source/docs are staged
- do not stage large `tmp/` profiles unless explicitly requested

Commit message should name the optimization, not the benchmark mechanics.
