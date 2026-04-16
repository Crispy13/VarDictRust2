# tsv_output_layer

**Source**: `src/mods/output.rs`, `src/mods/simple_post_process.rs`, `src/mods/somatic_post_process.rs`, `src/mods/amplicon_post_process.rs`, `src/modes.rs`, `src/utils.rs`, `src/scope.rs`
**Java counterpart**: `VarDictJava/src/main/java/com/astrazeneca/vardict/printers/*.java`, `.../postprocessmodules/*.java`, `.../modes/*.java`, `.../Utils.java`
**Status**: complete (with two documented accepted deviations)

## Overview
The TSV output layer is the cold tail of the pipeline. It consumes the finalized
`AlignedVarsData` produced by `to_vars_builder` and emits one or more TSV rows per variant
position using the three Java-compatible output variants (Simple, Somatic, Amplicon) and
their four column widths (Simple 36/38, Somatic 55/61, Amplicon 38/40 + debug suffix).
`src/modes.rs` drives sequential region iteration with Java-matching header printing.

## Method Inventory
| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `SimpleOutputVariant::new` + `simple_variant_36columns` / `simple_variant_38columns` / `to_tsv_line` | yes | Simple-mode row construction, non-Fisher and Fisher paths, CRISPR/debug suffix |
| `SomaticOutputVariant::new` + `calculate_fisher_somatic` + `somatic_variant_55columns` / `somatic_variant_61columns` / `to_tsv_line` | yes | Somatic-mode row construction with 3-branch Fisher logic, tumor/normal placeholder semantics |
| `AmpliconOutputVariant::new` + `amplicon_variant_38columns` / `amplicon_variant_40columns` / `debug_amp_variant` / `to_tsv_line` | yes | Amplicon-mode row construction including AMPBIAS debug fields |
| `VariantPrinter::print_line`, `VariantPrinter::from(PrinterType)` | yes | stdout/stderr line emission, sequential only |
| `simple_post_process` | yes | Per-position dispatch, `is_good_var` filtering, `adj_complex` + CRISPR zeroing, per-position `catch_unwind` recovery |
| `somatic_post_process`, `calling_for_one_sample`, `calling_for_both_samples`, `print_variations_from_first_sample`, `print_variations_from_second_sample`, `determinate_type`, `combine_analysis` | partial | Core dispatch and sorted union iteration match Java; `combine_analysis` intentionally stubbed pending re-entrant mode pipeline |
| `amplicon_post_process`, `count_variant_on_amplicons`, `fill_vref_list`, `is_amp_bias_flag` | yes | `IndexMap` preserves Java `LinkedHashMap` order, AMPBIAS logic, overlap decrement |
| `SimpleMode::not_parallel` / `SomaticMode::not_parallel` / `AmpliconMode::not_parallel` + `print_header` | partial | Sequential region iteration and headers match Java; parallel executor/queue not ported |
| `tsv_join!`, `format_half_even`, `zero_gated_format`, `nm_fisher_format`, `nm_non_fisher_format`, `hifreq_fisher_format` | yes | Heterogeneous join macro + four distinct formatting strategies |

## Java↔Rust Correspondence
| Java | Rust | Notes |
|------|------|-------|
| `LinkedHashMap<Integer, List<Variant>>` | `IndexMap<i32, Vec<Variant>>` | Amplicon `goodVariantsOnAmp` insertion-order preservation |
| `BigDecimal.setScale(n, HALF_EVEN)` | `format_half_even(pattern, value)` | Explicit 0.5-tie handling at 3+ decimals |
| `Utils.join(delim, obj...)` | `tsv_join!(delim, …)` macro | Heterogeneous values via `fmt::Display` |
| `try { … } catch (Exception e) { printExceptionAndContinue(…) }` | `panic::catch_unwind(AssertUnwindSafe(|| { … }))` + `eprintln!` | Per-position fault isolation in all three post-process modules |
| `PrintStream.println` | `VariantPrinter::print_line` locking stdout/stderr | Matches Java contention profile |
| `stopVardictWithException(…)` | `panic!(...)` in `try_to_get_reference` | Accepted deviation: both terminate the run |
| `variantPrinter.print(OutputStream)` + executor plumbing | — | Accepted deviation: parallel layer not ported |
| `getMode().pipeline(...)` re-entry for combine analysis | stubbed `CombineAnalysisData { max_read_length, type_: "" }` | Accepted deviation: requires re-entrant mode interface |

## Known Parity Traps
- **Three formatting strategies must not be mixed**: zero-gated non-Fisher, Fisher-rounded via `get_rounded_value_to_print`, and raw zero-gated for `nm` / `hifreq`. Each column hard-codes its strategy; do not refactor behind a single helper.
- **Somatic `is_noise(&mut self)`** mutates the variant payload before `determinate_type` emits output; `determinate_type` preserves that mutation ordering.
- **`genotype` null → `"0"`** and empty flank / sv string coercions are applied in the output variant constructors, not at print time.
- **Sorted position iteration** is explicit in all three post-process modules — the underlying maps do not guarantee order.
- **Amplicon `IndexMap`** insertion order is load-bearing for `AMPBIAS` descriptions.
- **`AssertUnwindSafe`** is valid because each per-position closure owns its mutable state except for the lock-protected output stream.

## Divergences
- `combine_analysis()` is stubbed. Blocks faithful LikelySomatic/Germline transition in somatic mode when re-entry into alignment would be required. Tracked as follow-up.
- Parallel executor / queue layer not ported. Current modes run sequentially.
- `try_to_get_reference` panics on failure rather than calling an equivalent of `stopVardictWithException`. Both terminate; stderr wording differs.

## Tests
Per-module TSV parity harness not present by design — TSV output parity is exercised at the
E2E sweep gate. Library tests (259 passed) and `parity_cigar_modifier` pass post-change.
