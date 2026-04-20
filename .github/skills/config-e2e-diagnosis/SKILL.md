---
name: config-e2e-diagnosis
description: >
  Diagnose and fix config-specific E2E parity mismatches by isolating failures
  to individual pipeline modules. Use when: config E2E test fails, E2E mismatch
  with non-default config, final gate after all modules pass per-module cycle,
  config regression found at E2E level.
---

# Config E2E Diagnosis

## When to Use
- After all pipeline modules pass their per-module parity cycle (Steps 0-7)
- A config E2E test from the `parity_config_e2e_push_*` family or `parity_config_e2e_all` fails
- You need to trace an E2E mismatch back to the responsible module

## Prerequisites
- All 6 modules have passed per-module Tier 1 + Tier 2 gates
- `parity_config_e2e.rs` tests exist and golden fixtures are generated
- Java and Rust binaries are built
- Install nextest first if it is not already available: `cargo install cargo-nextest --locked`.

## Pipeline Module Order (diagnosis sequence)
1. sam_file_parser
2. cigar_parser
3. cigar_modifier
4. realigner
5. sv_processor
6. tovars

## Phase 1: Run Config E2E

### Goal
Identify which (config, region) pairs produce mismatches at E2E level.

### Procedure
1. Run the `parity_config_e2e_push_*` test family (10 regions × 44 configs = 440 test cells):
   ```bash
   cargo nextest run --cargo-profile debug-release --test parity_config_e2e -E 'test(/parity_config_e2e_push_/)' -j 10
   ```
2. If ALL PASS → config E2E gate passes. Report PASS.
3. If any FAIL → record failing (config, region) pairs from test output.
4. **Coverage promotion:** Re-run `parity_config_e2e_all` for broader region coverage and use `tiered-config-test` for nightly/sweep expansion across the existing 44-config matrix.

### Outputs
- List of failing (config_name, region_str) pairs
- Test output showing first divergence per failure

## Phase 2: Isolate to Module

### Goal
For each failing (config, region) pair, identify which pipeline module first produces divergent output.

### Primary Method: `dual_run.py --debug-modules`
Use `scripts/dual_run.py --debug-modules` to capture per-module JSONL intermediates for the failing region and config, then compare Java vs Rust outputs in pipeline order.

```bash
python scripts/dual_run.py --region {region} --bam {bam} --ref {ref} \
    --config {config_name} --debug-modules cigar_parser realigner sv_processor tovars
```

The script sets `VARDICT_PARITY_{MODULE}` env vars for both Java and Rust, captures JSONL snapshots, and reports the first divergent module in pipeline order.

Supported by `dual_run.py`: `cigar_parser`, `realigner`, `sv_processor`, `tovars`.

Rust-side ready, manual fallback required until `dual_run.py` support lands: `sam_file_parser`, `cigar_modifier`.

### Sequential Diagnosis Order (reference)
Use this order when narrowing the first divergent stage:
1. `parity_sam_file_parser` / `parity_sam_file_parser_sweep`
2. `parity_cigar_parser` / `parity_cigar_parser_sweep`
3. `parity_cigar_modifier` / `parity_cigar_modifier_sweep`
4. `parity_realigner` / `parity_realigner_sweep`
5. `parity_sv_processor` / `parity_sv_processor_sweep`
6. `parity_tovars` / `parity_tovars_sweep`

### Secondary Method: Manual `VARDICT_PARITY_{MODULE}` fallback
For `sam_file_parser` and `cigar_modifier`, use the same failing region and config but set the module env var manually because Rust supports these debug snapshots even though `dual_run.py` still marks them deferred:

```bash
VARDICT_PARITY_SAM_FILE_PARSER=1 VARDICT_IMPL=rust cargo test \
   --profile debug-release --test parity_sam_file_parser_sweep -- --nocapture

VARDICT_PARITY_CIGAR_MODIFIER=1 VARDICT_IMPL=rust cargo test \
   --profile debug-release --test parity_cigar_modifier_sweep -- --nocapture
```

Mirror the same `VARDICT_PARITY_{MODULE}` variable on the Java side when comparing raw intermediates outside the Rust test harness.

### Outputs
- Identified root-cause module name
- Specific fixture/shard where divergence occurs
- Brief description of the divergence (field, Java value, Rust value)

## Phase 3: Create Failing Test

### Goal
Create a reproducible failing test that targets the identified module with the specific config+region.

### Procedure
1. Extract or generate a fixture for the failing (module, config, region) combination.
2. Add the fixture to `testdata/fixtures/{module}/` with a config-specific name.
3. Add a `#[test]` to the module's existing parity test file (e.g., `tests/parity_{module}.rs`):
   ```rust
   #[test]
   fn parity_{module}_config_{config_name}_{region_safe}() {
       // Load fixture with config applied
       // Assert byte-identical output to Java golden
   }
   ```
4. Confirm the test FAILS before any fix (red-green cycle).

### Naming Convention
- Fixture: `testdata/fixtures/{module}/{config_name}_{region_safe}.jsonl`
- Test function: `parity_{module}_config_{config_name}_{short_region_id}`

### Outputs
- Path to new fixture file
- Path to test file and function name
- Confirmation test fails with expected mismatch

## Phase 4: Fix (mismatch-repair)

### Goal
Fix the Rust module to match Java behavior for the identified config+region.

### Procedure
1. Use the `shard-diagnosis` skill to identify the exact field and root cause within the module.
2. Dispatch Port Engineer with `mismatch-repair` skill:
   - Input: diagnosis report from Phase 2 + shard-diagnosis output
   - Constraint: in-place repair, no adapter patterns
   - Gate: the Phase 3 failing test must pass after the fix
3. Port Engineer implements the fix.
4. Run Phase 3 test — must pass.

### Agent Routing
- **Parity Verifier** runs shard-diagnosis
- **Port Engineer** implements the fix using mismatch-repair
- **Orchestrator** coordinates the handoff

## Phase 5: Verify

### Goal
Confirm the fix resolves the original failure without introducing regressions.

### Procedure
1. Run the Phase 3 test — must PASS.
2. Run the full module sweep test — must PASS (no regression):
   ```bash
   cargo test --profile debug-release --test parity_{module}_sweep -- --include-ignored --nocapture
   ```
3. Re-run the `parity_config_e2e_push_*` test family — must PASS:
   ```bash
   cargo nextest run --cargo-profile debug-release --test parity_config_e2e -E 'test(/parity_config_e2e_push_/)' -j 10
   ```
4. If additional (config, region) pairs still fail, loop back to Phase 2 for the next failure.
5. When all config E2E tests pass → report CONFIG-E2E PASS.

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
- All `parity_config_e2e_push_*` tests pass (PASS verdict), OR
- A fix introduces a regression requiring manual intervention (ESCALATE)

## Related Skills
| Skill | Role |
|-------|------|
| tiered-config-test | Expand nightly/sweep coverage and tier promotion across the 44-config matrix |
| shard-diagnosis | Phase 4: field-level diagnosis within identified module |
| mismatch-repair | Phase 4: fix methodology for Port Engineer |
| module-parity-test | Phase 5: per-module regression check |

## Agent Responsibilities
| Agent | Phases |
|-------|--------|
| Parity Verifier | Phases 1, 2, 5 (run tests, diagnose) |
| Port Engineer | Phases 3, 4 (create test, fix code) |
| Gerneral-Purpose Agent | Ad-hoc tasks (fixture generation, script execution) |
| Orchestrator | Coordinates all phases, decides loop/escalate |