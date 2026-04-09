# VarDict-rs Codebase Cache

> Placeholder Rust codebase index for progressive cache docs.
> Update this file as Rust module documentation is added or refined.
> Keep Rust docs architecture-focused so they remain useful as the implementation evolves.

## Rust Source Root

```
src/
```

## Pipeline Flow

```
CLI/config setup
  → region/reference preparation
  → BAM/read processing
  → parsing and realignment stages
  → variation assembly
  → post-processing / parity formatting
  → output
```

## Module Index

Rows below mirror the Java cache structure so Java↔Rust correspondence can be tracked as Rust docs are added.

| Module | Primary Rust Files | Risk | Cache File | Status |
|--------|--------------------|------|------------|--------|
| EntryPoint | `src/lib.rs` | LOW | [entry_point.md](entry_point.md) | not started |
| Configuration and CLI | `src/config.rs` | LOW | [configuration.md](configuration.md) | not started |
| RegionBuilder | `src/reference.rs` | MEDIUM | [region_builder.md](region_builder.md) | not started |
| Modes | `src/scope.rs` | LOW | [modes.md](modes.md) | not started |
| SAMFileParser | `src/data.rs` | MEDIUM | [sam_file_parser.md](sam_file_parser.md) | not started |
| CigarModifier | `src/data.rs` | MEDIUM | [cigar_modifier.md](cigar_modifier.md) | not started |
| CigarParser | `src/data.rs` | HIGH | [cigar_parser.md](cigar_parser.md) | not started |
| VariationRealigner | `src/variations.rs` | HIGH | [variation_realigner.md](variation_realigner.md) | not started |
| StructuralVariantsProcessor | `src/variations.rs` | HIGH | [structural_variants_processor.md](structural_variants_processor.md) | not started |
| ToVarsBuilder | `src/variations.rs` | MEDIUM | [to_vars_builder.md](to_vars_builder.md) | not started |
| PostProcessModules | `src/parity/format.rs` | HIGH | [post_process_modules.md](post_process_modules.md) | not started |
| OutputVariant Printers | `src/parity/format.rs` | MEDIUM | [output_variant.md](output_variant.md) | not started |
| FisherExact | `src/fisher.rs` | HIGH | [fisher_exact.md](fisher_exact.md) | not started |
| ScopeData | `src/scope.rs`<br>`src/data.rs` | LOW | [scope_data.md](scope_data.md) | not started |
| VariationMap and Collections | `src/data.rs` | LOW | [variation_map.md](variation_map.md) | not started |
| Variations | `src/variations.rs` | MEDIUM | [variations.md](variations.md) | complete |
| Utils | `src/utils.rs`<br>`src/patterns.rs` | LOW | [utils.md](utils.md) | not started |

## Coverage Summary

- Rust cache docs in this index: 1 module doc (variations.md), placeholder index for remaining modules
- Expected workflow: create per-module docs as Review Gate updates Rust cache coverage after approved reviews

## Per-Module File Template

When creating a Rust module doc for the first time, use this structure:

```markdown
# module_name

**Source**: `src/path/to/module.rs`
**Java counterpart**: `VarDictJava/...`
**Status**: partial | complete

## Overview
One-paragraph summary of the module's role in the Rust pipeline.

## Method Inventory
| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| function_name | yes/no | one-liner |

## Java↔Rust Correspondence
| Java | Rust | Notes |
|------|------|-------|

## Known Parity Traps
- Ordering, formatting, nullability, sentinel values, or other parity-sensitive behavior.

## Divergences
- None noted yet.

## Cross-Module Dependencies
- Calls: downstream/upstream relationships that matter for parity.
```