# cigar_parser

**Source**: `src/mods/cigar_parser.rs`
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/modules/CigarParser.java`
**Status**: partial

## Overview

CigarParser is the core variant detection module. It implements `Module<RecordPreprocessor, VariationData>` — iterating BAM records from a region, parsing each record's CIGAR string operation-by-operation, and populating variant maps (SNPs, insertions, deletions, MNVs, complex variants), coverage, soft-clip structures, splice junctions, and SV accumulators. Every variant VarDict calls flows through this module. It is a hot-path module (per-read frequency).

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `process()` | yes | Record iterator loop, duprate computation, VariationData packaging |
| `round_duplicate_rate()` | yes | Integer HALF_UP rounding for Java-compatible duprate |
| `parse_cigar()` | yes | Main per-record CIGAR parsing loop (M/I/D/S/N/H dispatch) |
| `cleanup_cigar()` | yes | H removal, edge-I→S conversion (NOT called during parsing — see traps) |
| `process_soft_clip()` | no | 5'/3' soft-clip handling, chimeric detection, match-back |
| `process_insertion()` | no | Insertion variant creation, adjInsPos left-alignment |
| `process_deletion()` | no | Deletion variant creation, complex indel building |
| `add_variation_for_matching_part()` | no | SNP/MNV/complex variant creation in M-segments |
| `adj_ins_pos()` | no | Insertion position left-alignment (from VariationRealigner) |
| `prepare_sv_structures_for_analysis()` | no | SV structure population from discordant pairs |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `CigarParser.process()` | `CigarParser::process()` | Record loop + duprate computation |
| `String.format("%.3f", duprate)` then `Double.parseDouble()` | `round_duplicate_rate()` | Integer HALF_UP arithmetic — identical rounding |
| `CigarParser.parseCigar()` | `CigarParser::parse_cigar()` | Main per-record method |
| `isHasAndNotEquals(...) && isNotEquals('N', ref.get(start))` | `is_reference_mismatch_and_not_n(...)` | MNV-growth guard: mismatch and reference base must not be `N` |
| `CigarParser.cleanupCigar(record)` | `CigarParser::cleanup_cigar()` | NOT called during parsing (see trap below) |
| `CigarParser.isReadChimericWithSA()` | `is_read_chimeric_with_sa()` | Compares SA chr to `region.chr` (equivalent on Simple path) |
| `CigarParser.parseCigarWithAmpCase()` | `parse_cigar_with_amp_case()` | Amplicon fallback differs structurally (amplicon mode only) |

## Known Parity Traps

- **T11 — Duprate double round-trip**: Java formats `(double)duplicateReads/totalReads` with `String.format("%.3f", ...)` then parses back via `Double.parseDouble()`. Rust uses `round_duplicate_rate()` with integer HALF_UP arithmetic producing identical values. Design brief #6 technically forbids skipping format/parse, but integer arithmetic was approved as equivalent.
- **Cleanup timing**: Java caches `cigar` as a local variable before calling `cleanupCigar(record)`, which only rewrites the SAMRecord. The parser works against the cached copy. Rust must NOT call `self.cleanup_cigar()` before parsing — it would mutate the working CIGAR, breaking minmatch filtering for edge insertions (3I→3S changes match+insertion length).
- **T12 — SV duprate integer division**: `(discordantCount + 1) / (totalReads - duplicateReads + 1)` uses integer division. Always false for `> 0.5` unless denominator ≤ 1.
- **MNV growth must reject `N` anchors**: Java grows multi-nucleotide mismatches only when the current base mismatches and `ref[start] != 'N'`. Rust must use `is_reference_mismatch_and_not_n(...)` in the `parse_cigar()` MNV while-loop. Missing the non-`N` guard collapsed discrete non-insertion alleles into `G&GAG` and populated `mnp` on `T1-02/hg002/10:116065606-116065839`.
- **SA chromosome comparison**: `is_read_chimeric_with_sa()` compares SA chr to `region.chr` instead of `record.getReferenceName()`. Equivalent on Simple-mode path (RecordPreprocessor fetches only from region.chr). May diverge in multi-chromosome scenarios.
- **Amplicon fallback**: `parse_cigar_with_amp_case()` parses malformed amplicon config pieces independently, while Java falls back to defaults together. Only affects amplicon mode with invalid config.
