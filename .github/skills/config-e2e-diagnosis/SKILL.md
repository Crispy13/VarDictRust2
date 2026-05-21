---
name: config-e2e-diagnosis
description: >
   Diagnose active-gate config E2E parity mismatches by consuming the freshest
   full-scope gate artifact first, isolating failures to individual pipeline
   modules, and handing a reviewed plan file to the repair workflow. Use when:
   the final config E2E gate fails after all modules pass their per-module
   cycle, a gate-produced red artifact needs diagnosis, or a user-approved
   diagnostic rerun is needed after full-scope evidence is established.
---

# Config E2E Diagnosis

## Canonical Contract

The canonical loop is always:

1. Run the active gate at its full declared scope.
2. Consume the first mismatch from that full-scope artifact.
3. Repair the responsible Rust logic.
4. Re-run the same full declared scope.

This skill must preserve that scope. Do not silently narrow presets, tags,
chromosomes, regions, or modes from a full-scope failure artifact.

Any narrower rerun is diagnostic-only and requires explicit user approval. A
diagnostic rerun must be labeled diagnostic in the report and must never replace
the canonical full-scope artifact as the governing parity claim on its own.

## When to Use

- After all pipeline modules pass their per-module parity cycle (Steps 0-7)
- A config E2E gate artifact or rerun reports a mismatch at E2E level
- You need to trace a full-scope E2E mismatch back to the responsible module

## Prerequisites

- All 6 modules have passed per-module Tier 1 + Tier 2 gates
- VarDictJava is built, and the Rust `parity_e2e_sweep` target builds successfully
- The routed artifact or reviewed plan file names the active gate scope explicitly
- For any wrapper-driven `scripts/e2e_sweep_gate.sh` or `scripts/e2e_sweep_gate.py`
  run, Orchestrator has confirmed the chosen `--test-threads` count with the user
  and recorded it in the routed artifact or reviewed plan file
- Long wrapper-driven E2E parity sweep runs should be launched in sync/blocking terminal
   mode. The routed artifact or reviewed plan file must declare the `--report-dir` so
   humans can monitor `progress.log`, `child-logs/`, `heartbeats/`, `cell-runtimes.jsonl`,
   and the terminal `parity-failure-report.json` or `last-pass.json` there instead of
   requiring periodic agent polling. This monitoring rule must not narrow the active
   full-scope gate.

### Warning Taxonomy

Sweep-gate warning classes are interpreted as follows:

- `blocking` — the artifact is unusable until the underlying workflow problem is fixed
- `not-ready` — the artifact is informative, but it cannot drive canonical diagnosis or repair handoff; refresh full-scope evidence first
- `diagnostic-only` — the artifact remains usable for canonical routing
- unknown warning classes default to `not-ready` until classified

Cache-readiness warnings such as `missing_tsv`, `missing_monolithic_md5`,
`missing_monolithic_bytes`, `incompatible_chunks_json`,
`incompatible_backfilled_chunks`, `mismatch_monolithic_md5`,
`mismatch_monolithic_bytes`, `unreadable_tsv`, `missing_generator_flags`,
`mismatch_generator_flags`, `missing_bed_sha256`, and `mismatch_bed_sha256`
are `not-ready`. Treat unreadable staged paths, payload fingerprint failures,
and provenance/schema compatibility warnings as fixture/cache infrastructure
problems requiring cache refresh or provenance repair, not as Rust repair
evidence.

### State Authority Drift

Older session reports and some mission files were missing or inaccessible during the
impact analysis that produced this workflow update. When documentary state and live
artifacts disagree, prefer the freshest acknowledged artifact set, not stale summaries.

## Dispatch Boundaries

- **Phase 1: Evidence intake / full-scope refresh** — Orchestrator may dispatch this phase directly with a routed red artifact. No `plan-duck` checkpoint is required before the first pass.
- **Phase 2 / Phase 3: Diagnosis dispatch and repair handoff** — Before this combined dispatch runs after a failure, Orchestrator must run the global `plan-duck` skill and hand this skill the reviewed diagnosis plan file.
- **Phase 4: Repair dispatch** — After Phase 2/3 complete, Orchestrator must run `plan-duck` again on the combined outputs and hand Port Engineer the reviewed repair plan file before `mismatch-repair` starts.
- **Phase 5: Verification reruns** — These are mechanical reruns of the same full declared scope. Do not insert another `plan-duck` checkpoint unless the scope changes.

