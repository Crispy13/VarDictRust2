# TODO

Deferred work items in the VarDict-rs port that must be resolved before full production parity. Each item includes the blocker, impact, and unblock path so future work can pick it up without re-discovery.

---
# Planned
## Insepct and optimize high memory usage in testing:
- T1-02 somatic bam running with 12 threads takes about 18G.

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

## Conventions

- Add new items as numbered sections at the top (newest first) or in the appropriate category.
- Each item must record: **status / location / Java counterpart / current Rust behavior / why blocked / impact / unblock path / detection**.
- Remove items only when the underlying gap is closed and verified by a parity test.
