---
name: mismatch-repair
description: "Fix a parity mismatch between VarDictJava and vardict_rs by modifying existing Rust logic in-place. Use when: parity mismatch found, shard failure diagnosed, fix divergent output, repair Rust logic to match Java, align Rust to Java behavior, fix wrong column value, mismatch root cause identified via shard-diagnosis, or config-e2e-diagnosis hands off a reviewed repair plan file. Use this skill after diagnosis has identified the divergent field or failing repair surface. Do NOT use for initial diagnosis (use shard-diagnosis or config-e2e-diagnosis) or config expansion (use tiered-config-test)."
argument-hint: "Describe the mismatch: field name, Java value, Rust value, and position (e.g., 'AF field: Java=0.333 Rust=0.500 at chr20:31024500')"
---

# Mismatch Repair

Fix a parity mismatch by modifying the Rust code that produces the wrong value. This skill picks up where diagnosis leaves off — it takes either a diagnosed mismatch or a reviewed repair plan file and produces a verified, committed fix.

## Why This Exists

Stateless LLM agents, when faced with "Java produces X, Rust produces Y", default to the path of least resistance: add a wrapper function that converts Y→X at the output boundary. This appears to fix the parity test but doesn't fix the actual logic divergence. The next BAM file or config exposes the real bug because the adapter's assumptions don't hold.

This skill exists to redirect that instinct. Instead of patching the output, trace backward to the code that computed the wrong value and fix the logic there.

## Handoff Contract

When `mismatch-repair` is used from the E2E config failure path, the required handoff artifact is the user-accepted repair plan file written under the current CLI session-state artifact path. Read that plan file first, then read the diagnosis and failing-test artifacts it names. For non-E2E shard repairs, a direct shard-diagnosis task brief remains valid.

## The Anti-Adapter Rule

This is the most important constraint in the skill. Read it before touching any code.

**You must not add new functions whose purpose is to convert, adjust, format, or patch a value after it has been computed.** If you find yourself writing a function like `fn fix_af_for_java_compat(v: f64) -> f64` or `fn adapt_output(row: &mut OutputRow)`, you are fixing the wrong thing. The divergence is upstream in the logic that computed the value, not at the output boundary.

**Why this matters:** An adapter that maps wrong→right for one test case encodes assumptions about the failure mode. When a different BAM file triggers a different manifestation of the same logic bug, the adapter either doesn't apply or produces a new wrong answer. The Rust codebase then accumulates layers of shims that make future repairs harder to reason about.

**The escape hatch:** If you genuinely believe an adapter is the correct solution (e.g., Java uses a specific output formatting quirk that Rust handles differently by design), you must:
1. Write a justification comment at the adapter's definition explaining why in-place repair isn't feasible
2. Include the justification in the commit message
3. The adapter must be clearly named as a formatting concern (e.g., `format_af_java_compat`), not a logic fix

## Prerequisite

Before using this skill, you need either a diagnosed mismatch from `shard-diagnosis` or a reviewed repair plan file that points at the diagnosis artifacts to consume:
- The divergent field (column number + field name)
- Java and Rust values
- The responsible Rust module and file
- The genomic position

If you don't have either handoff, run `shard-diagnosis` first or ask the user for the accepted repair plan file.

## Phase 1: Root-Cause Localization

The goal is to produce a **divergence statement** — a one-paragraph description of exactly where and how the Rust code diverges from Java.

### Step 1: Trace backward from the output

Starting from the Rust file identified by `shard-diagnosis`, find the code that computes the divergent field's value. Follow the data flow backward:

1. Find where the field is written to the output (the print/format/serialize call)
2. Find the variable or expression that supplies the value
3. Trace that variable to where it's computed or accumulated

### Step 2: Find the corresponding Java code

Open the Java source for the same module. Look for the equivalent computation. Use traceability comments in the Rust code (e.g., `/// Ported from: CigarParser.java:L142`) to find the right location. If traceability comments are missing, search the Java source for the field name or computation pattern.

### Step 3: Compare the two code paths

Line up the Java logic and Rust logic side by side. Identify the **specific point of divergence**:
- Is a branch missing or inverted?
- Is an accumulator initialized differently?
- Is the iteration order different?
- Is a type conversion losing precision?
- Is a null/None check handled differently?
- Is a collection using HashMap where Java uses LinkedHashMap (iteration order)?

