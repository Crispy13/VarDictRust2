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
| SAMFileParser | `src/mods/sam_file_parser.rs` | MEDIUM | [sam_file_parser.md](sam_file_parser.md) | complete |
| CigarModifier | `src/data.rs` | MEDIUM | [cigar_modifier.md](cigar_modifier.md) | not started |
| CigarParser | `src/mods/cigar_parser.rs` | HIGH | [cigar_parser.md](cigar_parser.md) | partial |
| VariationRealigner | `src/mods/variation_realigner.rs` | HIGH | [variation_realigner.md](variation_realigner.md) | complete |
| StructuralVariantsProcessor | `src/mods/structural_variants_processor.rs` | HIGH | [structural_variants_processor.md](structural_variants_processor.md) | complete |
| ToVarsBuilder | `src/mods/to_vars_builder.rs` | HIGH | [to_vars_builder.md](to_vars_builder.md) | complete |
| PostProcessModules | `src/mods/simple_post_process.rs`<br>`src/mods/somatic_post_process.rs`<br>`src/mods/amplicon_post_process.rs` | HIGH | [tsv_output_layer.md](tsv_output_layer.md) | complete |
| OutputVariant Printers | `src/mods/output.rs`<br>`src/scope.rs` | MEDIUM | [tsv_output_layer.md](tsv_output_layer.md) | complete |
| TSV Output Mode Orchestrators | `src/modes.rs` | MEDIUM | [tsv_output_layer.md](tsv_output_layer.md) | complete |
| FisherExact | `src/fisher.rs` | HIGH | [fisher_exact.md](fisher_exact.md) | not started |
| ScopeData | `src/scope.rs`<br>`src/data.rs` | LOW | [scope_data.md](scope_data.md) | complete |
| VariationMap and Collections | `src/data.rs` | LOW | [variation_map.md](variation_map.md) | not started |
| Variations | `src/variations.rs` | MEDIUM | [variations.md](variations.md) | complete |
| Utils | `src/utils.rs`<br>`src/patterns.rs` | LOW | [utils.md](utils.md) | not started |

## Test and Workflow Harness Index

| Harness | Primary Files | Risk | Cache File | Status |
|---------|---------------|------|------------|--------|
| Full-BAM E2E Sweep Harness | `tests/parity_e2e_sweep.rs`<br>`tests/parity_e2e_sweep/common.rs`<br>`scripts/e2e_sweep_gate.py` | MEDIUM | [parity_e2e_sweep_harness.md](parity_e2e_sweep_harness.md) | partial |

## Coverage Summary

- Rust cache docs in this index: 8 module docs (cigar_parser.md, sam_file_parser.md, scope_data.md, structural_variants_processor.md, to_vars_builder.md, tsv_output_layer.md, variation_realigner.md, variations.md) plus 1 harness doc (parity_e2e_sweep_harness.md), placeholder index for remaining modules
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