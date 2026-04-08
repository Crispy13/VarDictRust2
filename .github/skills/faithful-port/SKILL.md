---
name: faithful-port
description: "Port a Java module to Rust with byte-identical output parity as the acceptance bar. Use when: porting a module, implementing Java logic in Rust, starting a new porting stage, translating Java to Rust, faithful port, module implementation, picking up an unported stage. Always use this skill when the active plan says to port or implement a module — this is the canonical porting methodology. Do NOT use for fixing parity mismatches (use mismatch-repair) or running parity tests (use module-parity-test)."
argument-hint: "Module name and stage, e.g. 'Configuration (S08)' or 'CigarParser (Layer 4c)'"
---

# Faithful Port

Port a single Java module to Rust so that the Rust code produces byte-identical output to the Java original. This skill covers the full cycle from reading the spec through implementation to the structural review gate — then hands off to testing skills.

## Core Principle

**Faithful translation first. Optimize later.**

Match Java logic line-by-line. No idiomatic Rust improvements, no clever abstractions, no performance optimizations. `clone()` is fine. `Arc<Mutex<>>` is fine. The goal is logic equivalence with the Java source, not beautiful Rust. Byte-identical output is the only acceptance bar.

The rationale: premature idiomatic Rust introduces divergences that are hard to diagnose. A faithful port creates a correct baseline. Optimization happens in a later phase, gated by parity regression tests.

## Module Classification

Before starting, classify the module to pick the right approach:

**Can I test this module with a single function call using known scalar inputs?**

- **YES → TDD-heavy.** Write a Java fixture probe, capture exact outputs, write `#[test]` functions with Java outputs as expected values, then implement to make tests pass.
  - Examples: FisherExact, Utils, Configuration parsing, OutputVariant format methods.

- **NO → Does the module produce a data structure consumed by the next pipeline stage?**
  - **YES → Analysis-heavy.** Read the spec, implement in method clusters (≤500 LOC each), run structural review, then hand off to parity harness.
    - Examples: CigarParser, VariationRealigner, SVProcessor, ToVarsBuilder.
  - **NO → SDD (Spec-Driven Development).** Implement from the module doc, compile gate only. These are data types tested implicitly when pipeline modules consume them.
    - Examples: Variation, Variant, Region, Sclip, ScopeData containers.

## Phase 1: Orient

Read the module's Java documentation from `copilot-office/codebase/java/{Module}.md`. Extract:

1. **Method inventory** — checklist for completeness. Every Java method needs a Rust equivalent.
2. **Parity traps** — the doc explicitly lists these (float formatting, collection ordering, null coercion, etc.).
3. **Cross-module dependencies** — verify all dependencies are already ported. If not, port them first or use placeholder types.

For **large modules (>500 LOC):** Produce a condensed parity brief — control flow, mutable state, collection ordering, null behavior, and formatting — sized to fit alongside the Rust file in context.

For **data type modules:** Note which fields must be `pub` (faithful POJO port), which need custom `PartialEq`/`Hash` (Java `equals` often uses a subset of fields), and which reference types not yet ported (use forward declarations or placeholders).

## Phase 2: Implement

### TDD-Heavy Path (Leaf Modules)

1. **Write a Java fixture probe.** A small Java program that calls the target method with controlled inputs and prints exact outputs. Run it once, capture the output.
2. **Write Rust tests first.** Create `#[test]` functions with the captured Java outputs as expected values. Tests will fail — this is intentional.
3. **Implement to pass.** Write Rust code that makes all tests pass. Follow the faithful translation rules below.

### SDD Path (Data Types)

1. **Implement from the module doc.** Translate Java fields, constructors, and methods to Rust structs and impl blocks.
2. **Compile gate only.** `cargo build --profile debug-release` must pass. These types are tested implicitly by their consumers.

### Analysis-Heavy Path (Pipeline Modules)

1. **Decompose into method clusters.** For modules >1,000 LOC, identify 3-5 logical groups of methods (e.g., CigarParser: initialization, CIGAR walking, quality accumulation, variant detection, cleanup). Port one cluster at a time.
2. **Implement each cluster.** Follow the faithful translation rules. After each cluster, run `cargo build --profile debug-release` to catch compile errors early.
3. **After all clusters compile:** proceed to Phase 3 structural review.

### Faithful Translation Rules

These are non-negotiable during Phase 1 porting:

1. **Match Java logic line-by-line.** Every Java branch, every null check, every accumulator, every loop. Do not refactor, simplify, or merge branches.

2. **Traceability comments on every non-trivial method:**
   ```rust
   /// Ported from: CigarParser.parseCigar()
   /// Java source: CigarParser.java:L142-L380
   fn parse_cigar(...) { ... }
   ```

3. **`clone()` is acceptable.** Don't fight the borrow checker. Match Java's ownership model (everything is heap-allocated in Java). Optimize ownership later.

4. **Do not add abstractions.** No trait hierarchies, no builder patterns, no newtype wrappers unless the Java original has the equivalent pattern.

