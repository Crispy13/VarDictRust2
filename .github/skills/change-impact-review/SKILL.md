---
name: change-impact-review
description: "Assess and classify performance impact before approval after any non-exempt code, workflow, or runtime-affecting change. Use when: reviewing code changes, pre-merge performance gate, E2E parity repair performance review, benchmark before/after, runtime telemetry review, regression detection, hot path change, allocator change, collection swap, algorithm change, loop modification, workflow/runtime policy edits. Produces PERF_SAFE / PERF_RISK / PERF_REGRESSION / PERF_PENDING / PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED verdict."
argument-hint: "Describe the change, e.g. 'SV processor find_match loop restructured' or 'review PR #42 for perf impact'"
---

# Change Impact Review

Mandatory pre-approval performance classification for VarDict-rs non-exempt code,
workflow, script, and runtime-affecting changes. Classifies risk, scales evidence to
that risk, runs benchmarks when needed, and produces a binding verdict.

> **Origin**: Created after a parity fix in `StructuralVariantsProcessor` caused a ~20x runtime regression. The fix was correct (parity passed) but no performance check existed to catch the slowdown before it was applied.

## Mandatory Trigger and Exemptions

Use this skill before approval after any non-exempt change that can affect runtime,
parity execution cost, benchmark policy, or workflow behavior. Non-exempt surfaces
include:

- `src/`, `scripts/`, `.github/agents/`, `.github/skills/`, `.github/instructions/`, and `.github/workflows/`
- Test harness behavior, fixture generation, and benchmark/performance policy changes
- `Cargo.toml` feature/profile/dependency changes that can alter runtime behavior
- Any other file that affects execution path, runtime cost, or parity workflow behavior

Narrow exemptions are allowed only with explicit written rationale:

- Pure prose docs-only changes
- Comment-only code changes that cannot affect compilation or runtime
- Isolated `#[cfg(test)]` / `tests/` changes that do not affect fixtures, harness behavior, runtime paths, or performance policy
- Dependency-version-only `Cargo.toml` bumps that do not alter features, profiles, or runtime code paths

Exception: after an E2E parity repair, the config-E2E workflow may still route this skill for a
`PERF_PENDING` review even when the repair looks fixture/provenance-only. In that mode,
verify the diff really has zero `.rs` source/test changes and zero `scripts/` changes
before classifying it as low-risk/data-only.
---

## Caller Context

This skill can be used in advisory or gate mode. The mode affects how the verdict is treated; custom agents are not required.

### Advisory Mode
When this skill is loaded during implementation self-checks:
- Classify risk using the decision tree
- Gather the evidence required for the risk class
- Include the verdict in your implementation report as **advisory**
- A later gate-mode verdict takes precedence

### Gate Mode
When this skill is loaded as the performance review checkpoint:
- A performance verdict is **mandatory** for every non-exempt change — do not skip the section
- Benchmark or telemetry evidence is **mandatory** whenever the selected risk class requires it
- Your verdict is **binding** — it determines whether the change is approved
- `PERF_REGRESSION` **blocks approval** and must be escalated through the active workflow
- `PERF_PENDING` is non-terminal and cannot be silently closed; it expires on the next code change to the same module/surface or at the next full-gate cycle, whichever comes first
- `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED` may proceed only after explicit user acknowledgment and a tracked optimization follow-up
- If an advisory verdict already exists, review it but produce a gate-mode classification
- In the skill-only config-E2E workflow, the current Copilot CLI session records this gate-mode verdict. Because this is not independent custom-agent review, surface any non-`PERF_SAFE` verdict to the user before acceptance.

### E2E Post-Repair Mode
After an E2E parity repair and successful verification rerun, the repair remains
`PERF_PENDING` until this skill emits a terminal non-pending verdict:
`PERF_SAFE`, `PERF_RISK`, `PERF_REGRESSION`, or
`PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED`.
Use the repair diff, the terminal artifact `runtime_summary`, and the referenced
`cell-runtimes.jsonl` as evidence. Runtime telemetry is side-channel metadata; it never
changes parity PASS/FAIL, but it can change the performance verdict.

Rules:
- Fixture/provenance-only classification requires evidence of zero `.rs` source/test
  changes and zero `scripts/` changes in the repair diff. If any Rust or script file
  changed, use the normal risk decision tree.
- If the selected risk class requires measurement and the baseline or required telemetry
  is missing, stale, or insufficient, use `PERF_PENDING` instead of silently downgrading
  the review. Low-risk/data-only repairs may still end at `PERF_RISK` with explicit
  `bootstrap-baseline` notation when no benchmark is required.
- The first successful canonical full-scope gate with runtime telemetry is the baseline
  candidate for later comparisons. Bounded diagnostic runs can inform judgment but cannot
  establish the canonical baseline.
