# to_vars_builder

**Source**: `src/mods/to_vars_builder.rs` (~1,470 LOC)
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/modules/ToVarsBuilder.java` (1,194 LOC)
**Status**: complete

## Overview

ToVarsBuilder transforms raw `Variation` maps (from CigarParser → VariationRealigner → StructuralVariantsProcessor) into final `Variant` objects. It takes `Scope<RealignedVariationData>` and produces `Scope<AlignedVarsData>`. The module iterates over non-insertion variant positions, builds `Variant` records with computed statistics (frequency, strand bias, quality, MSI, genotype, allele strings, flanking sequences), and organizes them into `Vars` structures keyed by position. Classification: Analysis-heavy.

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `process` | yes | Main entry; loops sorted positions, delegates to per-position processing |
| `process_position` | yes | Extracted helper for single position; enables `catch_unwind` |
| `create_variant` | yes | Builds `Variant` objects from non-insertion `Variation` entries |
| `create_insertion` | yes | Builds `Variant` objects from insertion `Variation` entries; returns updated `totalPosCoverage` |
| `calc_hicov` | yes | Sums high-quality read counts (excludes insertion/SV keys) |
| `adjust_variant_counts` | yes | Clamps negative fwd/rev counts to zero with stderr warning |
| `sort_variants` | yes | Stable sort: quality×coverage DESC, descriptionString ASC |
| `collect_vars_at_position` | yes | Distributes variants into reference vs non-reference buckets |
| `is_the_same_variation_on_ref` | yes | Skip check for reference-only positions |
| `collect_reference_variants` | yes | Core finalization: genotype, alleles, MSI, CRISPR, flanking, strand bias |
| `process_variant_finalization` | yes | Per-variant post-branch code extracted from `collect_reference_variants` |
| `update_ref_variant` | yes | Fill reference variant fields for ref-only or pileup update |
| `construct_debug_lines` | yes | Build DEBUG field from accumulated debug lines |
| `find_msi` | yes | Microsatellite instability detection (repeat patterns length 1-6) |
| `proceed_vref_is_deletion` | yes | MSI/shift3 computation for deletion variants |
| `proceed_vref_is_insertion` | yes | MSI/shift3 computation for insertion variants |
| `validate_refallele` | yes | Replace IUPAC ambiguity codes with standard bases |
| `iupac_replacement` | yes | IUPAC code → standard base lookup |
| `chromosome_limit` | yes | Non-mutating chrLengths lookup (replaces Java getOrElse) |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `process(Scope<RealignedVariationData>)` | `process(...)` | Args destructured from scope |
| `createVariant(...)` | `create_variant(...)` | T12: `extracnt > 0` (not `!= 0`) |
| `createInsertion(...)` | `create_insertion(...)` | T12: `extracnt != 0`; T24: cross-pos mutation; T25: returns totalPosCoverage |
| `calcHicov(...)` | `calc_hicov(...)` | Insertion hicov intentionally excluded (Java commented out) |
| `adjustVariantCounts(...)` | `adjust_variant_counts(...)` | eprintln for parity |
| `sortVariants(...)` | `sort_variants(...)` | T7: stable sort, partial_cmp |
| `collectVarsAtPosition(...)` | `collect_vars_at_position(...)` | T8: String.valueOf(null) → "null" |
| `isTheSameVariationOnRef(...)` | `is_the_same_variation_on_ref(...)` | T27: synthetic "I" key |
| `collectReferenceVariants(...)` | `collect_reference_variants(...)` + `process_variant_finalization(...)` | T9,T11,T13,T15-19,T25-26,T29-36 |
| `updateRefVariant(...)` | `update_ref_variant(...)` | T33,T36 |
| `constructDebugLines(...)` | `construct_debug_lines(...)` | |
| `findMSI(...)` | `find_msi(...)` | T14,T20,T21,T22 |
| `proceedVrefIsDeletion(...)` | `proceed_vref_is_deletion(...)` | T23: unwrap_or(0) |
| `proceedVrefIsInsertion(...)` | `proceed_vref_is_insertion(...)` | T23: unwrap_or(0) |
| `validateRefallele(...)` | `validate_refallele(...)` | IUPAC lookup |
| `chrLengths.getOrElse(chr, 0)` | `chromosome_limit()` | Non-mutating; T23 mitigated |

## Known Parity Traps

- **T3**: `locnt != 0 ? locnt : 0.5` divisor — prevents division by zero
- **T4/T5**: Keys sorted lexicographically within position (`sort()`)
- **T7**: Stable sort with `partial_cmp().unwrap_or(Equal)` — NaN sort difference is theoretical
- **T8**: `ref.get(position)` null → `Option<u8>`; `String.valueOf(null)` → `"null"`
- **T9**: `referenceVariant` can be None — gracefully returns empty string genotype
- **T12**: `extracnt > 0` in createVariant vs `extracnt != 0` in createInsertion
- **T14**: `msint` field = length of MSI unit string, not the string itself
- **T20**: Regex compiled fresh each iteration in `find_msi` (matches Java)
- **T21**: `shift3` preserved from first `find_msi` call, not overwritten by second
- **T22**: `msi <= shift3/dellen` uses `<=` (not `<`)
- **T23**: `chrLengths.getOrElse` → non-mutating `unwrap_or(0)`
- **T24**: `createInsertion` mutates `nonInsertionVariants[position+1]` fwd/rev counts
- **T25**: `totalPosCoverage` returned from `createInsertion`, locally mutated in `&`+`<DEL>` branch
- **T27**: Synthetic `"I"` key inserted to prevent skipping insertion positions
- **T28**: `catch_unwind` per position for error-and-continue semantics
- **T33**: `strandBiasFlag` format: `"refBias;varBias"` — semicolon check before append
- **T35**: Deletion `startPosition--` includes preceding base in alleles
- **T36**: `updateRefVariant` sets `leftseq`/`rightseq` to empty strings
