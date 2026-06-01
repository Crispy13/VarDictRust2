---
name: logic-parity-audit
description: "Run a white-box logic parity audit for a ported Rust module after Tier 1 parity passes or after a proven Rust mismatch-repair has passed focused/module verification and needs diff-scoped audit before performance review. Use this whenever the user asks for a logic check, code comparison, verify port, inspect ported code, compare Java and Rust, function-by-function review, method comparison, audit port quality, white-box review, structural parity audit, review translated logic, or post-repair audit of a touched Rust module/surface. This skill works for any ported module and is not a substitute for shard-diagnosis or mismatch-repair."
---

# Logic Parity Audit

Use this skill to inspect any ported module side by side against its Java source and produce an audit report that is useful to a reviewer, not just a pass or fail label. The audit reads the actual Java `.java` source files directly — not just documentation summaries — to compare logic, branches, null handling, and formatting against the Rust translation. The point is to catch structural mismatches that black-box parity tests may not exercise on the current inputs.

## Purpose

Tier 1 parity testing tells you that sampled outputs match. It does not prove that every method, branch, ordering choice, or null path was ported faithfully. This skill is the white-box complement to that black-box check.

It is especially useful when a module already passes the 100-region parity gate but still feels risky because:

- the Java class is large and stateful
- helper methods are easy to skip during translation
- collection choices can look harmless until a rare ordering-sensitive case appears
- a formatting or null-handling divergence may be latent rather than currently observable

The audit is deliberately narrow: it compares structure and parity-sensitive mechanics, then writes an ephemeral report that helps decide whether the module is ready for broader Tier 2 sweep coverage.

## Position In The Workflow

This skill extends the structural review from faithful-port rather than replacing it.

```text
faithful-port Phase 3 review
        ↓
module-parity-test Tier 1 (100 regions)
        ↓
logic-parity-audit
        ↓
tiered-config-test / Tier 2 sweep
```

It is also used as a post-repair checkpoint after a proven Rust config-E2E repair:

```text
config-e2e-diagnosis / shard-diagnosis
        ↓
mismatch-repair
        ↓
focused/module verification PASS
        ↓
read repair diff and select touched Rust module or logic surface
        ↓
logic-parity-audit
        ↓
change-impact-review
```

Why here:

- Tier 1 gives evidence that the module works on representative inputs.
- This audit asks whether the code still hides untested omissions or mechanical mismatches.
- Tier 2 is broader and more expensive, so it is worth catching obvious white-box issues before scaling up.

## Supported Scope

Use this skill for any ported module where you want systematic verification that the Rust translation faithfully mirrors the Java source. The audit is most valuable for larger, stateful modules with many methods, but it works equally well for smaller utilities — a shorter module simply produces a shorter report.

For post-repair audits, scope the audit to the touched Rust module or repaired logic surface selected from the repair diff. If the diff is broad or ambiguous, audit the full touched module. If the audit returns NEEDS_REVIEW, repair the findings before running `change-impact-review`.

Do not use this skill to diagnose a failing shard or to implement a mismatch repair. Those cases remain the responsibility of `shard-diagnosis`, `config-e2e-diagnosis`, and `mismatch-repair`. The post-repair use case starts only after the repair exists and focused/module verification has passed.

## Prerequisites

Before starting, make sure the audit has enough context to be meaningful:

1. Tier 1 parity already passed for the target module, or a proven Rust `mismatch-repair` has passed focused/module verification and the repair diff is available for scope selection. If the user explicitly asks for a pre-pass audit, record that override.
2. The Java source file exists in VarDictJava/src/main/java/... and is accessible for direct reading.
3. The Java module doc at copilot-office/codebase/java/{Module}.md is available as supplementary context (method inventory, parity warnings, algorithm notes).
4. The Rust implementation is present in src/ and is stable enough to inspect.
5. The parity rules are available in .github/instructions/rust-parity.instructions.md for type mapping, ordering, null handling, formatting, and traceability checks.

If any prerequisite is missing, do not invent confidence. Record the gap in the report and narrow the audit scope accordingly.

## Procedure

### Phase 1: Load Context

Read the actual source files for both languages:

1. The Java source file in VarDictJava/src/main/java/... — this is the primary comparison reference
2. The corresponding Rust source file or files under src/
3. The Java module doc in copilot-office/codebase/java/{Module}.md — use as supplementary context for method inventory, parity warnings, and algorithm notes
4. The parity rules in .github/instructions/rust-parity.instructions.md when a finding touches types, ordering, nulls, float formatting, or traceability
5. The recent Tier 1 or focused/module verification result, if available, to understand whether the audit is confirming a pass or explaining residual risk

Capture a quick working note with:

- Java source file path and class or classes in scope
- Rust struct, impl, or module in scope
- method count from the Java source
- any obvious parity traps called out by the Java module doc

