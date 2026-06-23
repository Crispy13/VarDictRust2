# Known Parity Gaps

**Status: one bounded, intent-justified gap** (`--UN` on unpaired-read-heavy BAMs — see below). Every
VarDictJava flag wired into VarDict-rs is byte-identical to VarDictJava at config-e2e scale, and
`KNOWN_PARITY_GAP_PRESETS` in [tests/common/mod.rs](../tests/common/mod.rs) is empty (the gap below is
*not* a config-e2e failure — it only manifests on inputs VarDictJava itself cannot process).

## Known Gaps

### `--UN` (unique, second-in-pair) on BAMs with many unpaired reads

On a BAM containing more than ~10 unpaired reads, **VarDictJava crashes (exit 1, no output)** under
`--UN`, whereas **VarDict-rs emits a correct, complete variant table.** The two are therefore not
byte-identical on such inputs — but only because VarDictJava produces *no* output to compare against.

**Root cause — a latent VarDictJava bug, not a design choice.** `CigarParser.skipOverlappingReads`
(`CigarParser.java:1916`) evaluates its `&&` chain in an unsafe order:

```java
// --UNA branch (line ~1912): paired guard FIRST — safe
if (conf.uniqueModeAlignmentEnabled && isPairedAndSameChromosome(record) && ...) { ... }

// --UN  branch (line 1916): getSecondOfPairFlag() FIRST — unguarded  ← bug
if (conf.uniqueModeSecondInPairEnabled && record.getSecondOfPairFlag() && isPairedAndSameChromosome(record) && ...) { ... }
```

`getSecondOfPairFlag()` throws `IllegalStateException: Inappropriate call if not paired read` on
unpaired reads. Because the paired guard is placed *after* it in the `&&` chain, it never short-circuits
in time. The sibling `--UNA` branch ten lines up puts the paired check first and is safe — proving the
intended order. The thrown exception is caught per-record (`CigarParser.java:104` →
`Utils.printExceptionAndContinue`), but the generic `MAX_EXCEPTION_COUNT` backstop (`Utils.java:273`,
designed to bail out on a genuinely corrupt BAM after ~10 caught exceptions) then escalates to a
critical `CompletionException` and stops the whole run with exit 1 and no output.

**Why VarDict-rs diverges by design.** The intent of `--UN` is "for *paired* reads, skip the
second-in-pair read when it overlaps its mate." Unpaired reads were never meant to reach
`getSecondOfPairFlag()`. VarDict-rs honors that intent: `src/mods/cigar_parser.rs` skips unpaired reads
under `unique_mode_second_in_pair_enabled` and continues. Reproducing VarDictJava's crash would mean
deliberately porting its operand-ordering bug so that VarDict-rs *also* produces no output on inputs the
author intended to handle — so we do not.

**Bound / scope.**
- On BAMs with **0 (or ≤ ~10) unpaired reads**, the threshold is never crossed: both tools skip the
  same reads and are **byte-identical**. This covers all paired-read fixtures (exome `hg002`,
  `hg005_exome`, somatic `wes_il_pair` — all have 0 unpaired reads), so `CM-UNIQUN` parity *is* exercised
  and green on the sweep tier there.
- On BAMs with **many unpaired reads** (e.g. `na12878_lowcov`: 930,836 unpaired reads), VarDictJava
  crashes and cannot produce a golden fixture at all, so `CM-UNIQUN` is **ungeneratable** there
  (`na12878_lowcov` is complete at 11/12 presets) and the divergence is unobservable by diff.

In short: VarDict-rs matches VarDictJava byte-for-byte everywhere VarDictJava actually emits output under
`--UN`; the only divergence is on inputs where VarDictJava crashes by its own bug.

## Resolved

The flags below were previously deferred gaps; they are now repaired and their config-e2e parity
assertions pass byte-identical (un-skipped):

| Flag | Preset | Fix |
|------|--------|-----|
| `-x` (segment extend) | `CM-EXTEND` | The config-e2e/sweep test harness now applies `number_nucleotide_to_extend` when building the scan Region (matching the production `-R`/BED paths in `src/bin/vardict_rs.rs`). Production was already correct. |
| `-D` (debug column) | `CM-DEBUG` | `src/mods/to_vars_builder.rs` now emits VarDictJava's `debugVariantsContent` format (`n:cnt:F-fwd:R-rev:freq:…`), with the `I` prefix for insertions. |

If a new gap is discovered: add the preset to `KNOWN_PARITY_GAP_PRESETS`, document it here, and track the fix.
