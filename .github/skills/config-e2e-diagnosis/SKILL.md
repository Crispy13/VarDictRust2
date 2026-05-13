---
name: config-e2e-diagnosis
description: >
   Diagnose config-specific chr1 sweep E2E parity mismatches by consuming
   gate-produced red artifacts first, isolating failures to individual
   pipeline modules, and handing a reviewed plan file to the repair workflow.
   Use when: a chr1 sweep preset fails, E2E mismatch with non-default config,
   final gate after all modules pass per-module cycle, config regression found
   at E2E level.
---

# Config E2E Diagnosis

## When to Use
- After all pipeline modules pass their per-module parity cycle (Steps 0-7)
- A config E2E test from the `parity_config_e2e_push_*` family (Binary A) or the `parity_config_e2e_cell_*` family (Binary B, 4,400 trials in `tests/parity_config_e2e_cells.rs`) fails
- You need to trace an E2E mismatch back to the responsible module

## Prerequisites
- All 6 modules have passed per-module Tier 1 + Tier 2 gates
- Sweep BED root is populated for the active tag (for hg002 chr1 runs: `tmp/sweep_beds_chr1only`)

### chr1 BED-Root Guard
`scripts/e2e_sweep_gate.py` enforces a hard guard when `--chrom 1` is the only chrom requested: if `--sweep-bed-root/<tag>/` contains BED files for chromosomes other than `1`, the gate exits non-zero. Set `VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1` to bypass (a stderr warning is printed). Use `tmp/sweep_beds_chr1only` for chr1 runs to avoid tripping the guard.

- Sweep fixtures are populated under `tmp/sweep_fixtures/output/<preset>/<chrom>/<tag>_<chrom>.tsv.zst`; regenerate one preset at a time with:

```bash
preset=T1-01
bash scripts/gen_e2e_sweep_golden.sh --config "$preset" --tags hg002
```

- VarDictJava is built, and the Rust `parity_e2e_sweep` test target builds successfully
- The ignored sweep test runs for the active tag with `--include-ignored`:

```bash
VARDICT_E2E_SWEEP_CONFIG=T1-01 \
VARDICT_E2E_SWEEP_BED_ROOT="$PWD/tmp/sweep_beds_chr1only" \
CI=true \
cargo test --profile debug-release --test parity_e2e_sweep -- --include-ignored --exact hg002_sweep::parity_e2e_sweep_hg002 --test-threads=1
```

This prerequisite command uses `--exact`, so `--test-threads=1` is incidental to the
single-test smoke check rather than a general sweep policy. For wrapper-driven chr1
sweep reruns, start from the `scripts/e2e_sweep_gate.py` default and override only when
host RAM or diagnosis needs justify it.

### Single-Cell Invocation (Binary B)
Run a single cell by its exact name; use this when a failure points to one (config, region) pair.

```bash
export VARDICT_IMPL=rust
unset PARITY_REGION_INDEX
cargo test --profile debug-release --test parity_config_e2e_cells \
   parity_config_e2e_cell_t1_01_r042 -- --ignored --exact
```

Cells live in `parity_config_e2e_cells` (libtest-mimic harness, `--test parity_config_e2e_cells`); push tests live in `parity_config_e2e` (standard libtest harness, `--test parity_config_e2e`).

## Dispatch Boundaries

- **Phase 1: Evidence intake / fallback rerun** — Orchestrator may dispatch this phase directly with a routed red artifact. No `plan-duck` checkpoint is required before the first pass.
- **Phase 2 / Phase 3: Diagnosis dispatch and repair handoff** — Before this combined dispatch runs after a failure, Orchestrator must run the global `plan-duck` skill and hand this skill the reviewed diagnosis plan file. The completed Phase 2/3 outputs are the artifact set that `plan-duck` turns into the reviewed repair plan file.
- **Phase 4: Repair dispatch** — After Phase 2/3 complete, Orchestrator must run `plan-duck` again on the combined outputs and hand Port Engineer the reviewed repair plan file before `mismatch-repair` starts. The canonical E2E path does not require a separate `shard-diagnosis` checkpoint before this dispatch.
- **Phase 5: Verification reruns** — These are mechanical reruns. Reuse the existing reports and the reviewed repair plan file. Do not insert another `plan-duck` checkpoint unless the scope changes.

