# variations

**Source**: `src/variations.rs`
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/variations/VariationUtils.java`
**Status**: complete

## Overview

Static utility functions for manipulating `Variation` objects and related data structures. Contains counter incrementing, strand bias calculation, soft-clip consensus finding, reference sequence building, count adjustment/correction, variant lookup/creation, and null-safe comparison helpers. Used by CigarParser, VariationRealigner, StructuralVariantsProcessor, and ToVarsBuilder.

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `inc_cnt` | yes | Increment counter in map by amount (generic over HashMap/BTreeMap via CountMap trait) |
| `strand_bias` | yes | Calculate strand bias flag (0, 1, or 2) from forward/reverse counts |
| `find_conseq` | yes | Build consensus sequence from soft-clip data; checks poly-A/T, low-complexity, adaptors |
| `join_ref` | yes | Build reference string from position-to-base map (inclusive end) |
| `join_ref_f64` | yes | Same but with f64 upper bound (exclusive) |
| `join_ref_for_5_lgins` | yes | Reference with 5' large insertion consensus overlay |
| `join_ref_for_3_lgins` | yes | Reference with 3' large insertion consensus overlay |
| `adj_cnt` / `adj_cnt_with_reference` | yes | Add variant counts to target; optionally subtract from reference + correctCnt |
| `correct_cnt` | yes | Clamp negative fields to zero after subtraction |
| `get_var_maybe` / `get_var_maybe_from_vars` | yes | Get variant from Vars by type selector (Var/Varn/Ref) |
| `get_or_put_vars` | yes | Get-or-create Vars at position in map |
| `get_variation_from_seq` | yes | Get-or-create Variation in Sclip's seq double-map |
| `get_variation` | yes | Get-or-create Variation in main hash |
| `get_variation_maybe` | yes | Nullable lookup by position and ref base |
| `is_has_and_equals_*` | yes | Null-safe comparison helpers (3 overloads) |
| `is_has_and_not_equals_*` | yes | Null-safe inequality helpers (2 overloads) |
| `is_equals` / `is_not_equals` | yes | Null-safe Option<u8> equality |
| `is_low_complex_seq` | yes | Inlined low-complexity sequence check (private) |
| `configure_variation_utils_scope` | yes | Set global scope (conf, adaptor maps) |
| `clear_variation_utils_scope` | yes | Reset global scope to defaults |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `VariationUtils.incCnt(Map, Object, int)` | `inc_cnt<K, M: CountMap<K>>()` | Generic via `CountMap` trait for HashMap/BTreeMap |
| `VariationUtils.strandBias(int, int)` | `strand_bias(i32, i32) -> i32` | Reads `conf.bias`, `conf.min_bias_reads` from global scope |
| `VariationUtils.findconseq(Sclip, int)` | `find_conseq(&mut Sclip, i32) -> String` | Identical algorithm; adaptor maps from global scope |
| `VariationUtils.joinRef(Map, int, int)` | `join_ref(&HashMap<i32, u8>, i32, i32)` | `Character` → `u8` |
| `VariationUtils.joinRef(Map, int, double)` | `join_ref_f64(...)` | Exclusive upper bound via float |
| `VariationUtils.adjCnt(Variation, Variation)` | `adj_cnt(&mut Variation, &Variation)` | 2-arg delegates to 3-arg with None |
| `VariationUtils.adjCnt(Variation, Variation, Variation)` | `adj_cnt_with_reference(..., Option<&mut Variation>)` | Nullable ref → Option |
| `VariationUtils.correctCnt(Variation)` | `correct_cnt(&mut Variation)` | Dir clamp via `add_dir(dir, -get_dir(dir))` |
| `VariationUtils.getVarMaybe(Vars, VarsType, Object...)` | `get_var_maybe_from_vars(&Vars, VarsType, VarMaybeArg)` | Varargs → enum `VarMaybeArg` |
| `VariationUtils.getVariation(hash, int, String)` | `get_variation(&mut HashMap, i32, impl Into<String>)` | Returns `&mut Variation` |
| `VariationUtils.getVariationMaybe(hash, int, Character)` | `get_variation_maybe(&HashMap, i32, Option<u8>)` | Nullable char → `Option<u8>` |
| `VariationUtils.isEquals(Character, Character)` | `is_equals(Option<u8>, Option<u8>)` | `Option` equality handles None==None |
| `GlobalReadOnlyScope.instance()` fields | `Lazy<RwLock<VariationUtilsScope>>` | Thread-safe global; tests use `TEST_SCOPE_LOCK` |

## Known Parity Traps

- **`getVarMaybe` fall-through**: Java `case var:` falls through to `case varn:` when index is OOB. The cross-type HashMap miss always returns null. Rust `match` returns `None` directly — behaviorally equivalent.
- **`extracnt += variant.varsCount`**: Intentionally uses `varsCount`, NOT `extracnt`. Tracks realignment-sourced counts.
- **`correctCnt` directional clamp**: Uses `addDir(dir, -getDir(dir))` rather than direct assignment. Equivalent to zeroing.
- **`findconseq` quality tie-breaking**: When `currentCount == maxCount`, the choice is overridden if `seq` has a higher quality entry for the current base. Both Java and Rust check `is_some() && quality > maxQuality`.
- **`is_low_complex_seq` inlined**: Java calls `VariationRealigner.islowcomplexseq()`. Rust inlines the algorithm locally. Must stay in sync if VariationRealigner version is ported separately.
- **Scope clone on read**: `current_scope()` clones the full `VariationUtilsScope`. Acceptable for non-hot-path usage.

## Divergences

- None noted. All methods match Java behavior faithfully.

## Cross-Module Dependencies

- **Reads from**: `crate::config::Configuration` (bias, min_bias_reads, y), `crate::config::{ADSEED, SEED_2}`, `crate::patterns::{B_A7, B_T7}`
- **Operates on**: `crate::data::{Variation, Sclip, VariationMap, Variant, Vars}`
- **Called by**: CigarParser, VariationRealigner, StructuralVariantsProcessor, ToVarsBuilder (all pipeline modules)
- **Utilities used**: `crate::utils::{reverse_sequence, substr_with_len}`