## Diagnosis-Ready Red Artifact Contract

The default Phase 1 evidence source is the routed gate artifact at the report-dir root,
typically `tmp/parity-iteration/<run-id>/parity-failure-report.json`. Do not synthesize
alternate artifact roots.

Treat the artifact as diagnosis-ready only when all of the following are true:

- `schema_version` is `2`
- `diagnosis_artifact.consumer_skill` is `config-e2e-diagnosis`
- `diagnosis_artifact.readiness.status` is `ready`
- `planned_cell_count`, `planned_pair_count`, `tested_cell_count`, `tested_pair_count`, `halted_early`, `original_matrix_scope`, and `warning_summary` are present
- `warning_summary.readiness_impact.status` is `ready`
- each failure entry carries `preset`, `tag`, and `region_str`

Optional runtime telemetry fields are compatible with the same schema-v2 contract:

- `runtime_summary` may appear at the artifact root and must be treated as side-channel
   metadata, not parity evidence.
- When present, `runtime_summary.cell_runtimes_path` points at the report-root
   `cell-runtimes.jsonl` file containing per-chunk runtime records.
- Missing or partial runtime telemetry does not change diagnosis readiness by itself;
   Review Gate/change-impact-review consumes it during the post-repair performance step.

Compatibility behavior for older schema-v2 artifacts:

- Older schema-v2 artifacts without the completeness or warning fields remain readable as historical context.
- They are not canonical full-scope evidence and must not drive repair handoff.
- Refresh full-scope evidence before resuming the canonical loop.

If any diagnosis-ready check fails, the fallback is a full-scope Phase 1 refresh. The
gate encodes that fallback explicitly through `diagnosis_artifact.default_action =
rerun-phase1-sweep` and `diagnosis_artifact.readiness.fallback_rerun_conditions`.

## Pipeline Module Order

1. sam_file_parser
2. cigar_parser
3. cigar_modifier
4. realigner
5. sv_processor
6. tovars

## Phase 1: Evidence Collection

### Goal

Identify which `(preset, region)` pairs produce mismatches at E2E level, starting from
the existing full-scope red artifact when it is already diagnosis-ready.

### Procedure

1. Read the routed artifact and extract the routed `parity-failure-report.json` path,
   the confirmed `--test-threads` count, and the artifact's recorded full-scope fields.
   If the routed material does not name the count for the wrapper-driven sweep gate run,
   stop and return to Orchestrator.
2. If the routed artifact exists and satisfies the diagnosis-ready contract above, use it
   as the default Phase 1 evidence source. Record the failing `(preset, tag, region_str)`
   triples from the artifact and carry forward `reproducer_cmd`, `report_path`,
   `fixture_source`, `sweep_bed_root`, `test_threads`, and `original_matrix_scope`.
   Do not rerun the broad sweep in this case.
3. Only if the routed artifact is missing, unreadable, schema-incompatible, incomplete,
   or marked not ready may you rerun the sweep gate. That rerun must preserve the active
   gate's full declared scope recorded by Orchestrator or the routed artifact. Do not
   silently collapse to a subset.
4. If a narrower diagnostic rerun is needed after full-scope evidence is established,
   stop and obtain explicit user approval first. Label the rerun diagnostic in the report
   and retain the full-scope artifact as the governing checkpoint.
5. If the refreshed full-scope rerun fails, record the first mismatch from the new
   `parity-failure-report.json` and reuse that freshly written artifact as the default
   Phase 1 evidence source for the remaining work.
6. If the refreshed full-scope rerun passes, the config E2E gate passes. Report PASS.
7. Optional fast smoke: run the `parity_config_e2e_push_*` family only as a user-approved
   diagnostic aid. Do not treat it as the canonical Phase 1 gate.
8. Coverage promotion remains the job of `tiered-config-test`, not an implicit fallback
   from this skill.

### Outputs

