# variation_realigner

**Source**: `src/mods/variation_realigner.rs` (5,250 LOC)
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/modules/VariationRealigner.java`
**Status**: complete

## Overview

VariationRealigner is the second pipeline stage after CigarParser. It takes raw variant maps (insertions, non-insertions, soft clips, SV structures) and performs local realignment: SV filtering, MNP adjustment, short indel absorption, and large indel discovery from soft clips. The module mutates shared variant maps in place throughout. Ported as standalone free functions (no `struct VariationRealigner`) to satisfy the borrow checker while passing `&mut HashMap` parameters for each shared mutable map.

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `process()` | yes | Entry point: init → filterAllSV → adjustMNP → realignIndels → build output |
| `fill_and_sort_tmp()` | yes | Flatten + 3-key sort (desc count, asc pos, desc description) |
| `find35match()` | yes | Sliding-window overlap scan, returns FIRST qualifying match |
| `ismatch()` / `ismatch_with_mm()` | yes | Sequence comparison with mismatch threshold |
| `islowcomplexseq()` | yes | Low-complexity filter (< 3 distinct bases) |
| `count_char()` | yes | Character occurrence count |
| `ismatchref()` / `ismatchref_with_mm()` | yes | Compare sequence to reference with mismatches |
| `adj_ins_pos()` | yes | Left-align insertion position |
| `adj_ref_cnt()` / `adj_ref_factor()` | yes | Factor-based reference count adjustment |
| `add_var_factor()` | yes | Variant count amplification |
| `rm_cnt()` | yes | Count subtraction |
| `find_mm5()` / `find_mm3()` | yes | 5'/3' mismatch walk with soft-clip used-marking side effect |
| `findbi()` | yes | Insertion breakpoint finder |
| `findbp()` | yes | Deletion breakpoint finder |
| `no_passing_reads()` | yes | BAM query for spanning reads |
| `filter_sv()` / `check_cluster()` | yes | SV filtering and mate clustering |
| `filter_all_sv_structures()` | yes | Orchestrate SV filtering + SOFTP2SV sort |
| `adjust_mnp()` | yes | MNP merging with asymmetric varsCount checks |
| `realigndel()` | yes | Two-pass short deletion absorption |
| `realignins()` | yes | Two-pass short insertion absorption, NEWINS rename |
| `realignlgdel()` | yes | Large deletion discovery (5'/3' asymmetric passes) |
| `realignlgins30()` | yes | Large insertion from paired 5'/3' soft clips |
| `realignlgins()` | yes | Large insertion from single soft clips |
| `realign_indels()` | yes | Sequences the 5 realignment sub-procedures |
| `get_sv()` / `remove_sv()` | yes | SV metadata get-or-create / remove helpers |
| `partial_pipeline_stub()` | yes | No-op stub for re-entrant CigarParser (S17 integration) |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `VariationRealigner.process()` | `process()` | `initFromScope()` inlined into `process()` |
| `VariationRealigner.fillAndSortTmp()` | `fill_and_sort_tmp()` | Generic over map value type |
| `VariationRealigner.realigndel()` | `realigndel()` | `bams` null→Option shadowing preserved |
| `VariationRealigner.realignins()` | `realignins()` | Returns NEWINS String |
| `VariationRealigner.realignlgdel()` | `realignlgdel()` | partialPipeline stubbed |
| `VariationRealigner.realignlgins30()` | `realignlgins30()` | Recursive realigndel/realignins calls |
| `VariationRealigner.realignlgins()` | `realignlgins()` | partialPipeline stubbed |
| Inner class `SortPositionDescription` | `SortPositionDescription` struct | |
| Inner class `MismatchResult` | `MismatchResult` struct | |
| Inner class `Mismatch` | `Mismatch` struct | |

## Known Parity Traps

1. **fillAndSortTmp sort order** — 3-key: desc count, asc position, desc description. Rust `String::cmp` matches Java `String.compareTo` for ASCII.
2. **bams parameter shadowing in realigndel** — Java tests null-ness of `bamsParameter`, never uses its value. Rust: `Option<&[String]>`.
3. **Asymmetric varsCount in adjustMNP** — Left: `<= 0`, Right: `< 0`.
4. **findMM3/findMM5 side effects** — Mark `soft_clips_{3,5}_end[pos].used = true` during mismatch walk.
5. **find35match returns FIRST qualifying match** — Do not optimize to find "best."
6. **Recursive calls with singleton maps** — Outer `tmp` snapshot is not invalidated.
7. **findMM3 vs findMM5 mismatch loop bound** — findMM3: `<= longmm` (4 iters), findMM5: `< longmm` (3 iters).
8. **ismatch direction formula** — `seq2[dir*n - (dir==-1 ? 1 : 0)]`.
9. **realignlgdel 3' meanPosition adjustment** — 3' pass adds `dellen * varsCount` before adjCnt; 5' does not.
10. **Pass-2 reverse loop skips element 0** — `for i in (1..tmp.len()).rev()`.
11. **partialPipeline stub** — 5 call sites are no-op. Will be wired when S17 pipeline integration is complete.
12. **SV list cloning** — `to_vec()` clones prevent `used` flag mutation from propagating back to `sv_structures`. Borrow-checker workaround; does not affect Tier 1 parity.

## Architectural Notes

- **No struct**: Entire module is free functions with explicit `&mut` parameters for each shared map. This avoids `&mut self` borrow conflicts.
- **COMP2 / COMP3 sorts**: Two separate sort closures for soft-clip position lists (COMP2 for lgdel/lgins, COMP3 for lgins30).
- **Cross-module stubs**: `find_match()`, `mark_sv()`, `mark_dup_sv()` call into `structural_variants_processor` (S17). `partial_pipeline_stub()` is a local no-op.
