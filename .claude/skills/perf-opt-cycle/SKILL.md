---
name: perf-opt-cycle
description: >-
  End-to-end performance optimization cycle for VarDict-rs, tuned to how this project actually
  iterates. Use this WHENEVER the user wants to optimize runtime or per-preset/production time,
  says "next optimization cycle", "find the next opt target", "prepare/run a perf cycle", asks to
  reduce sweep makespan, speed up a chunk/preset/region, or pick what to optimize next — even if
  they don't say the word "skill". It drives the full loop: pick the bottleneck target from the
  parity run log → profile it → devise a parity-safe change → implement → validate (byte-identical
  parity + User-CPU/RSS on the worst AND a typical chunk) → logic-parity-audit → decision report
  for the user to accept or reject. Orchestrates the `perf-optimization` and `logic-parity-audit`
  skills with this project's hard-won measurement discipline (measure the REAL pipeline, not just a
  microbench). Do NOT use for parity mismatch repair (use `mismatch-repair`) or initial porting.
---

# Perf-Opt Cycle — VarDict-rs

One trip through the loop turns "the sweep is slow" into a measured, parity-safe change with an
accept/reject recommendation. This skill exists because raw `perf-optimization` doesn't say *where*
to look or *how this repo validates parity*; the costly mistakes here are optimizing the wrong thing,
trusting a microbench, or shipping a CPU win that silently regresses memory or parity.

## Division of labor (reasoning vs execution)

Keep the engineering judgment with the orchestrator; offload mechanical execution.

- **Orchestrator (you) — all reasoning:** (1) pick the target from the parity run log; (2) write the
  exact profile command; (3) interpret the profile and **devise + design** the change; (4) run the
  **logic-parity-audit** yourself.
- **Execution — delegate to a high-reasoning Sonnet subagent when available** (runs your profile
  command, builds, implements the change you designed, measures/tests). If subagents aren't
  available, do it inline. Hand the executor a fully-specified packet, not "go figure out what to do."
- **The user decides acceptance. Never commit without explicit approval.**

> Why: a fully-delegated perf run once picked a change the compiler already did (a no-op). The
> reasoning stages are where value and risk live — keep them close.

## Stage 0 — Pick the target from the parity run log (reasoning)

Per-preset production wall ≈ (Σ per-chunk single-thread `rust_run`) ÷ thread count. So the makespan is
**parallelism-bound on total CPU work**, and the lever is the **common-floor cost every chunk pays** —
*not* the single tail chunk. Validate on the **heaviest chunk**: it both dominates the makespan and
amplifies the hotspot for profiling.

Read the newest parity-iteration log (e.g. under a sibling repo's
`tmp/parity-iteration/<run>/`): `cell-runtimes.jsonl` (per-chunk `rust_run`, `libtest_seconds`,
`preset`, `region_str`, `chunk_id`) and `progress.log` (per-preset wall). With Python (no `jq` here),
find the **slowest preset by `libtest_seconds`** and its **heaviest chunk by `rust_run`**. Look up that
preset's config flags in `tests/common/mod.rs` and translate to standalone CLI flags (e.g.
`PW-004` = `minr=8, min_bias_reads=1` → `-r 8 -B 1`). State the target explicitly: preset, chunk,
region, flags.

## Stage 1 — Profile that target (your command, executor runs it)

Profile the standalone binary single-thread on the target region; frame-pointer graphs over-attribute,
so use DWARF:

```
perf record -g --call-graph dwarf -F 997 -o tmp/perf_<tag>.data -- \
  ./target/debug-release/vardict_rs -R <region> \
  -b testdata/<bam> -G testdata/hs37d5.fa -N NA12878 <preset-flags> --th 1 > /dev/null
perf script -i tmp/perf_<tag>.data | inferno-collapse-perf > tmp/perf_<tag>.collapsed
inferno-flamegraph tmp/perf_<tag>.collapsed > tmp/flamegraph_<tag>.svg
```

**Interpret it yourself.** Leaf self-time alone misleads: the top leaves are usually inlined
primitives (`core::intrinsics::likely`, `__mm_movemask_epi8` = hashbrown SIMD probing,
`core::ptr::write` = hash-entry writes). **Resolve each hot primitive to its nearest `vardict_rs::`
ancestor** (walk each collapsed stack up to the first `vardict_rs::` frame, sum by primitive×ancestor).
That tells you the real hot functions. Note what is *not* controllable (BGZF `deflate_decompress` is
external libdeflate).

## Stage 2 — Devise the change (reasoning)

Pick the largest **controllable, parity-safe** lever, preferring `perf-optimization`'s low-risk tiers
(algorithm / data-structure / allocation) over micro-tweaks. Before designing, settle the parity
constraint for the structure you'll touch:

- **Output ordering:** VarDict sorts variant keys before emitting (`to_vars_builder.rs` + Java
  `ToVarsBuilder.java`), so per-position map *iteration order never reaches output* — a map's type is
  swappable as long as the **key set and values** match. But module-parity JSONL fixtures serialize
  some maps in **insertion order** (`src/parity/format.rs`), so a replacement must preserve insertion
  order and match the serde wire format.