- List of failing `(preset_name, tag, region_str)` tuples
- The routed or refreshed `parity-failure-report.json` path used as Phase 1 evidence
- The exact original full-scope matrix recorded in the artifact

## Phase 2: Diagnosis Dispatch

**Dispatch ownership:** Orchestrator runs Phase 2 and Phase 3 together under one
reviewed diagnosis plan file so the repair plan is built from a single completed
diagnosis/handoff artifact set.

### Goal

For each failing `(config, tag, region)` tuple, identify which pipeline module first
produces divergent output.

### Exact Freshness Replay Gate (MANDATORY)

Before any repair handoff, replay the exact failing tuple captured in the routed artifact
or reviewed diagnosis plan file:

- preset
- tag
- region or tile
- fixture source
- sweep BED root
- test-thread context

If that exact replay is impossible, no longer reproduces, or requires a scope change,
stop before repair handoff and return to Phase 1 for a fresh full-scope evidence refresh.

### Infrastructure-Issue Escalation (MANDATORY)

If the Primary or Secondary Method produces ambiguous, missing, or silently degraded
output — for example, `dual_run.py` reports `MISSING` for all modules, no JSONL files
appear in the expected snapshot directory, or env-var propagation fails — the verifier
must:

1. Stop Phase 2. Do not infer the root-cause module from code ownership alone.
2. Report the infrastructure defect explicitly with the exact command run, expected output,
   observed output, and inspected paths.
3. Escalate to Orchestrator for an infrastructure fix before Phase 2 resumes.

Code-ownership inference is a last-resort fallback only when infrastructure is confirmed
healthy and the tool still cannot reach a module boundary. In that case, label the
inference explicitly.

### Primary Method: `dual_run.py --debug-modules`

Use `scripts/dual_run.py --debug-modules` to capture per-module JSONL intermediates for
the exact failing region and config, then compare Java vs Rust outputs in pipeline order.

```bash
python scripts/dual_run.py --region {region} --bam {bam} --ref {ref} \
    --config {config_name} --debug-modules cigar_parser realigner sv_processor tovars
```

The script sets `VARDICT_PARITY_{MODULE}` env vars for both Java and Rust, captures JSONL
snapshots, and reports the first divergent module in pipeline order.

Supported by `dual_run.py`: `cigar_parser`, `realigner`, `sv_processor`, `tovars`.

### Modules without dual-run JSONL coverage

`sam_file_parser` and `cigar_modifier` are not dual-run comparable yet. If the first
divergent stage appears there, do not report a clean `dual_run.py` diff. Use the
per-module parity suites and the manual raw-intermediate fallback below, and label any
schema-normalized comparison as manual evidence rather than `dual_run.py` coverage.

### Sequential Diagnosis Order

1. `parity_suite sam_file_parser::` / `parity_sweep_suite sam_file_parser_sweep::`
2. `parity_suite cigar_parser::` / `parity_sweep_suite cigar_parser_sweep::`
3. `parity_suite cigar_modifier::` / `parity_sweep_suite cigar_modifier_sweep::`
4. `parity_suite realigner::` / `parity_sweep_suite realigner_sweep::`
5. `parity_suite sv_processor::` / `parity_sweep_suite sv_processor_sweep::`
6. `parity_suite tovars::` / `parity_sweep_suite tovars_sweep::`

### Secondary Method: Manual `VARDICT_PARITY_{MODULE}` fallback

For `sam_file_parser` and `cigar_modifier`, use the same failing region and config but
set the module env var manually to a controlled output directory:

```bash
VARDICT_PARITY_SAM_FILE_PARSER=./tmp/manual-sam-file-parser-rust \
VARDICT_IMPL=rust cargo test \
   --profile debug-release --test parity_sweep_suite sam_file_parser_sweep:: -- --nocapture --test-threads=1

VARDICT_PARITY_CIGAR_MODIFIER=./tmp/manual-cigar-modifier-rust \
VARDICT_IMPL=rust cargo test \
   --profile debug-release --test parity_sweep_suite cigar_modifier_sweep:: -- --nocapture --test-threads=1
```

