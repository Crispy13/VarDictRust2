# Known Parity Gaps (DO NOT USE these flags)

**Status:** These VarDictJava flags are **wired into VarDict-rs but NOT byte-identical** to VarDictJava.
Their config-e2e presets exist for coverage/declaration, but the parity assertion is **skipped** (see
`KNOWN_PARITY_GAP_PRESETS` in [tests/common/mod.rs](../tests/common/mod.rs) and the skip in `run_cell`). Do not
rely on these flags in production — VarDict-rs output diverges from VarDictJava. Each is a tracked fix-later task.

When a gap is fixed: remove the preset from `KNOWN_PARITY_GAP_PRESETS`, regenerate its golden, and confirm the
cells + push tests pass byte-identical; then delete its row here.

| Flag | Preset | Divergence (observed on the config-e2e regions) | Likely fix site |
|------|--------|--------------------------------------------------|-----------------|
| `-D` (debug) | `CM-DEBUG` | Debug genotype column format differs. Java: `G:3:F-1:R-2:1.0000:2:22.7:1:31.7:1:1.0000:60.0:6.000`. vdr: `Variant: G Cnt=3 Fwd=1 Rev=2`. | `src/mods/output.rs` (the `debug` field formatter) |
| `--UN` (unique, first-in-pair) | `CM-UNIQUN` | 2/100 regions diverge. VarDictJava throws `IllegalStateException: Inappropriate call if not paired read` on unpaired reads, catches it, and skips the record; vdr does not replicate the skip under unique-second-in-pair mode. | sam parsing / `src/mods/cigar_parser.rs` (unique-mode read handling) |
| `-x` (segment extend) | `CM-EXTEND` | Region label/scan not extended. With `-x 50`, Java reports the extended region (e.g. `20:1515366-1518971`); vdr reports the original (`20:1515416-1518921`). The config-e2e harness `parse_region` (`tests/common/mod.rs`) builds the region without applying `number_nucleotide_to_extend`. | config-e2e harness region setup (and verify the production CLI region-build path applies `-x`) |

## How these were found
Added during the config-preset flag-coverage expansion (teeth-probe → 12 new presets). 9 of the 12 were
byte-identical; these 3 surfaced real divergences — see `tmp/preset-teeth-probe/REPORT.md`. Decision (user):
ship all 12 in the test suite now, defer these 3 fixes to a later task, and mark them do-not-use here.