### Thread Count Contract
For any wrapper-driven `scripts/e2e_sweep_gate.sh` or `scripts/e2e_sweep_gate.py` run in this workflow, Orchestrator must confirm the chosen `--test-threads` count with the user and record it in the evidence brief or reviewed plan file. If a routed artifact omits that count, stop and return to Orchestrator instead of choosing a default inside this skill.

### Diagnosis-Ready Red Artifact Contract
The default Phase 1 evidence source is the routed gate artifact at the report-dir root, typically `tmp/parity-iteration/<run-id>/parity-failure-report.json`. Do not synthesize `tmp/parity-iteration/<preset>/...` paths.

Treat the artifact as diagnosis-ready only when all of the following are true:
- `schema_version` is `2`
- `diagnosis_artifact.consumer_skill` is `config-e2e-diagnosis`
- `diagnosis_artifact.readiness.status` is `ready`
- each failure entry carries both `preset` and `region_str`

If any of those checks fail, the fallback is a Phase 1 rerun. The gate encodes that fallback explicitly through `diagnosis_artifact.default_action = rerun-phase1-sweep` and `diagnosis_artifact.readiness.fallback_rerun_conditions`.

## Pipeline Module Order (diagnosis sequence)
1. sam_file_parser
2. cigar_parser
3. cigar_modifier
4. realigner
5. sv_processor
6. tovars

## Phase 1: Evidence Collection

### Goal
Identify which (preset, region) pairs produce mismatches at E2E level, starting from the existing red artifact when it is already diagnosis-ready.

### Procedure
1. Read the routed artifact and extract the confirmed `--test-threads` count for this host/run plus the routed `parity-failure-report.json` path. If the routed material does not name the count for the wrapper-driven sweep gate run, stop and return to Orchestrator.
2. If the routed `parity-failure-report.json` exists and satisfies the diagnosis-ready contract above, use it as the default Phase 1 evidence source. Record the failing `(preset, region_str)` pairs from `failures[].preset` and `failures[].region_str`, and carry forward `reproducer_cmd`, `report_path`, `sweep_bed_root`, and `fixture_source` for narrow follow-up. Do not rerun the broad sweep in this case.
3. Only if the routed artifact is missing, unreadable, schema-incompatible, or marked `diagnosis_artifact.readiness.status != ready`, rerun the gate. Start with the narrowest rerun that the routed evidence supports; if you do not already know the failing preset, run the chr1 sweep gate serially across the current 44 presets in `tests/common/mod.rs` `CONFIG_PRESETS` (mirrored in `scripts/config_presets.tsv`):
   ```bash
   set -euo pipefail
   tail -n +2 scripts/config_presets.tsv | cut -f1 | while read -r preset; do
     bash scripts/e2e_sweep_gate.sh \
       --preset "$preset" \
       --tag hg002 \
       --chrom 1 \
       --sweep-bed-root tmp/sweep_beds_chr1only \
       --fixture-source tmp/sweep_fixtures/output \
       --test-threads <confirmed-count> \
       --force
   done
   ```
4. Budget roughly 6-11 minutes per preset, or about 4-8 hours serial for all 44 presets.
5. If you already know which preset to chase, rerun just that preset:
   ```bash
   preset=T1-01
   bash scripts/e2e_sweep_gate.sh \
     --preset "$preset" \
     --tag hg002 \
     --chrom 1 \
     --sweep-bed-root tmp/sweep_beds_chr1only \
     --fixture-source tmp/sweep_fixtures/output \
      --test-threads <confirmed-count> \
     --force
   ```
6. If ALL PASS → config E2E gate passes. Report PASS.
7. If any FAIL → record the failing `(preset, region_str)` pair from the rerun's `parity-failure-report.json` at the report-dir root and reuse that freshly written artifact as the default evidence source for the remaining Phase 1 work.
8. Optional fast smoke: run the `parity_config_e2e_push_*` family when you want a 10-region sanity check, but do not treat it as the Phase 1 gate:
   ```bash
   cargo test --profile debug-release --test parity_config_e2e parity_config_e2e_push_ -- --test-threads=10
   ```