Mirror the same `VARDICT_PARITY_{MODULE}=./tmp/...` variable on the Java side when
comparing raw intermediates outside the Rust test harness, then account for the current
schema mismatch explicitly.

### Outputs

- Identified root-cause module name
- Specific fixture or shard where divergence occurs
- Brief description of the divergence (field, Java value, Rust value)
- Evidence that the exact freshness replay tuple still reproduces

### Review Boundary

The global `plan-duck` checkpoint covers the diagnosis dispatch that runs Phases 2 and 3
together, plus the later repair dispatch. Do not add a duplicate review checkpoint here.
If later evidence materially contradicts the isolation result, stops reproducing, or
expands the rerun scope, stop and return to Orchestrator so it can refresh the reviewed
plan via `plan-duck` before work continues.

## Phase 3: Repair Handoff

**Canonical handoff artifact:** The reviewed repair plan file carries the diagnosis output
from this phase into `mismatch-repair`. The failing test is executed and verified inside
`mismatch-repair` Phase 3; this skill defines the required inputs and naming contract.

This phase is completed by Parity Verifier as part of the same diagnosis dispatch that
runs Phase 2.

### Goal

Define the reproducible failing test that the reviewed repair plan file must hand off to
`mismatch-repair` for the identified module and config+region. The fixture path and
loader call must follow the canonical convention defined in `tests/common/mod.rs`
(`golden_fixture_path_with_config` / `load_golden_data_with_config`).

### Naming Convention

- **Config slug** — produced by `config_name_to_slug` in `tests/common/mod.rs`
- **Region slug** — produced by `safe_region_name` in `tests/common/mod.rs`
- **Fixture path** — `testdata/fixtures/{module}/{module}_{config_slug}_{region_safe}.jsonl.zst`
- **Test function** — `parity_{module}_config_{config_slug}_{region_safe}`

### Procedure

1. Record the fixture that must be generated for the failing `(module, config, region)`
   triple at the canonical path above.
2. Record the exact canonical path — file existence is the first thing the loader checks.
3. Record the `#[test]` that Port Engineer must add to the module's existing parity test
   file using the canonical loader helpers.
4. Record in the reviewed repair plan file that `mismatch-repair` Phase 3 must run the
   test once and confirm it fails with the same divergence Phase 2 identified.
5. Include the exact freshness replay tuple evidence from Phase 2 in the reviewed repair
   plan file. A stale or scope-shifted replay must block repair handoff.

### Outputs

- Inputs for the reviewed repair plan file: canonical fixture path, test file, function
  name, expected mismatch signal, and the exact freshness replay tuple

## Phase 4: Repair Dispatch (mismatch-repair)

### Goal

Fix the Rust module to match Java behavior for the identified config+region.

### Procedure

1. Use the Phase 2 isolation report and the Phase 3 failing-test contract as the repair inputs.
2. Orchestrator runs the global `plan-duck` skill on the Phase 2/3 outputs, writes the
   reviewed repair plan file, and dispatches Port Engineer with `mismatch-repair`.
3. Port Engineer implements the fix using `mismatch-repair`.
4. Run the Phase 3 test named in the reviewed repair plan file inside `mismatch-repair`
   Phase 3 — it must pass. Then run the focused/module verification required by the
   repair plan.
5. For any proven Rust `mismatch-repair`, read the repair diff after the fix exists and
   focused/module verification has passed. Use the touched Rust module or logic surface
   as the `logic-parity-audit` scope. If the diff is broad or ambiguous, audit the full
   touched module.
6. Run `logic-parity-audit` before Review Gate / `change-impact-review`. If the audit
   returns NEEDS_REVIEW, route findings back to Port Engineer before Review Gate.

Infrastructure-only repairs, cache refreshes, provenance fixes, and harness repairs do
not trigger `logic-parity-audit`; they follow the infrastructure/workflow review path.

### Agent Routing

- **Parity Verifier** runs Phases 1, 2, 3, and 5 and produces the diagnosis report, the
  Phase 3 failing-test contract, and the final verification report
- **Port Engineer** implements the fix using `mismatch-repair`
- **Orchestrator** coordinates the handoffs and runs `plan-duck` before diagnosis and
  repair dispatches