- **Presence vs value:** if code distinguishes `contains_key` from `value==0`, a dense replacement must
  track presence separately (sentinel / `Option`), not conflate "absent" with "present-but-0".
- **Position-keyed maps:** processing order is re-derived via a Java-HashMap-order helper from the key
  set, so the map type is swappable; but positions can extend slightly outside the region (read
  overhang, SV) — handle out-of-range with a fallback, don't assume tight bounds.

Proven wins in this repo (pattern library): `IndexMap`→insertion-ordered `Vec` for tiny per-position
maps (`VecMap`); `HashMap<i32,_>`→offset-indexed dense array with sliding window + fallback
(`CoverageMap`); removing redundant UTF-8 validation on provably-ASCII bytes.

## Stage 3 — Implement (execution)

One optimization at a time, no bundling. Hand the executor the exact design (new type's full API,
which alias/sites change, serde requirements). Tell it to **STOP and report** if a call site needs an
API you didn't anticipate, rather than improvising parity-affecting logic.

## Stage 4 — Validate — the gates (execution measures; you judge)

1. **Measure the REAL pipeline, not just a microbench.** A microbench is a cheap go/no-go gate, but the
   real chunk is the arbiter — a microbench once predicted a Vec swap would regress at N=100, yet the
   real worst chunk was 30% *faster* (high coverage ≠ many distinct alleles). Don't reject or accept on
   the microbench alone.
2. **User-CPU is the load-robust metric.** Check `uptime` first. On an idle box use `hyperfine`
   (warmup + ≥3 runs) for wall; always capture `/usr/bin/time -v` **User CPU + Peak RSS**. Under load,
   absolute User-CPU inflates (memory-bandwidth contention) — use a **consecutive-pair A/B** (baseline
   then optimized back-to-back) and trust the *relative* delta; flag that an idle re-measure is owed.
3. **Measure worst AND typical** chunks, and for any **dense-layout** change also a **low-coverage**
   region — a dense array can trade CPU for RSS on sparse data. **No RSS regression** beyond guardrail.
4. **Parity (byte-identical is the bar):** clean-baseline diff — `git stash` your change, build, capture
   baseline TSV; restore, build, capture optimized TSV; `diff` must be empty — on worst AND typical.
   Plus `cargo test --profile debug-release --test parity_suite tovars:: -- --include-ignored` (the
   runnable local golden; exercises serde) and `cargo test --profile debug-release --lib`.
5. **REJECT** on any tail regression, RSS regression, or parity/test failure — and "no conservative
   safe win exists, stop here" is a perfectly valid, honest outcome of Stage 1.

## Stage 5 — Logic-parity-audit (reasoning, you — no subagent)

Invoke the `logic-parity-audit` skill on the touched code vs its Java source (read read-only from a
sibling `VarDictJava` if not vendored). Verify operation-by-operation equivalence, key-set/order
preservation, presence/default semantics, and serde. Write the report under `tmp/logic-parity-audit/`
with a VERIFIED / NEEDS_REVIEW / FAILED verdict. Empirical byte-identical output supports but does not
replace this structural check.

## Stage 6 — Decision report (the user decides)

Write one report under `tmp/` and announce its path. Use this shape:

```
# Perf Optimization — <change> — Decision Report
## TL;DR (one line: what, the headline %, ACCEPT/REJECT lean)
## Target (from the parity run log)
## Profile (your command; your interpretation of the hot path)
## The change (design; files/blast radius)
## Measured results (before/after User-CPU + Peak RSS on worst AND typical; note load)
## Parity & correctness (clean diff + parity_suite tovars + tests; audit verdict + report path)
## Estimated per-preset / sweep impact
## Recommendation: ACCEPT/REJECT  — decision is the user's; nothing committed
## Post-acceptance: idle re-measure · change-impact-review · regen Java goldens at HEAD before merge
```

## Environment & gotchas

- Activate fully or `hts-sys` bindgen fails (`stddef.h not found`):
  `source <conda>/etc/profile.d/conda.sh && conda activate vdr && export LIBCLANG_PATH="$CONDA_PREFIX/lib"`.
- Binary is `vardict_rs`; build `cargo build --profile debug-release`. `jq` is not installed — parse
  JSONL with Python.
- The **authoritative full-sweep Java golden trial is commit-gated locally** (cache pinned to an older
  commit than HEAD), so it can't run here — rely on `parity_suite tovars` + the clean-baseline diff,
  and recommend regenerating goldens at HEAD before merge.
- **Workspace boundary:** edit/build/test only in the active VarDict-rs workspace; sibling repos
  (parity logs, VarDictJava) are read-only.
- One optimization per cycle; cycles stack on prior uncommitted wins — isolate the current change for
  the parity diff by capturing the baseline before changing.