9. **Coverage promotion:** Re-run the `parity_config_e2e_cell_*` family (Binary B, 4,400 cell-level trials) as a broader-region complement to the chr1 sweep result, and use `tiered-config-test` for nightly/sweep expansion across the existing 44-config matrix.

### Outputs
- List of failing (preset_name, region_str) pairs
- The routed or rerun `parity-failure-report.json` path used as Phase 1 evidence, plus the first recorded divergence per failure

## Phase 2: Diagnosis Dispatch

**Dispatch ownership:** Orchestrator runs Phase 2 and Phase 3 together under one reviewed diagnosis plan file so the repair plan is built from a single completed diagnosis/handoff artifact set.

### Goal
For each failing (config, region) pair, identify which pipeline module first produces divergent output.

### Infrastructure-Issue Escalation (MANDATORY)

If the Primary or Secondary Method produces ambiguous, missing, or
silently-degraded output — e.g., `dual_run.py` reports `MISSING` for all
modules, no JSONL files appear in the expected snapshot directory, or
env-var propagation fails — the verifier MUST:

1. **Stop Phase 2.** Do NOT infer the root-cause module from code-ownership
   reasoning alone. Do NOT proceed to Phase 3.
2. **Report the infrastructure defect explicitly** with: the exact command
   run, the expected output, the observed output, and the inspected paths.
3. **Escalate to the orchestrator for an infrastructure fix.** The fix
   must land before Phase 2 resumes.

Code-ownership inference is a last-resort fallback only when infrastructure
is confirmed healthy and the tool still cannot reach a module boundary
(e.g., the root-cause module truly is outside the `SUPPORTED_DEBUG_MODULES`
set). In that case, the inference must be explicitly labeled as such in
the report.

### Primary Method: `dual_run.py --debug-modules`
Use `scripts/dual_run.py --debug-modules` to capture per-module JSONL intermediates for the failing region and config (for example, `1:2324084-2324612` from the chr1 sweep tiles; if a tile is too large, narrow to the smallest sub-window that still reproduces the mismatch), then compare Java vs Rust outputs in pipeline order.

```bash
python scripts/dual_run.py --region {region} --bam {bam} --ref {ref} \
    --config {config_name} --debug-modules cigar_parser realigner sv_processor tovars
```

The script sets `VARDICT_PARITY_{MODULE}` env vars for both Java and Rust, captures JSONL snapshots, and reports the first divergent module in pipeline order.

Supported by `dual_run.py`: `cigar_parser`, `realigner`, `sv_processor`, `tovars`.

### Modules without dual-run JSONL coverage
`sam_file_parser` and `cigar_modifier` are not dual-run comparable yet. Stage 6 F3 verified that Java honors `VARDICT_PARITY_SAM_FILE_PARSER` and `VARDICT_PARITY_CIGAR_MODIFIER` and writes JSONL, but the Java payload schemas do not mirror the Rust snapshot payloads, so `dual_run.py` must keep these modules outside `--debug-modules` until the schemas are aligned.

Implication: if the first divergent stage appears to be `sam_file_parser` or `cigar_modifier`, do not report a clean dual-run module diff. Use the per-module parity suites and the manual raw-intermediate fallback below, and label any schema-normalized comparison as manual evidence rather than `dual_run.py` coverage.

### Sequential Diagnosis Order (reference)
Use this order when narrowing the first divergent stage:
1. `parity_suite sam_file_parser::` / `parity_sweep_suite sam_file_parser_sweep::`
2. `parity_suite cigar_parser::` / `parity_sweep_suite cigar_parser_sweep::`
3. `parity_suite cigar_modifier::` / `parity_sweep_suite cigar_modifier_sweep::`
4. `parity_suite realigner::` / `parity_sweep_suite realigner_sweep::`
5. `parity_suite sv_processor::` / `parity_sweep_suite sv_processor_sweep::`
6. `parity_suite tovars::` / `parity_sweep_suite tovars_sweep::`

### Secondary Method: Manual `VARDICT_PARITY_{MODULE}` fallback
For `sam_file_parser` and `cigar_modifier`, use the same failing region and config but set the module env var manually to a controlled output directory because Rust and Java can emit raw snapshots even though `dual_run.py` cannot compare them directly yet:

```bash
VARDICT_PARITY_SAM_FILE_PARSER=./tmp/manual-sam-file-parser-rust \
VARDICT_IMPL=rust cargo test \
   --profile debug-release --test parity_sweep_suite sam_file_parser_sweep:: -- --nocapture --test-threads=1

VARDICT_PARITY_CIGAR_MODIFIER=./tmp/manual-cigar-modifier-rust \
VARDICT_IMPL=rust cargo test \
   --profile debug-release --test parity_sweep_suite cigar_modifier_sweep:: -- --nocapture --test-threads=1
```

Mirror the same `VARDICT_PARITY_{MODULE}=./tmp/...` variable on the Java side when comparing raw intermediates outside the Rust test harness, then account for the current schema mismatch explicitly.

### Outputs
- Identified root-cause module name
- Specific fixture/shard where divergence occurs
- Brief description of the divergence (field, Java value, Rust value)

### Review Boundary
The global `plan-duck` checkpoint now covers the diagnosis dispatch that runs Phases 2 and 3 together, plus the later repair dispatch. Do not add a duplicate review checkpoint here. If later evidence materially contradicts the isolation result or expands the rerun scope, stop and return to Orchestrator so it can refresh the reviewed plan via `plan-duck` before work continues.

## Phase 3: Repair Handoff

**Canonical handoff artifact:** The reviewed repair plan file carries the diagnosis output from this phase into `mismatch-repair`. The failing test is executed and verified inside `mismatch-repair` Phase 3; this skill defines the required inputs and naming contract.

This phase is completed by Parity Verifier as part of the same diagnosis dispatch that runs Phase 2.

### Goal
Define the reproducible failing test that the reviewed repair plan file must hand off to `mismatch-repair` for the identified module and config+region. The fixture path and loader call must follow the canonical convention defined in `tests/common/mod.rs` (`golden_fixture_path_with_config` / `load_golden_data_with_config`) so the same helpers serve both the bare-region per-module suite and the new config-scoped tests.

