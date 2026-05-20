# TODO

Deferred work items in the VarDict-rs port that must be resolved before full production parity. Each item includes the blocker, impact, and unblock path so future work can pick it up without re-discovery.

---
# Planned
## Minimum stale Java cache guard follow-ups

**Status:** Minimum gate guard implemented; broader hardening deferred.
**Location:** [scripts/e2e_sweep_gate.py](scripts/e2e_sweep_gate.py), [scripts/sweep_fixtures_parallel.py](scripts/sweep_fixtures_parallel.py), [scripts/backfill_chunks_json.py](scripts/backfill_chunks_json.py)

The minimum workflow-managed stale-cache fix validates staged Java TSV content against paired `*.chunks.json` `monolithic_md5` fingerprints and prevents expected non-empty `--output-only` refreshes from succeeding as no-ops. Backfilled sidecars without gate provenance are cache-readiness failures, not Rust repair evidence.

Deferred hardening remains out of the first slice: Rust harness-side content validation, CI YAML changes, a broad no-mutation audit framework, and a stronger new content hash field. Revisit those only if gate-time validation proves insufficient for the active workflow.

## Agents blocks on terminal commands, but Human can see the progress.

**Status:** Set for the current E2E parity sweep.
**Location:** [scripts/e2e_sweep_gate.py](scripts/e2e_sweep_gate.py) and dated report directories under [tmp/parity-iteration](tmp/parity-iteration)

The current wrapper-driven E2E parity sweep already writes human-readable progress and log artifacts under the selected `--report-dir` before and during long runs. For active or recent runs, humans can monitor:

- `progress.log` for matrix, staging, pair progress, idle diagnostics, final pass/fail, and report paths.
- `child-logs/*.stdout.log` and `child-logs/*.stderr.log` for mirrored cargo child output.
- `heartbeats/*.log` for Rust harness phase markers and side-channel runtime telemetry.
- `cell-runtimes.jsonl` when test execution reaches runtime aggregation.
- `parity-failure-report.json` or `last-pass.json` for terminal gate artifacts after completion.

Operating rule: launch long wrapper-driven E2E parity sweep commands in sync/blocking terminal mode after confirming `--test-threads`, record the chosen `--report-dir`, and rely on the report directory files above for human visibility instead of periodic agent polling. Monitoring must not silently narrow the canonical full-scope gate.


# Deferred
## 0.5 Complete CM-* call-mode test expansion