- `PERF_SAFE` requires no hot-path concern, no suspicious slow cells in the runtime
  summary, and either a usable baseline or a clear low-risk/data-only rationale.
- `PERF_RISK` covers documented residual risk, noisy telemetry, low-risk/bootstrap
  baseline cases, or slow cells that are not yet proven regressions.
- `PERF_PENDING` covers insufficient required evidence for the chosen risk class. It must
  carry the missing-evidence reason and the expiry trigger.
- `PERF_REGRESSION` is reserved for clear before/after regressions, obvious hot-path
  slowdowns, or telemetry that shows a repair made the gate materially slower with no
  parity-required justification.
- `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED` is reserved for parity-required fixes whose
  regression is real, documented, explicitly acknowledged by the user, and paired with a
  tracked optimization follow-up.

## Step 0: Determine Whether Review Is Required

1. Check whether the change falls under the non-exempt surfaces listed above.
2. If claiming an exemption, write the exact rationale in the review output.
3. If the rationale is incomplete or the change touches runtime-adjacent surfaces,
   treat it as non-exempt and continue.

## Step 1: Classify Risk

Determine the performance risk level of the change using this decision tree.

```
Is the change in a hot-path module?
├─ YES → Is there an algorithm or data structure change?
│        ├─ YES → HIGH
│        └─ NO  → Is there a new allocation in a loop? New clone()?
│                  ├─ YES → MEDIUM
│                  └─ NO  → LOW
└─ NO  → Is there a new allocation pattern or collection type change?
          ├─ YES → MEDIUM
          └─ NO  → LOW
```

### Hot-Path Modules

These modules execute per-read or per-base — small changes have large impact:

| Module | File | Frequency | Benchmark |
|--------|------|-----------|-----------|
| CigarParser | `src/mods/cigar_parser.rs` | Per-read | `cigar_parser_bench` |
| VariationRealigner | `src/mods/variant_realigner.rs` | Per-region | `pipeline_bench` |
| StructuralVariantsProcessor | `src/mods/structural_variants_processor.rs` | Per-region | `pipeline_bench` |
| ToVarsBuilder | `src/mods/to_vars_builder.rs` | Per-position | `pipeline_bench` |
| Pipeline (record processing) | `src/mods/pipeline.rs` | Per-read | `pipeline_bench` |
| VecMap / InnerMap | `src/data/vecmap.rs` | Per-position | `vecmap_vs_hashmap_bench` |

### Risk Signals

| Signal | Risk | Example |
|--------|------|---------|
| Loop restructure in hot module | HIGH | Changing iteration order in `parse_cigar` |
| New `HashMap`/`IndexMap` in hot loop | HIGH | Adding a lookup table inside per-read processing |
| Algorithm complexity change (O(n)→O(n²)) | HIGH | Nested search replacing direct lookup |
| Collection type swap | MEDIUM | `HashMap` → `BTreeMap`, `Vec` → `LinkedList` |
| New `.clone()` on hot path | MEDIUM | Cloning a `String` or `Vec` per iteration |
| Added `.collect()` mid-pipeline | MEDIUM | Materializing an iterator unnecessarily |
| New branch in cold path | LOW | Additional config check at startup |
| Formatting / output change | LOW | Changing print format in `OutputVariant` |
| Refactor with same logic | LOW | Extracting method, renaming, reordering fields |

---

## Step 2: Act on Risk Level

### LOW Risk

No benchmark required. A classification is still required.

Record in verdict:
```
**Risk**: LOW — {one-line reason}
**Verdict**: PERF_SAFE
```

### MEDIUM Risk

Run the relevant benchmark (from the Hot-Path Modules table above). If no specific benchmark covers the change, use `pipeline_bench` as the integration-level fallback.

```bash
conda activate vdr
export LIBCLANG_PATH=$CONDA_PREFIX/lib

# 1. Benchmark BEFORE the change (stash or checkout base)
git stash  # or: git checkout <base-branch>
cargo bench --profile debug-release --bench <bench_name> -- --save-baseline before

# 2. Benchmark AFTER the change
git stash pop  # or: git checkout <change-branch>
cargo bench --profile debug-release --bench <bench_name> -- --baseline before
```

Evaluate results:
- **≤5% regression**: PERF_SAFE (within noise floor)
- **5–15% regression**: PERF_RISK (document and proceed with acknowledgment)
- **>15% regression**: PERF_REGRESSION (block approval)
- **Required benchmark missing/stale**: PERF_PENDING (record missing evidence + expiry)

### HIGH Risk

Run **both** module benchmark and wall-clock measurement:

