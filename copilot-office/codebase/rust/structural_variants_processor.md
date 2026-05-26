# structural_variants_processor

**Source**: `src/mods/structural_variants_processor.rs` (3,745 LOC)
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/modules/StructuralVariantsProcessor.java`
**Status**: complete

## Overview

Detects structural variants (DEL, INV, DUP) from soft-clipped read consensus sequences and discordant read pairs. Sits between VariationRealigner (S16) and ToVarsBuilder (S18). Processes SV evidence stored in `SVStructures` (populated during BAM traversal), finds consensus sequences, aligns them to reference via seed-based matching, constructs variant description strings, and adjusts variant/coverage counts. Boundary SV paths now re-enter the partial pipeline for Java-equivalent reference-extension windows. The module also provides `find_match`, `mark_sv`, and `mark_dup_sv` used by S16's realignlgdel/realignlgins30.

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `process()` | yes | Main entry: gate SV → find_all_svs → adj_snv → output_clipping |
| `find_all_svs()` | yes | Orchestrates DEL → INV → findsv → DELdisc → INVdisc → DUPdisc in strict order |
| `find_del()` | yes | 5' and 3' soft-clip-based deletion detection |
| `find_inv()` | yes | Dispatcher for 4 inversion sub-calls |
| `find_inv_sub()` | yes | Core inversion logic with complement alignment and realigndel callback |
| `findsv()` | yes | General SV from raw soft-clips (DEL + INV fallback) |
| `find_del_disc()` | yes | DEL from discordant pairs only |
| `find_inv_disc()` | yes | INV from discordant pairs (fwd×rev nested loop) |
| `find_dup_disc()` | yes | DUP from discordant pairs (writes to insertion_variants) |
| `find_match()` | yes | Forward seed-based alignment with complex indel fallback |
| `find_match_rev()` | yes | Reverse/complement seed-based alignment |
| `find_match_default()` | yes | Default-seed wrapper for find_match |
| `find_match_rev_default()` | yes | Default-seed wrapper for find_match_rev |
| `is_overlap()` | yes | Interval overlap with 3×rlen proximity rule |
| `check_pairs()` | yes | Highest-count representative selection + marking overlapping clusters |
| `mark_sv()` | yes | Inner-coordinate overlap marking for DEL/INV |
| `mark_dup_sv()` | yes | Outer-coordinate overlap marking for DUP |
| `fill_and_sort_tmp_sv()` | yes | Filter to curseg + stable sort by count descending |
| `adj_snv()` | yes | Short soft-clip rescue as SNVs (≤5bp consensus) |
| `output_clipping()` | yes | Debug remaining-clip stderr output |
| `run_partial_pipeline()` | yes | Re-enters SAMFileParser + CigarParser(true) for boundary SV reference-extension windows |
| `prefetch_dup_breakpoint_reference_if_missing()` | yes | Mirrors Java `findDUPdisc()` predicted-breakpoint reference prefetch before DUP partial-pipeline gates |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `StructuralVariantsProcessor` class | Free functions in `structural_variants_processor.rs` | No struct; explicit `&mut` params |
| `initFromScope()` | Inlined into `process()` argument list | Field transfer via `RealignedVariationData` |
| `PairsData` inner class | `PairsData` struct | Direct mapping |
| `getMode().partialPipeline(...)` | `run_partial_pipeline()` | Moves live variation/coverage/soft-clip maps through SAMFileParser and CigarParser(true), then writes updated maps back |
| `findDUPdisc()` breakpoint `getReference(bp/pe +/- 150, 300, reference)` | `prefetch_dup_breakpoint_reference_if_missing()` before `run_partial_pipeline()` | Preserves Java's reference-loaded side effect for forward `bp` and reverse `pe` DUP gates |
| `StructuralVariantsJsonlWriter` | Not ported | Debug-only; non-blocking for TSV parity |

## Known Parity Traps

1. **Collection ordering**: `VariationMap.entries` = `IndexMap` (Java `LinkedHashMap`), `Sclip.soft` = `IndexMap`. `fill_and_sort_tmp_sv` preserves count-descending priority and uses ascending position as the deterministic equal-count tie-break for Rust `HashMap` soft-clip inputs. `find_del` no-softp fallback scans snapshot and sort `soft_clips_3_end` / `soft_clips_5_end` keys before the Java first-match break, avoiding process-random Rust `HashMap` traversal at candidate-selection boundaries.
2. **markSV vs markDUPSV coordinates**: markSV uses inner (end/mstart), markDUPSV uses outer (start/mend). Critical difference.
3. **Seed uniqueness**: `seeds.len() == 1` — ambiguous seeds skipped entirely.
4. **findMatchRev complements only**: Complement-only on sequence. Reverse done separately for dir==1.
5. **findsv pairs==0**: Triggers `continue`, NOT INV fallthrough (trap 21).
6. **Left-alignment asymmetry**: findDEL uses `ref[bp] == ref[softp-1]`; findINVsub uses `ref[softp] == complement(ref[bp])`.
7. **Integer division fwd/rev split**: `mcnt / 2` and `mcnt - mcnt / 2` — exact integer division preserved.
8. **DUP extracnt**: `tmp.extracnt = tcnt` set for DUP only.
9. **Doubled stats in findDELdisc**: `2 * varsCount`, `2 * meanQuality`, etc. for discordant pairs.
10. **varsCount = 0 explicit reset**: Preserved after `get_variation()` for re-existing entries.
11. **Partial-pipeline re-entry**: DEL, INV, and DUP boundary branches must stay guarded by Java-equivalent `isLoaded` checks and must thread live `non_insertion_variants`, `insertion_variants`, `ref_coverage`, and soft-clip maps through the re-entry.
12. **DUP predicted-breakpoint reference prefetch**: In `findDUPdisc()`, Java prefetches `bp +/- 150` for forward DUP and `pe +/- 150` for reverse DUP, with extension `300`, before running the current-cluster partial pipeline when the predicted base is absent. This mutates loaded-reference state for later DUP clusters and prevents extra Rust-only partial-pipeline writebacks.
13. **Forward DUP refined-end coverage handoff**: In `findDUPdisc()` forward DUP soft-clip realignment, Java mutates the local `end` after `pe` is left-aligned (`end = pe`) and then compares/promotes `refCoverage[end]` into `refCoverage[bp]`. Rust must update the same local `end`; reusing the original `dup.end` copies coverage from the wrong coordinate.

## Divergences

1. **Partial-pipeline debug snapshots**: Rust now applies the Java-equivalent map side effects for SV boundary re-entry, but does not currently emit a separate partial-pipeline CIGAR debug snapshot. Main-region `sv_processor` and final TSV parity are the acceptance surface.
2. **Deterministic Rust `HashMap` ordering guards**: Java soft-clip maps are `HashMap`-backed but deterministic for the observed fixture/runtime. Rust applies explicit ascending-position ordering at output-affecting SV candidate surfaces where Java breaks on the first viable candidate; this is an intentional parity guard, not a downstream output adapter.

## Cross-Module Dependencies

- **Upstream**: Called by pipeline after VariationRealigner (S16). Receives `RealignedVariationData`.
- **Downstream**: Output consumed by ToVarsBuilder (S18). Writes to `non_insertion_variants`, `insertion_variants`, `ref_coverage`.
- **Cross-calls to S16**: `find_inv_sub` creates realigndel callback via direct function call with previousScope data.
- **Cross-calls from S16**: `find_match`, `mark_sv`, `mark_dup_sv` called by `realignlgdel`/`realignlgins30` in variation_realigner.rs.
- **Partial re-entry**: `run_partial_pipeline` calls `sam_file_parser_process` and `CigarParser::new(true).process_preprocessor` over modified boundary regions, matching Java `AbstractMode.partialPipeline()` side effects.
- **Shared utilities**: `find_conseq`, `adj_cnt`, `inc_cnt`, `get_variation` from `variations.rs`; `ismatchref_with_mm` from `variation_realigner.rs`; complement/reverse from `utils.rs`.