### Naming Convention
- **Config slug** — produced by `config_name_to_slug` in `tests/common/mod.rs`: lowercase ASCII, with `-` mapped to `_` (e.g. `T1-01` → `t1_01`, `CM-amplicon` → `cm_amplicon`). Always derive the slug through this helper; never hand-roll it.
- **Region slug** — produced by `safe_region_name` in `tests/common/mod.rs` (e.g. `1:2324084-2324612` → `1_2324084_2324612`).
- **Fixture path** — `testdata/fixtures/{module}/{module}_{config_slug}_{region_safe}.jsonl.zst` (zstd-compressed JSONL, matching the bare-region layout already used by every per-module parity harness).
- **Test function** — `parity_{module}_config_{config_slug}_{region_safe}` (suite-prefixed inside the module's file under `tests/parity_suite/`).

### Procedure
1. Record the fixture that must be generated for the failing `(module, config, region)` triple. Use the same generator the per-module suite already uses (e.g. `scripts/batch_fixtures.sh` or the module-specific `scripts/gen_*_golden*` driver), point it at the failing config preset and region, and place the output at the canonical path above; do **not** invent a parallel directory layout.
2. Record the exact canonical path `testdata/fixtures/{module}/{module}_{config_slug}_{region_safe}.jsonl.zst` — file existence is the first thing the loader checks.
3. Record the `#[test]` that Port Engineer must add to the module's existing parity test file (e.g. `tests/parity_suite/{module}.rs`) using the canonical loader:
   ```rust
   use crate::common::{golden_fixture_path_with_config, load_golden_data_with_config};

   #[test]
   fn parity_{module}_config_{config_slug}_{region_safe}() {
       let region = "1:2324084-2324612";          // failing region from Phase 2
       let config_slug = "T1-01";                  // failing preset from Phase 1 (raw name; helper slugifies)
       let golden = load_golden_data_with_config("{module}", Some(config_slug), region);
       // Drive the module under the matching config and assert byte-identical output to `golden`.
   }
   ```
   The helper accepts the raw preset name (e.g. `"T1-01"`) and applies `config_name_to_slug` internally; do not pre-slugify at the call site.
4. Record in the reviewed repair plan file that `mismatch-repair` Phase 3 must run the test once and confirm it FAILS with the same divergence Phase 2 identified (red-green cycle). A fixture-not-found panic from `load_golden_data_with_config` indicates the path or slug is wrong — recheck step 2 before proceeding.

### Outputs
- Inputs for the reviewed repair plan file: canonical fixture path, test file, function name, and expected mismatch signal

## Phase 4: Repair Dispatch (mismatch-repair)

### Goal
Fix the Rust module to match Java behavior for the identified config+region.

### Procedure
1. Use the Phase 2 isolation report and the Phase 3 failing-test contract as the repair inputs. In the canonical E2E path, do not insert a separate `shard-diagnosis` checkpoint before repair dispatch.
2. Orchestrator runs the global `plan-duck` skill on the Phase 2/3 outputs, writes the reviewed repair plan file, and dispatches Port Engineer with `mismatch-repair`:
   - Input: reviewed repair plan file naming the diagnosis report from Phase 2 and the failing-test artifacts from Phase 3
   - Constraint: in-place repair, no adapter patterns
   - Gate: the Phase 3 failing test must pass inside `mismatch-repair` Phase 3 after the fix
3. Port Engineer implements the fix.
4. Run the Phase 3 test named in the reviewed repair plan file inside `mismatch-repair` Phase 3 — must pass.

### Agent Routing
- **Parity Verifier** runs Phases 1, 2, 3, and 5 and produces the diagnosis report, the Phase 3 failing-test contract named in the reviewed repair plan file, and the final verification report
- **Port Engineer** implements the fix using mismatch-repair
- **Orchestrator** coordinates the handoffs and runs `plan-duck` before diagnosis dispatch and before repair dispatch

## Phase 5: Verify

### Goal
Confirm the fix resolves the original failure without introducing regressions.

### Procedure
1. Reuse the reviewed repair plan file and run the Phase 3 test named there inside `mismatch-repair` Phase 3 — must PASS. Do not insert another `plan-duck` checkpoint before this rerun unless the repair scope changed.
2. Reuse the confirmed `--test-threads` count already recorded in the reviewed repair plan file for any wrapper-driven sweep rerun. If the plan file omits it, stop and return to Orchestrator.
3. Run the full module sweep test — must PASS (no regression):
   ```bash
   cargo test --profile debug-release --test parity_sweep_suite {module}_sweep:: -- --include-ignored --nocapture --test-threads=1
   ```
4. Re-run the chr1 sweep gate for the formerly failing preset — must PASS:
   ```bash
   preset=T1-01
   bash scripts/e2e_sweep_gate.sh \
     --preset "$preset" \
     --tag hg002 \
     --chrom 1 \
     --sweep-bed-root tmp/sweep_beds_chr1only \
     --fixture-source tmp/sweep_fixtures/output \
      --test-threads <confirmed-count> \
     --force
   ```
4. If additional (config, region) pairs still fail, loop back to Phase 2 for the next failure.
5. When all chr1 sweep presets pass → report CONFIG-E2E PASS.

### Outputs
- Final PASS/FAIL status
- List of fixes applied (module, config, brief description)
- Any remaining failures that need attention

## Looping Behavior

This skill operates as a loop:
```
Phase 1 → [for each failure:] Phase 2 → Phase 3 → Phase 4 → Phase 5 → [loop if more failures]
```

The loop terminates when:
- All required chr1 sweep presets pass (PASS verdict), OR
- A fix introduces a regression requiring manual intervention (ESCALATE)

## Related Skills
| Skill | Role |
|-------|------|
| plan-duck | Pre-dispatch review for the diagnosis and repair plan files; skip it for Phase 5 mechanical reruns |
| tiered-config-test | Expand nightly/sweep coverage and tier promotion across the 44-config matrix |
| mismatch-repair | Phase 4: fix methodology for Port Engineer; Phase 3: canonical verification loop for the failing config-e2e test |
| module-parity-test | Phase 5: per-module regression check |

## Agent Responsibilities
| Agent | Phases |
|-------|--------|
| Parity Verifier | Phases 1, 2, 3, 5 (run tests, isolate, define the repair handoff, verify) |
| Port Engineer | Phase 4 (implement fix in mismatch-repair) |
| Gerneral-Purpose Agent | Ad-hoc tasks (fixture generation, script execution) |
| Orchestrator | Coordinates all phases, runs `plan-duck` before diagnosis and repair dispatches, decides loop/escalate |