```bash
# 1. Module benchmark (same as MEDIUM)
cargo bench --profile debug-release --bench <bench_name> -- --save-baseline before
# ... apply change ...
cargo bench --profile debug-release --bench <bench_name> -- --baseline before

# 2. Wall-clock on representative workload
# BEFORE:
hyperfine --warmup 1 --runs 3 \
  './target/debug-release/vardict -G testdata/hs37d5.fa \
   -b testdata/NA12878.mapped.*.bam -N NA12878 -th 4 \
   -z -c 1 -S 2 -E 3 -g 4 -R "1:100000000-105000000"' \
  2>&1 | tee ./tmp/perf_before.txt

# AFTER:
hyperfine --warmup 1 --runs 3 \
  './target/debug-release/vardict -G testdata/hs37d5.fa \
   -b testdata/NA12878.mapped.*.bam -N NA12878 -th 4 \
   -z -c 1 -S 2 -E 3 -g 4 -R "1:100000000-105000000"' \
  2>&1 | tee ./tmp/perf_after.txt
```

Same thresholds apply (≤5% / 5–15% / >15%).

If the regression is real and parity-required with no practical safe alternative in the
current cycle, do not silently pass it as `PERF_RISK`. Escalate for explicit user
acknowledgment; only then may the verdict become
`PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED`.

---

## Step 3: Produce Verdict

Include this block in the code review output:

```
### Performance Verdict

**Risk**: {HIGH | MEDIUM | LOW}
**Modules touched**: {list of hot-path modules, or "none (cold path)"}
**Benchmark**: {bench name and result, or "not required (LOW risk)"}
**Wall-clock**: {before → after, or "not required"}
**Exemption rationale**: {"none" or explicit exemption text}
**Verdict**: {PERF_SAFE | PERF_RISK | PERF_REGRESSION | PERF_PENDING | PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED}
**Evidence**: {one-line summary, e.g. "pipeline_bench: +2.3% (within noise)"}
**Expiry / follow-up**: {"none" | pending expiry trigger | optimization follow-up path}
```

### Verdict Definitions

| Verdict | Meaning | Action |
|---------|---------|--------|
| `PERF_SAFE` | No measurable regression (≤5%) or LOW risk change | Approve (performance aspect) |
| `PERF_RISK` | Measurable/documented risk that may proceed only with written acceptance rationale | Conditional approval |
| `PERF_PENDING` | Required baseline, benchmark, or telemetry evidence is insufficient | Do not close review; record expiry trigger and return for follow-up |
| `PERF_REGRESSION` | Regression >15% or materially slower hot-path behavior without an approved exception | **Block approval**. Escalate via `perf-optimization` skill |
| `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED` | Real regression required for parity, explicitly acknowledged by the user, with tracked optimization follow-up | Conditional approval only after acknowledgment + follow-up registration |

### Escalation

When verdict is `PERF_REGRESSION`:

1. Do NOT approve the change
2. Report to the orchestrator with the benchmark evidence
3. The orchestrator decides next action:
   - **Redesign**: Ask implementer for an alternative approach
   - **Profile**: Invoke `perf-optimization` skill for root cause analysis
   - **User decision**: Present the trade-off (correctness vs performance) to the user

When verdict is `PERF_PENDING`:

1. Do NOT silently convert it to `PERF_SAFE` or `PERF_RISK`
2. Record exactly what evidence is missing
3. Record the expiry trigger: next same-module/surface code change or next full-gate cycle

When verdict is `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED`:

1. Record the measured regression and why parity requires the fix now
2. Capture explicit user acknowledgment
3. Create or cite the tracked optimization follow-up before allowing approval

---

## Quick Reference: Benchmark Commands

| Benchmark | Command | Covers |
|-----------|---------|--------|
| CigarParser | `cargo bench --profile debug-release --bench cigar_parser_bench` | Per-read CIGAR parsing |
| Pipeline (integration) | `cargo bench --profile debug-release --bench pipeline_bench` | Full region processing |
| VecMap vs HashMap | `cargo bench --profile debug-release --bench vecmap_vs_hashmap_bench` | Collection operations |
| Wall-clock (5MB) | `hyperfine --warmup 1 --runs 3 './target/debug-release/vardict ... -R "1:100000000-105000000"'` | End-to-end runtime |
| Wall-clock (1MB) | `hyperfine --warmup 1 --runs 5 './target/debug-release/vardict ... -R "1:100000000-101000000"'` | Quick sanity check |

## Anti-Patterns

| Don't | Why | Do Instead |
|-------|-----|------------|
| Skip benchmarks for "trivial" hot-path changes | The SV processor incident was a "trivial" restructure | Always benchmark MEDIUM+ changes |
| Benchmark only in debug mode | 10-50x slower, different hot paths | Always `--profile debug-release` |
| Compare single runs | High variance | Use criterion (`--save-baseline`) or `hyperfine` (≥3 runs) |
| Accept >15% regression "because parity requires it" without acknowledgment | Silent debt accumulates and the follow-up disappears | Use `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED` only with explicit acknowledgment and tracked follow-up |
| Run benchmarks with other heavy processes | Noisy results | Quiesce or use `hyperfine` |
