# TODO

Deferred work items in the VarDict-rs port that must be resolved before full production parity. Each item includes the blocker, impact, and unblock path so future work can pick it up without re-discovery.

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
