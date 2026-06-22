# Known Parity Gaps

**Status: none.** All VarDictJava flags wired into VarDict-rs are byte-identical to VarDictJava at
config-e2e scale, and `KNOWN_PARITY_GAP_PRESETS` in [tests/common/mod.rs](../tests/common/mod.rs) is empty.

## Resolved

The three flags below were previously deferred gaps; they are now repaired and their config-e2e parity
assertions pass byte-identical (un-skipped):

| Flag | Preset | Fix |
|------|--------|-----|
| `-x` (segment extend) | `CM-EXTEND` | The config-e2e/sweep test harness now applies `number_nucleotide_to_extend` when building the scan Region (matching the production `-R`/BED paths in `src/bin/vardict_rs.rs`). Production was already correct. |
| `--UN` (unique, second-in-pair) | `CM-UNIQUN` | `src/mods/cigar_parser.rs` now fully skips unpaired reads under `unique_mode_second_in_pair_enabled`, replicating VarDictJava's `getSecondOfPairFlag()` `IllegalStateException` → per-record catch-and-skip. |
| `-D` (debug column) | `CM-DEBUG` | `src/mods/to_vars_builder.rs` now emits VarDictJava's `debugVariantsContent` format (`n:cnt:F-fwd:R-rev:freq:…`), with the `I` prefix for insertions. |

If a new gap is discovered: add the preset to `KNOWN_PARITY_GAP_PRESETS`, document it here, and track the fix.