5. **Port in method clusters for large modules.** Each cluster ≤500 LOC. Compile after each cluster.

6. **All fields `pub` for POJOs.** Java POJOs have public fields or getters. Use `pub` fields in Rust — add encapsulation later.

### Type Mapping Reference

These are parity-critical. Using the wrong type causes mismatches that are hard to trace.

| Java Type | Rust Type | Why It Matters |
|-----------|-----------|----------------|
| `int` | `i32` | Match signed 32-bit semantics |
| `long` | `i64` | |
| `double` | `f64` | |
| `char` | `u8` | Genomic data is ASCII |
| `String` | `String` | Not `&str` — match Java heap ownership |
| `null` | `Option::None` | Every Java null check → `Option` match |
| `Integer` / boxed | `Option<i32>` | Boxed primitives are nullable |
| `HashMap<K,V>` | `HashMap<K,V>` | **Only** when iteration order never affects output |
| `LinkedHashMap<K,V>` | `IndexMap<K,V>` | **Critical** — preserves insertion order. #1 parity failure source. |
| `TreeMap<K,V>` | `BTreeMap<K,V>` | Sorted key order |
| `ArrayList<T>` | `Vec<T>` | |
| `StringBuilder` | `String` + `push_str` | |

### Float Formatting — The #1 Parity Trap

Java's `DecimalFormat` uses **HALF_UP** rounding. Rust's `format!("{:.4}", v)` uses **HALF_EVEN** (banker's rounding). These produce different results at midpoints:

- `0.0015` with 3 decimals: Java → `"0.002"`, Rust → `"0.001"`

**Use the project's custom `round_half_even` / `java_format_double` helper** (in `src/utils.rs`) that matches Java's DecimalFormat behavior. Never use raw `format!` for parity-critical floats.

### Integer Overflow

Java silently wraps on integer overflow. Rust panics in debug mode. For arithmetic that overflows in Java:
```rust
let result = a.wrapping_add(b);  // Matches Java silent overflow
```

## Phase 3: Structural Review

Run this checklist **before** handing off to testing. It catches ~30% of parity issues without running the expensive harness.

- [ ] Every Java method from the module doc has a corresponding Rust function
- [ ] Every Java branch has a corresponding Rust branch (no `todo!()` or `unimplemented!()` stubs in output-affecting paths)
- [ ] All output-affecting maps use `IndexMap` where Java uses `LinkedHashMap`
- [ ] Float formatting uses the project's HALF_UP helper, not raw `format!`
- [ ] Null/Option handling matches Java null-check patterns
- [ ] Collection iteration order matches Java for all output-producing paths
- [ ] All POJO fields are `pub`
- [ ] Traceability comments present on all non-trivial methods

**If any check fails, fix it before proceeding to testing.** Don't waste time on the parity harness when a structural issue is visible.

Then compile and lint:
```bash
cargo build --profile debug-release
cargo clippy -- -D warnings
cargo fmt --check
```

## Phase 4: Hand Off to Testing

Based on module type:

- **TDD-heavy (leaf modules):** Run `cargo test --profile debug-release -- --include-ignored`. All fixture tests must pass.
- **SDD (data types):** Compile gate passed in Phase 3. Done — these are tested implicitly by consumers.
- **Analysis-heavy (pipeline modules):** Hand off to the `module-parity-test` skill for JSONL golden fixture comparison, or to the shard parity harness for TSV output comparison.

## Phase 5: Fix Failures

If tests or parity checks fail:

1. Use `shard-diagnosis` skill to identify the first divergent field.
2. Use `mismatch-repair` skill to fix the Rust logic in-place.
3. Re-run tests.
4. Repeat until all tests pass.

**Do not skip mismatch-repair and try to fix inline.** The anti-adapter rules in that skill prevent the agent from adding shim functions instead of fixing root causes.

## Phase 6: Commit + Promote

1. Run lint: `cargo clippy -- -D warnings && cargo fmt --check`
2. Commit the implementation, any Java-derived fixtures, and regression tests together in one commit.
3. Update the active plan marking the module/stage as complete.
4. Advance to the next module in the layer.

**Rule: No module advances past its gate until the gate passes 100%.**

## Relationship to Other Skills

```
active-plan  ──→  faithful-port  ──→  module-parity-test / cargo test
(what to port)    (how to port)       (verify output)
                                            ↓ (if failures)
                                      shard-diagnosis  ──→  mismatch-repair
                                      (find divergence)     (fix in-place)
                                                                  ↓
                                                          tiered-config-test
                                                          (verify broadly)
```

- **Active plan:** Upstream. Tells the agent which module to port. This skill tells the agent how.
- **module-parity-test:** Downstream. Verifies pipeline module output against Java golden fixtures.
- **mismatch-repair:** Downstream. Fixes parity mismatches found during testing.
- **shard-diagnosis:** Downstream. Diagnoses specific shard failures.
- **tiered-config-test:** Further downstream. Expands config coverage after fixes.
- **codebase-doc-manage:** Parallel. Manages the Java module documentation that Phase 1 reads.
