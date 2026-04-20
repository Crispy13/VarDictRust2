# structural_variants_processor

**Source**: `src/mods/structural_variants_processor.rs` (3,745 LOC)
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/modules/StructuralVariantsProcessor.java`
**Status**: complete

## Overview

Detects structural variants (DEL, INV, DUP) from soft-clipped read consensus sequences and discordant read pairs. Sits between VariationRealigner (S16) and ToVarsBuilder (S18). Processes SV evidence stored in `SVStructures` (populated during BAM traversal), finds consensus sequences, aligns them to reference via seed-based matching, constructs variant description strings, and adjusts variant/coverage counts. Also provides `find_match`, `mark_sv`, and `mark_dup_sv` used by S16's realignlgdel/realignlgins30.

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
| `partial_pipeline_stub()` | yes | No-op stub for off-window pipeline re-entry |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `StructuralVariantsProcessor` class | Free functions in `structural_variants_processor.rs` | No struct; explicit `&mut` params |
| `initFromScope()` | Inlined into `process()` argument list | Field transfer via `RealignedVariationData` |
| `PairsData` inner class | `PairsData` struct | Direct mapping |
| `getMode().partialPipeline(...)` | `partial_pipeline_stub()` | No-op; requires full pipeline integration |
| `StructuralVariantsJsonlWriter` | Not ported | Debug-only; non-blocking for TSV parity |

## Known Parity Traps

1. **Collection ordering**: `VariationMap.entries` = `IndexMap` (Java `LinkedHashMap`), `Sclip.soft` = `IndexMap`. `fill_and_sort_tmp_sv` uses stable sort on count only — no tiebreaker.
2. **markSV vs markDUPSV coordinates**: markSV uses inner (end/mstart), markDUPSV uses outer (start/mend). Critical difference.
3. **Seed uniqueness**: `seeds.len() == 1` — ambiguous seeds skipped entirely.
4. **findMatchRev complements only**: Complement-only on sequence. Reverse done separately for dir==1.
5. **findsv pairs==0**: Triggers `continue`, NOT INV fallthrough (trap 21).
6. **Left-alignment asymmetry**: findDEL uses `ref[bp] == ref[softp-1]`; findINVsub uses `ref[softp] == complement(ref[bp])`.
7. **Integer division fwd/rev split**: `mcnt / 2` and `mcnt - mcnt / 2` — exact integer division preserved.
8. **DUP extracnt**: `tmp.extracnt = tcnt` set for DUP only.
9. **Doubled stats in findDELdisc**: `2 * varsCount`, `2 * meanQuality`, etc. for discordant pairs.
10. **varsCount = 0 explicit reset**: Preserved after `get_variation()` for re-existing entries.

## Divergences

1. **partial_pipeline_stub**: Java re-runs upstream partial pipeline for off-window SV context. Rust no-ops. Affects region-boundary DEL/INV/DUP discovery for SVs extending beyond the window. Does not affect within-region Tier 1 parity (100/100 PASS).
2. **JSONL writer**: Java debug helper not ported. Non-blocking for TSV parity.

## Cross-Module Dependencies

- **Upstream**: Called by pipeline after VariationRealigner (S16). Receives `RealignedVariationData`.
- **Downstream**: Output consumed by ToVarsBuilder (S18). Writes to `non_insertion_variants`, `insertion_variants`, `ref_coverage`.
- **Cross-calls to S16**: `find_inv_sub` creates realigndel callback via direct function call with previousScope data.
- **Cross-calls from S16**: `find_match`, `mark_sv`, `mark_dup_sv` called by `realignlgdel`/`realignlgins30` in variation_realigner.rs.
- **Shared utilities**: `find_conseq`, `adj_cnt`, `inc_cnt`, `get_variation` from `variations.rs`; `ismatchref_with_mm` from `variation_realigner.rs`; complement/reverse from `utils.rs`.
