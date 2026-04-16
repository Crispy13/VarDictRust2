---
name: rust-freshness-verification
description: >-
  Verify version-sensitive Rust claims before you answer with confidence. Reach for
  this skill when the user asks about Rust editions, crate versions, MSRV, target
  support, toolchain behavior, recently stabilized syntax, or when you are about to
  generate code that quietly depends on a specific edition or crate major version.
  Also trigger when writing setup or configuration code for crates whose API surface
  shifts across releases (e.g., tracing-subscriber, clap, axum, hyper). Trigger even
  when the user only hints at recency with words like latest, current, new project,
  upgrade, or supported today, because stale defaults create subtle breakage that
  looks authoritative until the build or docs disagree.
---

# Freshness Verification

## Purpose

Use this skill when a Rust answer can go stale faster than the model's background knowledge.
Version claims sound precise, which makes them easy to trust and expensive to debug later.
Run a short verification chain before stating an edition, a crate version, a stabilization fact,
or a toolchain capability as if it were current.
Keep the response useful even when fresh sources are unavailable by naming the uncertainty and
showing the user how to verify the last step directly.

## When to Verify

Verify when the user asks for an explicit version, edition, or release recommendation.
These questions invite a concrete answer, and concrete answers are the easiest place for stale facts to hide.

Verify when you are about to generate code that depends on edition-gated or recently stabilized syntax.
The code may look correct while still failing on the user's edition or toolchain.

Verify when the user asks for the latest dependency version or says to add a crate without a version.
Dependency advice becomes wrong quickly, and a casual "latest" can lock a project onto the wrong major version.

Verify when you mention APIs that changed across major crate releases, such as clap, tokio, axum, or hyper.
The surface syntax often looks familiar enough that outdated examples slip through review.

Verify when the user asks about MSRV, target triples, tier support, platform availability, or toolchain channels.
These facts change over time and affect whether a recommendation works outside your own assumptions.

Verify when the user mentions upgrading, migrating, modernizing, or using a newly released Rust feature.
Migration questions usually hinge on what changed recently, not what used to be true.

Verify proactively during code generation if you need to mention "requires Rust X", "edition 2024", or a crate major version.
Doing the check up front is cheaper than writing code that later needs caveats or rewrites.

## Verification Procedure

Follow this tiered chain and stop at the first trustworthy source.

### Tier 1: Check instruction pins

Check loaded instruction files and workspace guidance for explicit version pins or policy statements.
Use these first because they are zero-latency and often encode the project's intended truth.
Look for pinned Rust editions, preferred channels, crate versions, or documented defaults in `.instructions.md` files and related guidance.

### Tier 2: Check the freshness cache

Check `./tmp/freshness-cache/` for a recent verified fact that matches the claim you want to make.
Use the cache when it names the fact, includes a verification date, and cites the source that justified it.
Prefer cache hits for facts that are still inside the advisory freshness window.
Treat stale entries as hints, not proof, and continue down the chain when age matters.

### Tier 3: Query Context7 when available

Query Context7 for the authoritative docs when the first two tiers do not settle the answer.
Resolve the right library or documentation source first, then query the docs for the specific fact you need.
Use a narrow question such as the current crate major version, the edition required for syntax, or the documented MSRV.
If Context7 is not configured, unavailable, or does not cover the relevant source, say that plainly and continue to Tier 4 without blocking the response.

### Tier 4: State uncertainty and show the next check

If you still cannot verify the claim, say that you are not sure it is current.
Give the user a direct verification step instead of bluffing, such as `cargo search serde --limit 1`,
`rustc --version`, `rustup target list --installed`, or the relevant official crate or Rust documentation URL.
Phrase the answer so the uncertain part is isolated from the rest of the guidance.

After a successful Tier 3 lookup or manual verification, write the fact to the freshness cache if it is likely to help again.

## Examples

### Example 1: Edition question with a Tier 1 hit

User: "What edition should I use for a new Rust project in 2025?"

1. Check instruction pins for an explicit workspace default.
2. Find a pin that says new Rust work should target edition 2024.
3. Answer with edition 2024 and note that the guidance came from the workspace pin.

Why this chain works: the project already declared the intended default, so a faster external lookup would only add latency without improving confidence.

### Example 2: Latest dependency request with Tier 2 miss and Tier 3 lookup

User: "Add the latest serde to Cargo.toml."

1. Check instruction pins for a pinned serde version and find none.
2. Check `./tmp/freshness-cache/` and find no recent serde entry.
3. Query Context7 for serde's current recommended crate version.
4. Add the verified version, then record the fact in the cache with the date and source.

Why this chain works: "latest" is exactly the kind of claim that ages out, so an old memory is less useful than a fresh doc-backed answer.

### Example 3: Proactive syntax verification with graceful Context7 fallback

User: "Write an async closure for this callback."

1. Notice that the requested syntax may depend on edition or recent stabilization details.
2. Check instruction pins for edition guidance and confirm edition 2024 is expected.
3. If more detail is needed, try Context7; if it is unavailable, say you are not fully sure the stabilization details are current.
4. Provide the code together with the edition assumption and a direct follow-up verification step.

Why this chain works: proactive verification prevents code generation from silently assuming a capability the user's toolchain may not support.

## Respect Project Context

Trust project-local facts before global defaults.
If `Cargo.toml`, `rust-toolchain.toml`, CI files, or workspace instructions specify `edition`, `rust-version`, channel, or pinned crate versions, treat that as the active source of truth for this repository.
Verify only the claims that go beyond what the project already states.
When project-local settings conflict with a general recommendation, explain the difference instead of overwriting the local choice.

## Freshness Cache

Store reusable verified facts in `./tmp/freshness-cache/`.
Use Markdown files that record the fact, the verification date, and the source used to verify it.
Re-verify toolchain and target-support facts when they are older than 6 weeks.
Re-verify edition, MSRV, and ecosystem guidance facts when they are older than 6 months.
Write to the cache after a successful Context7 lookup or after a direct manual verification step.
Skip cache writes for guesses, unresolved contradictions, or answers that still carry an uncertainty warning.

Suggested entry shape:

```markdown
## serde current version
- Verified: 2026-04-15
- Source: docs.rs / crates.io / Context7 lookup
- Fact: Use serde = "<verified version>"
```

## Reference Files

Read `references/rust-ecosystem.md` for edition history, version-sensitive crates, and heuristics about which claims usually need verification.
Read `references/context7-setup.md` when Context7 is missing or unconfigured and you need the setup path or fallback workflow.