## Phase 5: Verify

### Goal

Confirm the fix resolves the original failure without introducing regressions.

### Procedure

1. Reuse the reviewed repair plan file and run the Phase 3 test named there inside
   `mismatch-repair` Phase 3 — it must pass.
2. Reuse the confirmed `--test-threads` count already recorded in the reviewed repair
   plan file for any wrapper-driven sweep rerun. If the plan file omits it, stop and
   return to Orchestrator.
3. Run the full module sweep test — it must pass with no regression.
4. For a proven Rust `mismatch-repair`, read the repair diff and run
   `logic-parity-audit` for the touched Rust module or logic surface before Review Gate
   approval and before the full-scope rerun is accepted as the next canonical step. If
   the diff is broad or ambiguous, audit the full touched module. If the audit returns
   NEEDS_REVIEW, route findings back to Port Engineer for targeted fixes before Review
   Gate.
5. Re-run the same full declared gate scope recorded in the reviewed repair plan file and
   the source artifact's `original_matrix_scope`. Do not silently narrow verification to
   only the formerly failing preset, tag, chromosome, or region.
6. If that same full-scope rerun passes after a parity repair, record the repair as
   `PERF_PENDING` and return to Orchestrator for Review Gate/change-impact-review before
   treating the repair as approved. The final repair report must include a terminal
   non-pending verdict: `PERF_SAFE`, `PERF_RISK`, `PERF_REGRESSION`, or
   `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED`. If evidence is still insufficient,
   keep the repair at `PERF_PENDING`, document the missing evidence, and record the
   expiry trigger: next same-module/surface code change or next full-gate cycle.
7. If additional failures remain in that same full scope, loop back to Phase 2 for the
   next failure.
8. When the same full-scope gate passes and the post-repair performance verdict is not
   `PERF_REGRESSION`, report CONFIG-E2E PASS only after the Review Gate result is
   terminal. `PERF_RISK` is conditional and must carry the Review Gate rationale,
   including bootstrap-baseline notation when applicable. `PERF_REGRESSION_ACCEPTED_PARITY_REQUIRED`
   also requires explicit user acknowledgment and a tracked optimization follow-up.

### Outputs

- Final PASS/FAIL status
- List of fixes applied (module, config, brief description)
- Any remaining failures that need attention

## Looping Behavior

This skill operates as a loop:

```text
full-scope Phase 1 -> [for each failure:] Phase 2 -> Phase 3 -> Phase 4 -> focused/module verification -> logic-parity-audit for Rust repairs -> Review Gate/change-impact-review -> full-scope Phase 5 -> [loop if more failures]
```

The loop terminates when:

- The same full declared gate scope passes, OR
- A fix introduces a regression or infrastructure defect requiring manual intervention

## Historical / Diagnostic Notes

- HG002 and single-chromosome reruns are historical examples and may still appear in
  older handoff notes. They are not the governing rule.
- `--chrom 1` or a chr1-only BED root is valid only when the user explicitly approves a
  diagnostic single-chrom rerun or when the active gate itself was intentionally scoped
  that way.
- `VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1` is diagnostic-only context for approved
  single-chrom reruns against a broader BED root. It is not canonical workflow guidance.

## Related Skills

| Skill | Role |
|-------|------|
| plan-duck | Pre-dispatch review for the diagnosis and repair plan files; skip it for Phase 5 mechanical reruns |
| tiered-config-test | Expand nightly/sweep coverage and tier promotion across the 44-config matrix |
| mismatch-repair | Phase 4 fix methodology for Port Engineer; Phase 3 canonical verification loop for the failing config-e2e test |
| module-parity-test | Phase 5 per-module regression check |
| logic-parity-audit | Mandatory post-repair audit for proven Rust divergence repairs before Review Gate/full-scope acceptance |

## Agent Responsibilities

| Agent | Phases |
|-------|--------|
| Parity Verifier | Phases 1, 2, 3, 5 |
| Port Engineer | Phase 4 |
| Gerneral-Purpose Agent | Ad-hoc tasks |
| Orchestrator | Coordinates phases, runs `plan-duck`, and enforces the full-scope contract |