### Step 4: Write the divergence statement

Produce a concise statement like:

> **Divergence:** In `src/cigar_parser.rs:L340-L355`, the `mean_quality` accumulator uses `sum / count` with integer division, but Java's `CigarParser.java:L280` uses `(double)sum / count` which produces a floating-point result. This causes the MeanQual field (col 19) to be truncated.

This statement guides Phase 2. If you can't write a clear divergence statement, you haven't found the root cause yet — keep tracing.

### Plan Freshness Check
If Phase 1 confirms the divergence described in the routed artifacts, continue directly into Phase 2. If Phase 1 uncovers a materially different divergence, expanded repair scope, or evidence that the accepted repair plan file is stale, stop and refresh the repair plan with the user before repair work continues.

## Phase 2: In-Place Repair

### Hard Rules

These are non-negotiable. Violating them means the fix is wrong, even if it makes the test pass.

1. **No new adapter/wrapper/shim functions.** See The Anti-Adapter Rule above.

2. **Modify the existing Rust function** that produces the wrong value. The fix goes where the computation happens, not at the output boundary.

3. **If the Rust code has been refactored to idiomatic style** (iterators instead of for-loops, different module boundaries, combinators instead of mutable state), fix the logic within the refactored idiom. Do not rewrite idiomatic Rust to match Java's structure. The goal is logic equivalence, not structural mimicry. For example:
   - Java: `for (int i = 0; i < list.size(); i++) { sum += list.get(i); }`
   - Rust (refactored): `let sum: f64 = list.iter().sum();`
   - If the Rust version is wrong, fix the iterator chain — don't rewrite it as a for-loop.

4. **One mismatch → one code location.** If your fix touches more than 2 source files (excluding the test file), pause and ask: are you fixing the root cause, or are you patching multiple symptoms? Re-trace from Phase 1 if needed.

### Soft Rules

These are strong preferences, not absolutes:

- **Fix the earliest divergence point.** If field A is wrong and field B depends on A, fix A. Don't try to compensate for A's error in B's computation.
- **Verify accumulation order.** When Java uses mutable state accumulated across a loop (common in CigarParser), verify the Rust equivalent accumulates in the same order with the same initial values.
- **Preserve idiomatic Rust.** Fix the logic bug, not the code style. If the Rust code uses `match` where Java uses `if-else`, keep the `match`.

### Common Fix Patterns

These are the most frequent root causes. Check them first:

| Symptom | Likely Root Cause | Fix |
|---------|------------------|-----|
| Float value slightly off | Integer division instead of float division | Cast numerator or denominator to `f64` before dividing |
| Float formatting differs (0.125 vs 0.1250) | Different decimal formatting | Use `java_format_double()` helper from parity utils |
| Map iteration order differs | `HashMap` where Java uses `LinkedHashMap` | Replace with `IndexMap` |
| Missing variant rows | Branch not implemented or condition inverted | Add the missing branch or fix the condition |
| Extra variant rows | Filter condition missing or inverted | Add or fix the filter |
| Integer value off by one | Overflow wrapping difference | Use `wrapping_add` / `wrapping_mul` for Java-equivalent overflow |
| Null/empty field mismatch | `Option::None` vs default value | Match Java's null semantics — use `Option<T>` and serialize as empty |
| Genotype allele order | Sorting or comparison differs | Match Java's allele ordering logic |

### Apply the Fix

1. Open the Rust file at the divergence point identified in Phase 1
2. Modify the existing logic to match Java's behavior
3. Add a brief comment if the fix isn't obvious: `// Match Java: use float division (CigarParser.java:L280)`
4. Compile: `cargo build --profile debug-release`

## Phase 3: Verify

### Step 1: Write or update the regression test

Create a `#[test]` function that captures this specific mismatch. The test must:
- Use a **Java-generated fixture** as the expected output (never Rust output — that locks in bugs)
- Be named following the convention: `test_{module}_{description}_parity` or `test_target_bam_{bam_slug}_{chr}_{description}_parity`
- **Fail before the fix** (if writing the test first) or be verifiable against the Java fixture

Extract the fixture from the Java shard output:
```bash
# Copy from Java cache (already deterministic)
cp tmp/na12878_parity/<label>/<chr>/java/shard_NNN.tsv testdata/fixtures/<appropriate_dir>/
```