**Status:** 🟡 Deferred during multithreading Stage 1.5
**Location:** [tests/parity_config_e2e.rs](tests/parity_config_e2e.rs#L411) + missing golden fixtures under [tmp/e2e_fixtures](tmp/e2e_fixtures)
**Introduced:** 2026-04-23, commit `3d738bb test-infra(B2,B4): swap 6 threshold-redundant presets for call-mode CM-*`

### Context

Commit `3d738bb` swapped 6 threshold-redundant T2/T3 presets for call-mode `CM-*` presets (`CM-FISHER`, `CM-PILEUP`, `CM-NOSV`, `CM-NOREAL`, `CM-CHIMERIC`, `CM-MAPQ30`) in `scripts/config_presets.tsv` and `tests/common/mod.rs::CONFIG_PRESETS`, but left the downstream test-matrix expansion incomplete.

Commits `ab0e9a7` + `f3e55b6` (Stage 1.5 of the multithreading port) finished the helper wiring — test function identifiers, `parse_java_flags` accepting bare flags, and `config_preset()` match arms for each CM-* → correct `Configuration` field mutation.

### Remaining Gaps

1. **`config_preset_alignment` assertion (L411 of `tests/parity_config_e2e.rs`)** — still asserts CM-* presets leave `fisher`, `do_pileup`, `disable_sv`, `chimeric`, `perform_local_realignment`, `mapping_quality` at defaults. It must be updated to assert the expected CM-* field values per the TSV flag mapping.
2. **Golden JSONL fixtures** for 6 CM-* presets do not exist under `tmp/e2e_fixtures/`. The 6 CM-* push tests in `tests/parity_config_e2e.rs` compile and dispatch correctly but bail out on missing golden files before any byte comparison. No real parity failure observed.

### Unblock Path

1. Rewrite `config_preset_alignment` to assert the field values each CM-* preset is expected to set (per TSV column 2).
2. Run Java VarDictJava E2E sweep with each CM-* flag set against the E2E fixture regions to generate the 6 missing golden JSONLs.
3. Commit goldens under `tmp/e2e_fixtures/CM-{FISHER,PILEUP,NOSV,NOREAL,CHIMERIC,MAPQ30}/`.
4. Run `cargo test --profile debug-release --test parity_config_e2e -- --test-threads=1` → expect 46/46 pass.

### Blast Radius

Does not block the multithreading port. Does not affect Binary B. Only relevant to Binary A (`parity_config_e2e`) 6 push tests + 1 alignment test. Determinism tests (Stages 3–4 of multithreading port) use `-B 4` (= T1-14), unaffected.

---

## 0. Add `CM-TH4` preset to `scripts/config_presets.tsv`

**Status:** 🟡 Blocked on multithreading port (handoff-multithreading.md)
**Location:** [scripts/config_presets.tsv](scripts/config_presets.tsv)
**Java counterpart:** `-th 4` command-line flag, exercised in Java parity sweeps.
**Introduced:** 2026-04-23, during multithreading port planning.

### Current Rust Behavior

`-th` is accepted in [src/bin/vardict_rs.rs](src/bin/vardict_rs.rs) but always routes to `not_parallel()` regardless of value. Adding a CM-TH4 preset now would silently pass through the same code path as `default`, giving false coverage.

### Unblock Path

1. Complete the multithreading port (Stages 1–5 of plan in [docs/handoff-multithreading.md](docs/handoff-multithreading.md)).
2. Add `CM-TH4\t-th 4\tFour-thread parallel execution.` row to `scripts/config_presets.tsv`.
3. Regenerate goldens for the new preset via `scripts/gen_e2e_golden_tsv.sh --push-only --all-configs`.
4. Confirm Binary B 4400 → 4840 cells and baseline stays at zero failures.

### Detection

If added before the port is complete, every CM-TH4 cell will be byte-identical to default — parity passes but coverage is fake. The handoff doc (§5) calls this out explicitly.

**Multithreading port status (2026-04-23):** Stages 1–5 complete (commits `9248efc` → `a4c7916`). SimpleMode + SomaticMode `parallel()` are both byte-identical to `not_parallel()` across 16 determinism test cases including `-B 4` (TLS-scope guard). The port is functionally complete; this item is now an optional *validation* step rather than a blocker.

---

## 0.7 Multithreading Stage 6: full sweep validation + CM-TH4 preset

**Status:** 🟢 Optional validation step (multithreading port complete)
**Location:** [scripts/config_presets.tsv](scripts/config_presets.tsv) + [scripts/gen_e2e_golden_tsv.sh](scripts/gen_e2e_golden_tsv.sh)
**Introduced:** 2026-04-23, handoff-multithreading.md §§ Stage 6

### Unblock Path

1. Run Binary A + Binary B full sweep with `-th 1,4,8` against all 44 configs × all regions. Confirm byte-identical across thread counts.
2. Add `CM-TH4\t-th 4\tFour-thread parallel execution\t2\tboth` row to `scripts/config_presets.tsv`.
3. Regenerate goldens via `scripts/gen_e2e_golden_tsv.sh --push-only --all-configs`.
4. Confirm Binary B 4400 → 4840 cells, baseline stays at zero failures.

### Why Not Blocking

The 16 determinism test cases (`tests/parity_parallel_determinism.rs`) already gate nondeterminism across both modes including the `-B 4` TLS-scope regression guard. Stage 6 would increase config × region breadth but is not required for correctness.

---

## 0.8 SomaticMode CLI dispatch in `src/bin/vardict_rs.rs`

**Status:** ⛔ Blocked on M5 / combine_analysis (TODO #1)
**Location:** [src/bin/vardict_rs.rs](src/bin/vardict_rs.rs)
**Introduced:** 2026-04-23, Stage 5 of multithreading port

### Current Rust Behavior

`src/bin/vardict_rs.rs` constructs only `SimpleMode` regardless of how many BAM paths the CLI receives. `-th` flag works correctly but only dispatches through SimpleMode's `parallel()`. SomaticMode's `parallel()` impl (Stage 5, commit `a4c7916`) is exercised only by `tests/parity_parallel_determinism.rs`.

### Unblock Path

1. Un-stub `combine_analysis` (TODO #1). Without it, somatic CLI output would lack rescue/downgrade labels.
2. Wire mode selection in `src/bin/vardict_rs.rs`: single BAM path → SimpleMode; two BAM paths separated by `|` → SomaticMode; amplicon designations → AmpliconMode.
3. Match Java `main()`'s mode-selection logic from `VarDictLauncher.java` / `com.astrazeneca.vardict.Main`.

### Why It Is Blocked

Same dependency chain as TODO #1. Wiring the CLI before `combine_analysis` lands would produce output missing somatic rescue labels — Binary B would fail on every somatic cell.

---

## 0.9 AmpliconMode + SplicingMode `parallel()` implementations

**Status:** 🟡 Optional; deferred from multithreading handoff
**Location:** [src/modes.rs](src/modes.rs) — `impl ParallelMode for AmpliconMode`, `SplicingMode`
**Introduced:** 2026-04-23, multithreading port

### Current State

After Stage 4.5 (`8d7d150`), `ParallelMode` is a trait with default implementations of `parallel()` + `not_parallel()`. AmpliconMode and SplicingMode compile with stub `regions()` returning `&[]` and `process_region_to_buffer()` that panics. The crossbeam/rayon topology is already inherited from the trait default — these modes just need their per-region work wired in.

### Unblock Path (AmpliconMode)

1. Read Java `AmpliconMode.createParallelMode()` + per-region task body to determine whether amplicon post-merge runs inside the per-region task (safe, no extra work) or across regions after all tasks complete (needs `post_parallel_hook()` override).
2. Port the per-region amplicon pipeline into `process_region_to_buffer()`.
3. Optional: override `post_parallel_hook()` for cross-region merge.
4. Extend `tests/parity_parallel_determinism.rs` with amplicon fixtures.

### Unblock Path (SplicingMode)

1. Similar to AmpliconMode but with splicing-specific pipeline.
2. Verify the splicing variant-call path is fully ported in Rust (may hit pre-existing porting gaps in splicing logic itself, not just parallel).

### Why Not Blocking

Neither mode is in the original multithreading handoff scope. AmpliconMode + SplicingMode users can use `-th 1` (serial) path which routes to `not_parallel()` default. Parity is unaffected.

---

## 1. `somatic_post_process::combine_analysis` is stubbed

**Status:** ⛔ Blocked on M5 (Mode orchestrator re-entrancy)
**Location:** [src/mods/somatic_post_process.rs](src/mods/somatic_post_process.rs) — `combine_analysis()` function
**Java counterpart:** `VarDictJava/src/main/java/com/astrazeneca/vardict/postprocessmodules/SomaticPostProcessModule.java` — `combineAnalysis()` (L385–L476)
**Introduced:** 2026-04-17, commit `cc30fd9` (TSV output layer full port)

### Current Rust Behavior

Returns a placeholder:
```rust
CombineAnalysisData {
    max_read_length,
    type_: String::new(),
}
```

The empty `type_` flows through as "no rescue / downgrade happened", so every candidate retains its default somatic label.

### What Java Actually Does

At SomaticPostProcessModule.java:L404:
```java
AlignedVarsData tpl = getMode().pipeline(currentScope, new DirectThreadExecutor()).join().data;
```

`combineAnalysis()`:
1. Expands a rescue `Region` around `variant1.startPosition - maxReadLength` through `variant1.endPosition + maxReadLength`.
2. Fetches reference bases for the expanded region via `ReferenceResource.getReference(region)`.
3. Builds a new `Scope<InitialData>` labeled `"bam1:bam2"` (combined-BAM logical source).
4. **Re-runs the full pipeline** (SAMFileParser → CigarParser → Realigner → SVProcessor → ToVarsBuilder → postprocess) synchronously on that combined BAM via `getMode().pipeline(currentScope, new DirectThreadExecutor()).join()`.
5. Looks up the same variant description string in the combined callset.
6. Decision tree:
   - Combined callset lacks the variant → return `"FALSE"` (suppress the somatic row entirely).
   - Combined support exceeds original sample by ≥ `minr` reads → subtract `variant1` counts from combined result into `variant2`, clamp to zero, recompute weighted means, force `isAtLeastAt2Positions = true`, `hasAtLeast2DiffQualities = true`, return a non-empty label that overrides `StrongSomatic` with (typically) `"Germline"`, `"LikelySomatic"`, or similar.

### Why It Is Blocked

Three dependencies do not yet exist in the Rust workspace:

1. **No re-entrant `Mode::pipeline(scope, executor)` function.** [src/modes.rs](src/modes.rs) currently implements only sequential region iteration and header printing. There is no single composed pipeline entry point that chains the six pipeline modules (`cigar_modifier` → `sam_file_parser` → `cigar_parser` → `variation_realigner` → `structural_variants_processor` → `to_vars_builder`) and returns a `Scope<AlignedVarsData>`.

2. **No `DirectThreadExecutor` equivalent.** Java uses `DirectThreadExecutor` to run the pipeline synchronously on the calling thread (not the shared executor pool). Rust has no equivalent yet because the Rust side does not have an executor abstraction — the pipeline modules are called directly and synchronously. Structural shim only; no real blocker here, but the abstraction has to exist before `combine_analysis` can call it.

3. **No combined-BAM logical source abstraction.** Java passes `"bam1:bam2"` as the BAM label and SAMFileParser interprets the colon as "read from both sources in sequence". The Rust `SAMFileParser` may or may not honor this convention — needs verification. If it does not, a shim is needed.

### Impact

- **Paired-sample (Somatic) mode only.** Single-sample (Simple) and Amplicon modes do not call `combineAnalysis()`.
- Within Somatic mode, candidates that should be **suppressed** (combined callset lacks the variant → `"FALSE"`) are instead **emitted** with their default label (`StrongSomatic` or `StrongLOH`).
- Candidates that should be **relabeled** (combined support high → `Germline` / `LikelySomatic`) retain `StrongSomatic` / `StrongLOH`.
- Net effect on downstream: false-positive inflation of strong-somatic calls for low-support indels in paired runs. Non-indel SNVs unaffected (the Java branch guards on `nt.matches(...)` for non-SNV long/regex-matched indels only).

### Unblock Path (estimated M5 scope)

1. Port `Mode.pipeline(Scope<InitialData>, Executor)` as a Rust function in `src/modes.rs`. It should compose the six pipeline modules and return `Scope<AlignedVarsData>`.
2. Add a `DirectThreadExecutor` stand-in (likely a trait with a `.run(closure)` method and a synchronous implementation).
3. Verify or implement combined-BAM handling in `SAMFileParser` for the `"bam1:bam2"` label convention.
4. Port `combine_analysis` body:
   - Reference fetch via `ReferenceResource`.
   - Build rescue `Scope<InitialData>`.
   - Call `Mode::pipeline(rescue_scope, DirectThreadExecutor)`.
   - Variant lookup + decision tree exactly as Java L425–L476.
   - Field-by-field subtraction and weighted-mean recomputation.
5. Add somatic parity tests that exercise low-support indels to validate rescue behavior.

### Detection

A paired-BAM parity test comparing Rust somatic output to Java somatic output on any region with low-coverage indels will flag this immediately: Rust will emit extra `StrongSomatic` rows that Java suppresses or relabels. Until that test exists, the gap is silent.

---

## Conventions

- Add new items as numbered sections at the top (newest first) or in the appropriate category.
- Each item must record: **status / location / Java counterpart / current Rust behavior / why blocked / impact / unblock path / detection**.
- Remove items only when the underlying gap is closed and verified by a parity test.
