# TODO

Deferred work items in the VarDict-rs port that must be resolved before full production parity. Each item includes the blocker, impact, and unblock path so future work can pick it up without re-discovery.

---
# Planned
## Optimize memory usage for pileup

## Do we really need dynamic dispatch for line sinks?
```rust
pub type VariantLineSink = dyn Fn(&str) + Send + Sync + 'static;
pub type VariantOwnedLineSink = dyn Fn(String) + Send + Sync + 'static;
```
we can use enum or just one function.

## Sweep gate: stale golden-cache provenance → cache reports `not-ready`

**Context:** `scripts/e2e_sweep_gate.py` `run_provenance_check` was reordered so the
cheap metadata checks run before the expensive `zstd -dc | md5` content check, which
runs only when a cell is otherwise ready (commit on `dev/claude`). That removed the
~13-minute wasted decompress (CM-PILEUP's 36 GB golden cache on `/hdd-disk1`) seen in
the `na12878_lowcov` run — but it only makes the gate **fail fast**, not pass.

**Real issue:** the `na12878_lowcov` golden cache is 700/700 `not-ready`
(`mismatch_generator_flags=615` + `incompatible_backfilled_chunks=85`). The recorded
`generator_flags` are `['--fisher']` instead of the expected
`--output-only --config <PRESET> --tags <tag> --sweep-bed-root <root>`, and several
backfilled sidecars lack `generator_flags`/`preset`/`bed_sha256`. So the gate does not
trust this cache as a canonical result even though the parity tests themselves pass.

**Unblock path (pick one):**
1. Regenerate/backfill the `chunks.json` sidecars so `generator_flags`/`preset`/
   `bed_sha256` match the gate's expectation (see `scripts/backfill_chunks_json.py`
   and `scripts/gen_e2e_sweep_golden.sh`). Makes the cache genuinely `ready`.
2. Or confirm the mismatch is purely cosmetic (TSV *content* is correct; only the
   recorded provenance string is incomplete) and normalize/relax that specific check
   in `provenance_metadata_warning` / `missing_backfilled_provenance_keys`.

**Then (only after the cache is `ready` again):** the legitimate content md5 returns
and CM-PILEUP's 36 GB will be decompressed for real — move the sweep-fixture cache
off the 86%-full HDD `/hdd-disk1` onto SSD to keep that read fast. (The same large
reads also slow the `tests` phase, where CM-PILEUP went idle 600s/1200s.)

**Detection:** rerun the provenance phase of the sweep gate on `na12878_lowcov`;
non-empty `warning_summary` / `readiness=not-ready` in `parity-failure-report.json`
flags it.


# Deferred
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