### Step 2: Run tests

```bash
cargo test --profile debug-release -- --include-ignored --skip parity_config_e2e_cell_
```

All tests must pass, including the new regression test.

### Step 2b: Config-e2e failing-test re-run

Use this step when `mismatch-repair` is reached through `config-e2e-diagnosis` Phase 4 and the reviewed repair plan file names a config-E2E sweep or cell-binary failure, not a per-module shard. See `config-e2e-diagnosis` Phase 3 for the fixture-generation procedure; this step is the canonical post-fix execution point for that failing test.

- **Test name pattern:** `parity_{module}_config_{config_slug}_{region_safe}` (for example, `parity_cigar_parser_config_t1_01_1_2324084_2324612`).
- **Fixture path:** `testdata/fixtures/{module}/{module}_{config_slug}_{region_safe}.jsonl.zst`.
- **Loader functions:** use `golden_fixture_path_with_config` and `load_golden_data_with_config` from `tests/common/mod.rs`; derive the config slug through `config_name_to_slug`, not by hand.
- **Harness:** `config-e2e-diagnosis` Phase 3 records the test location in the reviewed repair plan file; add the test to `tests/parity_suite/{module}.rs`, which is mounted by `tests/parity_suite.rs` under the `{module}` module. If the failing test was deliberately added to a different harness, cite that harness file path and its test discovery convention before using an alternative command.
- **Validation command:**
   ```bash
   source "$(conda info --base)/etc/profile.d/conda.sh" && conda activate vdr && export LIBCLANG_PATH="$CONDA_PREFIX/lib"
   cargo test --profile debug-release --test parity_suite \
      {module}::parity_{module}_config_{config_slug}_{region_safe} -- --exact
   ```
- **Pass criteria:** the test exits 0 with no parity diff; continue with the remaining Phase 3 verification steps.
- **Fail criteria:** the test still fails, or it fails on a new field. Treat that as a back-edge, not a forward-edge: loop back to **`mismatch-repair` Phase 1** (Root-Cause Localization) with the new observed divergence.

This step extends Phase 3; it does not replace the regression-test, full-test, shard re-run, or new-mismatch checks below.

### Step 3: Re-run the affected parity shard

Clear stale Rust cache first:
```bash
rm -rf tmp/na12878_parity/<label>/<chr>/rust/
rm -rf tmp/na12878_parity/<label>/<chr>/diff/
```

Then re-run the shard and verify it passes.

### Step 4: Check for new mismatches

The fix must not introduce new mismatches. If the shard now fails on a different field, that's a new bug introduced by the fix — diagnose and repair it before proceeding.

## Phase 4: Commit Gate

Before committing, run lint checks:

```bash
cargo clippy -- -D warnings
cargo fmt --check
```

Then commit the fix, Java-derived fixture, and regression test together in one commit (per ops-policy). The commit message should reference:
- The field that was mismatched
- The root cause (from the divergence statement)
- The config and chromosome where it was found

## Handoff

After the fix is committed:
- If this was the only known mismatch → run `tiered-config-test` smoke preset to verify no regressions
- If more mismatches remain → return to `shard-diagnosis` for the next one
- If a hot-path module was modified (CigarParser, VariationRealigner, SVProcessor, ToVarsBuilder) → optionally run `change-impact-review` to assess performance impact

## Relationship to Other Skills

```
shard-diagnosis --------------------\
                                     -> mismatch-repair -> tiered-config-test
config-e2e-diagnosis + reviewed repair plan file --/
   (find it)          (fix it)           (verify broadly)
```

- **shard-diagnosis**: Upstream for the targeted-fix route. Produces the diagnosed mismatch that this skill consumes.
- **module-parity-test**: Parallel. Used for module-level golden fixture testing during initial porting. This skill is for fixing mismatches after diagnosis, including shard-level parity sweeps and config-E2E repair handoffs.
- **tiered-config-test**: Downstream. Used after repair to verify no regression across configs.
- **CLI user-review checkpoint**: Upstream for config-E2E repair routing. Produces the accepted repair plan file this skill consumes; do not repeat it for simple verification reruns.
- **change-impact-review**: Optional gate for hot-path modules.