Why reading the actual Java source matters: documentation is a curated summary that can miss details, rename methods, or omit edge-case branches. The `.java` file is the ground truth. The module doc supplements this with pre-analyzed parity warnings and algorithm notes that accelerate the review.

### Phase 2: Build The Method Inventory

Create a side-by-side inventory before judging individual methods.

For each Java method in the source file:

1. Find the Rust counterpart.
2. Record the pairing, or mark it missing.
3. Note whether the method is output-affecting, helper-only, or uncertain.
4. Cross-reference with the module doc's method inventory if available for additional context.

Use this temporary working shape:

| Java Method | Rust Counterpart | Role | Notes |
|---|---|---|---|
| parseCigar() | parse_cigar() | output-affecting | core walk |

Why do this first: completeness defects are easy to miss during free-form review, especially when method names were renamed during translation. A formal inventory forces the audit to answer the basic question of whether the port covers the whole class.

### Phase 3: Compare Each Method

For each method pair, inspect the parity-sensitive parts that are most likely to hide structural divergence.

Record findings for these dimensions:

1. Logic equivalence
2. Null to Option handling
3. Float formatting
4. Traceability comment
5. Status verdict

What each dimension means:

- Logic equivalence: compare branches, loop structure, early returns, accumulator updates, collection choice in output-affecting paths, and any ordering-sensitive iteration.
- Null to Option handling: check whether Java null paths became explicit Option paths at the boundary, and whether None propagates the same way Java null did.
- Float formatting: check whether Java-style formatted output uses the shared parity helper instead of raw Rust formatting expressions.
- Traceability comment: confirm that non-trivial Rust methods point back to the Java source so future mismatch work can be anchored quickly.

Use the parity rules as the tie-breaker when you inspect type and collection decisions. In particular, treat LinkedHashMap to HashMap substitutions, raw format! for parity-critical numeric text, and silent null coercions as audit findings even if the current sample inputs passed.

Use Codex rubber-duck mode to review your result.

### Phase 4: Apply Conservative Auto-Fixes

This phase exists to remove obvious mechanical noise from the report, not to rewrite logic. Only auto-fix when the Java intent is clear from local context and the patch is small, mechanical, and low-risk.

Split the allowed patterns into two risk categories and validate them differently.

Non-behavioral auto-fixes:

- Missing traceability comment on a non-trivial ported method: add a Ported from line with the Java file and line span
- Missing pub on a POJO-style field that should mirror the Java data model: add pub

Behavioral auto-fixes:

- HashMap where Java clearly uses LinkedHashMap: change to IndexMap
- Raw format! or equivalent Rust float formatting in a parity-critical output path: switch to the shared Java-compatible formatter

Why the allow list is short: structural review is valuable precisely because it stays honest about uncertainty. Once the change moves beyond a local mechanical correction, the audit should stop pretending it can repair the issue safely.

After any auto-fix, validate the workspace before the report is handed off.

- Non-behavioral fixes: run `cargo build --profile debug-release`
- Behavioral fixes: run `cargo test --profile debug-release --test parity_suite {module}:: -- --include-ignored` inside Phase 4 after applying the fix

Behavioral fixes require one extra guardrail: if the module's Tier 1 parity test regresses after the mechanical change, revert that auto-fix immediately and document the finding as `NEEDS_REVIEW` with a note that the mechanical fix caused a regression.

### Phase 5: Generate The Report

Write the final report to tmp/logic-parity-audit/ using a module-and-date-based filename such as tmp/logic-parity-audit/CigarParser-2026-04-10.md.

Keep reports ephemeral:

- save under tmp/ so they follow project ops policy
- do not commit them unless the user explicitly asks
- treat them as review artifacts, not permanent source files

The report needs two levels of detail:

- a class-level summary so a reviewer can decide whether the module is ready to advance
- a method-level table so the next person can act on specific findings without redoing the audit

If any auto-fixes were applied in Phase 4, include the Auto-Fix Manifest section in the report. List every auto-fix with its before/after diff and parity re-run result.

## Report Format

Use this template exactly enough that another agent or reviewer can skim it quickly:

```markdown
# Logic Parity Audit: {Module}

**Date:** {YYYY-MM-DD}
**Tier 1 status:** PASS | NOT VERIFIED | USER OVERRIDE
**Java source:** VarDictJava/src/main/java/.../{Module}.java
**Java doc:** copilot-office/codebase/java/{Module}.md
**Rust source:** {rust source path}
**Report path:** tmp/logic-parity-audit/{Module}-{date}.md

## Summary
- Total methods: {N}
- Verified: {N}
- Needs review: {N}
- Failed: {N}
- Auto-fixed: {N}

## Class Summary
| Java Class | Rust Struct or Impl | Method Inventory | Mechanical Fixes | Status |
|---|---|---|---|---|
| {JavaClass} | {RustType} | {complete / partial / missing methods} | {N or none} | {VERIFIED / NEEDS_REVIEW / FAILED} |

## Method Detail
| Java Method | Rust Function | Logic Equivalence | Null→Option Handling | Float Formatting | Traceability Comment | Status |
|---|---|---|---|---|---|---|
| parseCigar() | parse_cigar() | ✅ matches branch structure | ✅ | N/A | ✅ | VERIFIED |
| getVariant() | get_variant() | ⚠ else branch differs | ✅ | ✅ | ❌ missing | NEEDS_REVIEW |

## Issues Found
### Issue 1: {short title}
- Java reference: {file and line or doc section}
- Rust reference: {file and line}
- Why it matters: {impact on parity or review confidence}
- Severity: {HIGH / MEDIUM / LOW}
- Auto-fixed: {Yes / No}
- Action: {patched mechanically | flagged for manual review}

## Auto-Fix Manifest
| # | Pattern | File | Lines | Old | New | Parity Re-Run |
|---|---------|------|-------|-----|-----|---------------|
| 1 | IndexMap swap | src/foo.rs | L42 | `HashMap<String, Vec<...>>` | `IndexMap<String, Vec<...>>` | PASS |
| 2 | Traceability comment | src/foo.rs | L10 | (none) | `// Ported from Foo.java:15-30` | N/A |

## Outcome
- Ready for Tier 2 sweep: {Yes / No}
- Blocking items: {none or short list}
- Suggested next skill: {tiered-config-test | faithful-port follow-up | manual review}
```

Each manifest entry includes the pattern type, file path, line range, old text, new text, and parity re-run result. Use `PASS` for behavioral fixes and `N/A` for non-behavioral fixes.

Why this format works:

- The summary answers the gate question quickly.
- The class table keeps completeness and status visible at module scale.
- The method table makes the audit actionable instead of hand-wavy.
- The issue section preserves reasoning so later fixes do not depend on memory.

## Auto-Fix Scope

Use the following boundaries explicitly in the report.

### Allow List

| Pattern | Why it is safe enough |
|---|---|
| HashMap to IndexMap when Java uses LinkedHashMap | This is a direct data-structure parity correction with clear intent |
| Add missing traceability comment | Improves auditability without changing behavior |
| Replace raw float formatting with the shared Java-compatible formatter | Fixes a common mechanical parity trap when the output path is obvious |
| Add missing pub on POJO-style field | Restores Java-like data visibility without changing algorithmic behavior |

### Deny List

| Pattern | Why it stays manual |
|---|---|
| Different branch structure | Could reflect a real logic divergence or an intentional rewrite; the audit should not guess |
| Different algorithm or state update strategy | High risk of changing behavior without full diagnosis |
| Missing method | Could be dead code, renamed code, inlined logic, or a real omission |
| Ambiguous logic difference of any kind | The report should surface uncertainty, not bury it behind a speculative patch |

If a finding touches the deny list, leave the code alone and document it clearly.

## Verdict Criteria

Use these verdicts consistently.

### VERIFIED

Use VERIFIED when:

- a Rust counterpart exists
- the inspected logic is materially equivalent
- parity-sensitive mechanics are either correct or not applicable
- no unresolved concern remains after any small mechanical fix

### NEEDS_REVIEW

Use NEEDS_REVIEW when:

- the counterpart exists but at least one dimension is ambiguous
- a mechanical issue was found but the surrounding logic still needs a human look
- the Java doc and Rust code line up imperfectly enough that confidence is limited

This is the normal bucket for uncertain but non-fatal findings. It tells the reviewer where attention is needed without overstating failure.

### FAILED

Use FAILED when:

- the method appears missing with no credible counterpart
- there is a clear logic, ordering, null-handling, or formatting divergence
- the class-level completeness is too weak to justify Tier 2 progression

FAILED means the audit found a concrete blocker, not just discomfort.

## Anti-Patterns

Avoid these mistakes while using the skill:

- Treating a Tier 1 pass as proof that the code structure is complete
- Auto-fixing ambiguous logic because the patch seems probably right
- Auditing only public entry points and ignoring helper methods that feed output-affecting behavior
- Comparing names only instead of branch structure, null propagation, ordering, and formatting
- Committing the report by default instead of keeping it under tmp/
- Expanding the audit to unrelated modules just because they are nearby in the codebase

## Handoff Rules

Advance to Tier 2 only when the report supports it.

- If every class-level status is VERIFIED and the remaining issues are either absent or purely mechanical and already validated, hand off to tiered-config-test or the next Tier 2 sweep step.
- If any item is NEEDS_REVIEW, stop and present the report to the user or reviewer before broadening test coverage.
- If any item is FAILED, do not promote the module. Surface the blocking findings and route the work back to manual repair or the appropriate porting follow-up.

The spirit of this skill is simple: use white-box inspection to buy confidence that the black-box pass means something durable, then move to Tier 2 with fewer hidden surprises.