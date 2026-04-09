# scope_data

**Source**: `src/scope.rs`, `src/data.rs` (Variant impl block)
**Java counterpart**: `data/scopedata/Scope.java`, `data/scopedata/GlobalReadOnlyScope.java`, `variations/Variant.java` (methods only)
**Status**: complete

## Overview

Pipeline infrastructure module providing two state containers (`Scope<T>`, `GlobalReadOnlyScope`) and four behavior methods on `Variant`. `Scope<T>` carries common context (BAM path, region, reference, splice set, printer) plus type-parameterized stage data through pipeline transitions. `GlobalReadOnlyScope` is a write-once singleton holding configuration, chromosome lengths, sample metadata, and adaptor maps accessed by every pipeline module. The four Variant methods (`is_noise`, `adj_complex`, `var_type`, `is_good_var`) implement noise gating, allele trimming, type classification, and multi-criteria quality filtering.

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `Scope::new()` | yes | Full 8-param constructor, wraps shared fields in Arc |
| `Scope::with_data()` | yes | Stage transition: moves common fields, replaces data payload |
| `GlobalReadOnlyScope::new()` | yes | Internal constructor; derives printer_type_out from conf |
| `GlobalReadOnlyScope::init()` | yes | One-shot init; panics on double-init |
| `GlobalReadOnlyScope::instance()` | yes | Returns cloned snapshot from RwLock |
| `GlobalReadOnlyScope::set_mode()` | yes | One-shot mode assignment via SharedMode (Arc) |
| `GlobalReadOnlyScope::get_mode()` | yes | Returns Option\<SharedMode\> |
| `GlobalReadOnlyScope::clear()` | yes | Test-only reset of instance and mode |
| `VariantPrinter` enum | yes | Placeholder: Out/Err variants, From\<PrinterType\> |
| `AbstractMode` trait | yes | Empty trait + SharedMode type alias |
| `Variant::is_noise()` | yes | Noise gate: mutates self (zeroes counts) if criteria met |
| `Variant::adj_complex()` | yes | Prefix/suffix allele trimming using substr helpers |
| `Variant::var_type()` | yes | Allele classification: SNV/Insertion/Deletion/Complex/SV |
| `Variant::is_good_var()` | yes | Multi-criteria quality filter (10+ stages) || `InitialData` | yes | 5 HashMap fields for variants/coverage/softclips; Default impl |
| `CombineAnalysisData` | yes | max_read_length + type_ data holder for combine analysis |
## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `Scope<T>` (8 fields, 2 ctors) | `Scope<T>` (8 fields, `new()` + `with_data()`) | Shared refs via Arc |
| `Scope(Scope<?>, T)` inherit ctor | `Scope::with_data<U>(self, data) -> Scope<U>` | Consumes old scope |
| `GlobalReadOnlyScope` (volatile static + synchronized) | `Lazy<RwLock<Option<GlobalReadOnlyScope>>>` | Supports test clear() |
| `GlobalReadOnlyScope.mode` (volatile static) | `Lazy<RwLock<Option<SharedMode>>>` | Separate static |
| `instance().conf.goodq` in is_noise/is_good_var | `conf: &Configuration` param | Decoupled for testability |
| `VariantPrinter` (abstract class) | `VariantPrinter` enum (Out/Err) | Placeholder for Phase 1 |
| `AbstractMode` (abstract class) | `AbstractMode` trait + SharedMode | Placeholder for Phase 1 |
| `Variant.isNoise()` | `Variant::is_noise(&mut self, &Configuration)` | wrapping_sub for overflow |
| `Variant.adjComplex()` | `Variant::adj_complex(&mut self)` | Uses substr/substr_with_len |
| `Variant.varType()` | `Variant::var_type(&self) -> String` | Uses ANY_SV regex |
| `Variant.isGoodVar(Variant, String, Set)` | `Variant::is_good_var(&self, Option<&Variant>, Option<&str>, &HashSet, &Configuration)` | Explicit conf param |
| `InitialData` (5 HashMap fields) | `InitialData` in `src/data.rs` (L323) | HashMap outer maps, VariationMap inner; Default → 5 empty maps |
| `CombineAnalysisData` (int + String) | `CombineAnalysisData` in `src/data.rs` (L948) | `type` → `type_` keyword rename |
| `Set<String> splice` in Scope | `Arc<HashSet<String>>` | HashSet for .contains() only |
| `samplem: String` (nullable) | `samplem: Option<String>` | Nullable semantics preserved |
| `ampliconBasedCalling: String` (nullable) | `amplicon_based_calling: Option<String>` | Nullable semantics preserved |

## Known Parity Traps

1. **`is_noise()` mutates self** — Returns bool AND zeroes 5 fields. Must be `&mut self` and callers must not separate the check from the mutation.
2. **`is_noise()` integer overflow** — `total_pos_coverage -= position_coverage` uses `wrapping_sub()` to match Java's silent wraparound.
3. **`adj_complex()` negative-index substr** — Suffix trimming uses `substr_with_len(bytes, -n, 1)` and `substr_with_len(bytes, 0, 1 - n)`. These negative indices are handled by the `substr` helpers ported from Java Utils.
4. **`var_type()` ordering** — The 8-branch classification must follow Java's exact sequence: ref==var → SNV → ANY_SV → empty → first chars differ → Insertion → Deletion → Complex.
5. **`is_good_var()` dead code drop** — Java has `this == null` (unreachable) and `type == null` at the strand bias block (unreachable after type resolution). Both safely omitted in Rust.
6. **`GlobalReadOnlyScope::instance()` clones** — Returns full struct clone due to RwLock approach. Callers should cache locally to avoid repeated allocations.
7. **Scope.splice is HashSet** — Not BTreeSet. Used only for `.contains()` lookups, ordering irrelevant.

## Divergences

- `GlobalReadOnlyScope::instance()` returns owned clone (Java returns shared reference). Necessary for RwLock-based resettable design.
- Variant methods accept `&Configuration` as parameter (Java reads `instance().conf` internally). Improves testability without changing call-site semantics.
- `SharedMode` uses `Arc` (Java uses direct reference). Allows cloning the mode reference.

## Cross-Module Dependencies

- **Upstream**: `Configuration` from `src/config.rs`, `Reference`/`ReferenceResource` from `src/reference.rs`, `Region` from `src/data.rs`, `PrinterType` from `src/config.rs`
- **Downstream consumers**: `Scope<T>` consumed by all pipeline stages; `GlobalReadOnlyScope` read by CigarParser, ToVarsBuilder, Modes, etc.; Variant methods called by ToVarsBuilder and PostProcessModules
- **Utility deps**: `substr`/`substr_with_len` from `src/utils.rs`, `ANY_SV` from `src/patterns.rs`
