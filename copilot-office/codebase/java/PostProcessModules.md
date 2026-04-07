# PostProcessModules

**Source**: `postprocessmodules/*.java`
**LOC**: 858
**Rust counterpart**: not yet ported in this workspace
**Status**: complete

## Overview

The post-process modules are the last Java-side decision layer between `ToVarsBuilder` and the tabular output printers. They do not discover variants; they decide which `Variant` objects survive to output, how reference-only or no-coverage positions are represented in pileup mode, how paired-sample labels are assigned, and how amplicon-local evidence is aggregated into a single output row. In pipeline terms: `Mode -> SAMFileParser/realignment/SV/toVars -> PostProcessModule -> OutputVariant printer`. Their behavior is parity-critical because they mutate `Variant` objects in-place (`adjComplex()`, `isNoise()` side effects, combine-analysis subtraction), choose which placeholder objects are passed into the printer constructors, and therefore indirectly control the Fisher test values emitted by the printer layer.

## Pipeline Role By Class

### SimplePostProcessModule

Consumes one `Scope<AlignedVarsData>` and emits one `SimpleOutputVariant` row for each accepted output candidate. It handles pileup-only reference rows, null/no-coverage skeleton rows, complex variant normalization, and CRISPR field zeroing.

### SomaticPostProcessModule

Consumes two aligned-variant scopes and performs paired-sample reconciliation. It assigns labels such as `StrongSomatic`, `LikelySomatic`, `Germline`, `LikelyLOH`, `StrongLOH`, `AFDiff`, `SampleSpecific`, and `Deletion`, optionally re-runs the calling pipeline on combined BAMs for indel rescue, and selects the exact tumor/normal placeholder variants that drive the downstream Fisher outputs.

### AmpliconPostProcessModule

Consumes per-amplicon `Vars` maps plus the amplicon layout for each position. It merges good variants across amplicons, deduplicates allele-identical calls, marks low-coverage amplicons, derives the `AMPBIAS` flag when amplicons disagree, and packages both supporting and non-supporting amplicon evidence into `AmpliconOutputVariant`.

## Method Inventory

### SimplePostProcessModule

| Method | Visibility | Lines | Analyzed? | Summary |
|--------|------------|-------|-----------|---------|
| `SimplePostProcessModule(VariantPrinter)` | public | L25-L27 | yes | Stores the printer used for every emitted row. |
| `accept(Scope<AlignedVarsData>)` | public | L33-L107 | yes | Iterates aligned positions, filters/normalizes variants, and prints `SimpleOutputVariant` rows. |

### SomaticPostProcessModule

| Method | Visibility | Lines | Analyzed? | Summary |
|--------|------------|-------|-----------|---------|
| `SomaticPostProcessModule(ReferenceResource, VariantPrinter)` | public | L47-L50 | yes | Stores reference access and output printer dependencies. |
| `accept(Scope<AlignedVarsData>, Scope<AlignedVarsData>)` | public | L56-L93 | yes | Unions both position sets, branches into one-sample or paired reconciliation, and catches per-position failures. |
| `callingForOneSample(Vars, boolean, String, Region, Set<String>)` | package-private | L103-L128 | yes | Emits rows when only one BAM has coverage at a position. |
| `callingForBothSamples(Integer, Vars, Vars, Region, Set<String>)` | package-private | L138-L147 | yes | Chooses whether BAM1-driven or BAM2-driven paired comparison should run. |
| `printVariationsFromFirstSample(Integer, Vars, Vars, Region, Set<String>)` | private | L156-L275 | yes | Main BAM1-first reconciliation path, including strong somatic, germline, and LOH decisions. |
| `printVariationsFromSecondSample(Integer, Vars, Vars, Region, Set<String>)` | private | L285-L336 | yes | BAM2 fallback path, primarily for strong LOH and combine-analysis rescue. |
| `determinateType(Vars, Variant, Variant, Set<String>)` | package-private | L346-L369 | yes | Maps paired evidence to `LikelyLOH`, `LikelySomatic`, `Germline`, `AFDiff`, or `StrongSomatic`. |
| `combineAnalysis(Variant, Variant, String, int, String, Set<String>, int)` | package-private | L385-L476 | yes | Re-runs the pipeline on combined BAMs to downgrade false somatic indels or recover germline evidence. |

### AmpliconPostProcessModule

| Method | Visibility | Lines | Analyzed? | Summary |
|--------|------------|-------|-----------|---------|
| `process(Region, List<Map<Integer, Vars>>, Map<Integer, List<Tuple2<Integer, Region>>>, Set<String>, VariantPrinter)` | public | L32-L201 | yes | Aggregates per-amplicon calls at each position and prints `AmpliconOutputVariant` rows. |
| `countVariantOnAmplicons(Variant, Map<Integer, List<Variant>>)` | private | L208-L219 | yes | Counts how many amplicons contain the exact ref/alt allele pair. |
| `fillVrefList(List<Tuple2<Variant, String>>, List<Variant>)` | private | L227-L238 | yes | Deduplicates output candidates by allele pair while preserving highest-frequency first occurrence. |
| `isAmpBiasFlag(Map<Integer, List<Variant>>)` | private | L246-L272 | yes | Detects disagreement between amplicons and derives the `AMPBIAS` flag. |

## Method Analyses

### SimplePostProcessModule

#### `SimplePostProcessModule(VariantPrinter)`

1. Save the provided `VariantPrinter` into the instance field.
2. All output rows emitted later through `accept()` go through this printer.

#### `accept(Scope<AlignedVarsData>)`

1. Initialize `lastPosition` for exception reporting.
2. Iterate the `alignedVariants` map entry-by-entry in stored order.
3. For each position, load `Vars variantsOnPosition` and `Configuration conf`.
4. If the position has no structural-variant marker (`sv.isEmpty()`) and falls outside the requested BED region, skip it.
5. Build a local `vrefs` list of variants that should be printed for this position.
6. If `variantsOnPosition.variants` is empty:
   - If pileup mode is disabled, skip the position.
   - If the reference variant is `null`, emit a `SimpleOutputVariant(null, region, sv, position)` skeleton row immediately and continue.
   - Otherwise mark the reference variant as reference output by forcing `vartype = ""`, then push it into `vrefs`.
7. If non-reference variants exist, iterate them in order:
   - Skip candidates whose reference allele contains `N`.
   - If ref and alt alleles are equal, only keep them when pileup mode is enabled.
   - If the variant starts at a different coordinate than the current map key, pileup mode is enabled, and this is the only variant at the site, emit or synthesize a reference row first so the original position still gets output.
   - Compute `vref.vartype = vref.varType()`.
   - Run `isGoodVar(referenceVariant, vartype, splice)`.
   - If the variant is not good and pileup mode is disabled, skip it; otherwise keep it anyway for pileup output.
   - Add the selected object to `vrefs`.
8. Iterate `vrefs` in the same order they were assembled.
9. For any variant whose type is `Complex`, call `adjComplex()` in-place before printing.
10. If CRISPR mode is disabled globally, force `vref.crispr = 0` so the printer does not inherit stale values.
11. Create `SimpleOutputVariant(vref, region, sv, position)` and print it.
12. If any exception occurs at the position, report it via `printExceptionAndContinue()` using the last successful position value.

### SomaticPostProcessModule

#### `SomaticPostProcessModule(ReferenceResource, VariantPrinter)`

1. Save the `ReferenceResource` used by `combineAnalysis()`.
2. Save the `VariantPrinter` used for all paired output rows.

#### `accept(Scope<AlignedVarsData>, Scope<AlignedVarsData>)`

1. Treat the second scope argument as the primary region/splice source and BAM1 variant map.
2. Read `region`, `splice`, and both `alignedVariants` maps.
3. Set `maxReadLength` to the maximum of the two scopes' `maxReadLength` values.
4. Build the union of BAM1 and BAM2 covered positions.
5. Sort the unified position list ascending.
6. For each position inside the requested region:
   - Load BAM1 `Vars v1` and BAM2 `Vars v2`.
   - If both are `null`, skip.
   - If BAM1 is missing, call `callingForOneSample(v2, true, "Deletion", region, splice)`.
   - If BAM2 is missing, call `callingForOneSample(v1, false, "SampleSpecific", region, splice)`.
   - Otherwise call `callingForBothSamples(position, v1, v2, region, splice)`.
7. Catch any exception per position and continue via `printExceptionAndContinue()`.

#### `callingForOneSample(Vars, boolean, String, Region, Set<String>)`

1. Return immediately if the sample has no variant list.
2. Iterate every variant in the surviving sample.
3. Skip pure-reference entries where ref equals alt.
4. Compute `vartype`, then require `isGoodVar(...)`.
5. Normalize complex variants with `adjComplex()`.
6. If `isFirstCover` is true, print `SomaticOutputVariant(variant, variant, null, variant, ...)`.
7. Otherwise print `SomaticOutputVariant(variant, variant, variant, null, ...)`.
8. The caller supplies the label, so this method only filters, normalizes, and fills the missing side with `null`.

#### `callingForBothSamples(Integer, Vars, Vars, Region, Set<String>)`

1. If both samples have empty `variants` lists, return.
2. If BAM1 has at least one variant candidate, run the BAM1-first path `printVariationsFromFirstSample(...)`.
3. Only if BAM1 has no variant candidates at all and BAM2 does, run `printVariationsFromSecondSample(...)`.
4. This means the second-sample path is not symmetric fallback for individual BAM1 failures; it only runs when BAM1's `variants` list is empty.

#### `printVariationsFromFirstSample(Integer, Vars, Vars, Region, Set<String>)`

1. Start at the first BAM1 variant and process a prefix of the list while each current element passes `isGoodVar(...)`.
2. For each BAM1 candidate in that prefix:
   - Skip pure-reference entries.
   - Cache `descriptionString` as `nt` and compute `vartype`.
   - Normalize complex variants with `adjComplex()`.
   - Look for the same description string in BAM2 via `getVarMaybe(v2, varn, nt)`.
3. If BAM2 contains the same description string:
   - Call `determinateType(v2, vref, v2nt, splice)`.
   - Print `SomaticOutputVariant(vref, v2nt, vref, v2nt, ..., type)`.
4. If BAM2 lacks the same description string:
   - Build a placeholder `varForPrint` from BAM2's first variant coverage, BAM2 reference variant, or `null`.
   - Default the label to `StrongSomatic`.
   - If the BAM1 candidate is a non-SNV long/regex-matched indel, create an empty BAM2 placeholder `Variant` and insert it into `v2.varDescriptionStringToVariants` before rescue analysis.
   - If that indel also has `positionCoverage < minr + 3` and is not structural (`!nt.contains("<")`), call `combineAnalysis(...)`.
   - If combine analysis returns `FALSE`, suppress the row entirely.
   - If combine analysis returns a non-empty type string, replace `StrongSomatic` with that new label.
5. Print with one of two payload shapes:
   - `StrongSomatic`: use `varForPrint` as the BAM2-side object so coverage/reference placeholders survive into output and Fisher calculations.
   - Any rescued type: use the mutated `v2nt` from combine analysis.
6. Increment the processed counter after each accepted BAM1 candidate.
7. If zero BAM1 variants were processed from the prefix, inspect BAM2 instead for LOH-style output:
   - Skip BAM2 entries that fail `isGoodVar(...)` or are pure reference.
   - Look up the same description string in BAM1.
   - If BAM1 has the same variant, label it `LikelyLOH` when BAM1 frequency is below `lofreq`, else `Germline`, then print paired output.
   - If BAM1 lacks the variant, synthesize a BAM1 placeholder from BAM1's first variant/reference, derive genotype fallback (`v1var.genotype`, duplicated reference genotype, or `N/N`), and print `StrongLOH`.

#### `printVariationsFromSecondSample(Integer, Vars, Vars, Region, Set<String>)`

1. Iterate every BAM2 variant.
2. Skip pure-reference entries and any candidate that fails `isGoodVar(...)`.
3. Default the label to `StrongLOH`.
4. Ensure BAM1 has a mutable placeholder entry for the BAM2 description string using `computeIfAbsent(...)`, then force its `positionCoverage` to zero.
5. For low-support long/regex-matched non-structural indels in BAM2, call `combineAnalysis(...)` against that BAM1 placeholder.
6. If combine analysis returns `FALSE`, suppress the row.
7. If it returns a non-empty label, override `StrongLOH` and use the mutated BAM1 placeholder as `varForPrint`.
8. Otherwise use BAM1 reference data if available, else `null`, as the comparison-side payload.
9. Normalize complex BAM2 variants with `adjComplex()`.
10. Print `SomaticOutputVariant(v2var, v2var, varForPrint, v2var, ..., type)`.

#### `determinateType(Vars, Variant, Variant, Set<String>)`

1. Ask whether the comparison-side variant is itself good in its own sample.
2. If it is good:
   - If the primary variant frequency is above `1 - lofreq` and the comparison-side frequency is between `0.2` and `0.8`, classify as `LikelyLOH`.
   - Otherwise, if the comparison-side frequency is below `lofreq` or its position coverage is `<= 1`, classify as `LikelySomatic`.
   - Otherwise classify as `Germline`.
3. If the comparison-side variant is not good:
   - If its frequency is below `lofreq` or coverage is `<= 1`, classify as `LikelySomatic`.
   - Otherwise classify as `AFDiff`.
4. As a final override, call `variantToCompare.isNoise()`.
5. If that method returns true and the primary variant type is `SNV`, force `StrongSomatic`.
6. Because `isNoise()` mutates the object in-place, this classification step can also zero or alter downstream output fields.

#### `combineAnalysis(Variant, Variant, String, int, String, Set<String>, int)`

1. Optionally log debug output to stderr when `conf.y` is enabled.
2. If the event spans more than `SVMINLEN`, skip rescue and return `(maxReadLength, "")`.
3. Expand a local rescue `Region` around the candidate using `variant1.startPosition - maxReadLength` through `variant1.endPosition + maxReadLength`.
4. Fetch reference bases for that expanded region from `ReferenceResource`.
5. Build a new `Scope<InitialData>` over the combined BAM label `bam1:bam2` and re-run the current mode pipeline synchronously through `getMode().pipeline(..., new DirectThreadExecutor()).join()`.
6. Update `maxReadLength` from the rerun result.
7. Look up the same description string at the original position in the combined callset.
8. If the combined result lacks the variant, return `(maxReadLength, "FALSE")`.
9. If the combined support exceeds the original sample by at least `minr` reads:
   - Subtract `variant1` counts from the combined result into `variant2` field-by-field.
   - Clamp all subtracted count fields to zero.
   - Recompute mean fields by weighted subtraction when `positionCoverage != 0`, else zero them all.
   - Force `isAtLeastAt2Positions = true` and `hasAtLeast2DiffQualities = true`.
   - If `totalPosCoverage <= 0`, return `FALSE`.
   - Recompute allele frequency.
   - Copy `highQualityToLowQualityRatio` from `variant1` because it cannot be reconstructed accurately.
   - Copy genotype from the combined call.
   - Recompute strand bias flags using `VariationUtils.strandBias(...)` on both reference and variant directional counts.
   - Return `(maxReadLength, "Germline")`.
10. If the combined call has more than two fewer supporting reads than the original sample, return `FALSE` as evidence of unstable calling.
11. Otherwise return `(maxReadLength, "")`, meaning keep the original label.

### Somatic Label Assignment Summary

| Label | Where Assigned | Effective Condition |
|-------|----------------|---------------------|
| `Deletion` | `accept()` -> `callingForOneSample(..., true, ...)` | BAM1 missing entirely at the position, BAM2 has a good variant. |
| `SampleSpecific` | `accept()` -> `callingForOneSample(..., false, ...)` | BAM2 missing entirely at the position, BAM1 has a good variant. |
| `StrongSomatic` | `printVariationsFromFirstSample()` or `determinateType()` | BAM1-only call with no successful rescue, or SNV counterpart downgraded to noise. |
| `LikelySomatic` | `determinateType()` | Counterpart exists but is low-frequency or effectively absent (`coverage <= 1`). |
| `Germline` | `determinateType()` or `combineAnalysis()` | Strong support in both samples, or combined-BAM rescue reconstructs the missing sample. |
| `LikelyLOH` | `determinateType()` or BAM2 fallback in `printVariationsFromFirstSample()` | One sample is near-homozygous while the other still retains mid-range or low-level support. |
| `StrongLOH` | `printVariationsFromFirstSample()` or `printVariationsFromSecondSample()` | BAM2-driven event with only synthesized/ref-only evidence on BAM1. |
| `AFDiff` | `determinateType()` | Counterpart is present but fails quality heuristics while still having non-trivial allele fraction. |

### Fisher Test Integration

The post-process modules do not execute Fisher tests themselves; the actual p-value and odds-ratio calculations live in `SimpleOutputVariant`, `SomaticOutputVariant`, and `AmpliconOutputVariant`. These modules are still Fisher-critical because they decide which `Variant` objects are passed into those constructors:

1. `SimplePostProcessModule` decides whether the printer sees a real variant, a reference variant, or `null`.
2. `SomaticPostProcessModule` is the most sensitive path because `SomaticOutputVariant` computes three Fisher tests from the chosen tumor/normal payloads. Placeholder objects such as `varForPrint`, `v1nt`, and `v2nt`, plus any fields mutated by `combineAnalysis()` or `isNoise()`, directly change the strand-bias and somatic significance numbers in the final output.
3. `AmpliconPostProcessModule` chooses the representative variant and the good/bad amplicon evidence lists that determine the per-row counts and therefore the Fisher test emitted by `AmpliconOutputVariant` when `--fisher` is enabled.

### AmpliconPostProcessModule

#### `process(Region, List<Map<Integer, Vars>>, Map<Integer, List<Tuple2<Integer, Region>>>, Set<String>, VariantPrinter)`

1. Collect all amplicon-covered positions and sort them.
2. For each position, initialize working sets for:
   - `gvs`: good variant plus amplicon-region tuples.
   - `ref`: reference variants.
   - `vrefList`: representative variants to print.
   - `goodmap`: string keys of `amplicon-ref-alt` combinations.
   - `vcovs`: per-amplicon coverage values.
   - `goodVariantsOnAmp`: good variants grouped by amplicon.
   - `nocov`, `maxcov`, and a tracked-but-unused `maxaf`.
3. For each amplicon covering the position:
   - Read `Vars vtmp` for that amplicon and position.
   - If variants exist, iterate them, tracking `totalPosCoverage` into `vcovs` and `maxcov`.
   - Compute each variant type and retain only those passing `isGoodVar(...)`.
   - For each good variant, append `(variant, "chr:start-end")` to `gvs`, append it to that amplicon's good list, update `goodmap`, and update `maxaf`.
   - If no variant list exists, fall back to reference coverage or zero coverage.
   - Add any available reference variant to `ref`.
4. Count `nocov` by marking amplicons whose coverage is below `maxcov / 50.0`.
5. Sort `gvs` by descending allele frequency and `ref` by descending total coverage.
6. If there are no good variants:
   - If pileup mode is disabled, skip the position.
   - If reference variants exist, seed `vrefList` with the highest-coverage reference.
   - Otherwise print `AmpliconOutputVariant(null, rg, null, null, position, 0, nocov, false)` and continue.
7. If good variants exist, call `fillVrefList(gvs, vrefList)` to deduplicate allele-identical output candidates.
8. Derive the initial `flag = isAmpBiasFlag(goodVariantsOnAmp)`.
9. For each representative variant in `vrefList`:
   - If `flag` is set, rebuild `goodVariants` around the top good variant's description string across all amplicons.
   - If every original good-variant tuple is recovered under that single description string, clear the flag.
   - Count how many amplicons contain the representative allele with `countVariantOnAmplicons(...)`.
   - Initialize `currentGvscnt` from that count.
   - Build `badVariants` from amplicons that do not contain the representative allele key.
   - Skip pileup-only reference duplicates when appropriate.
   - If the variant lies fully inside an amplicon's insert region, capture that amplicon's first variant, reference variant, or `null` as opposing evidence.
   - If the variant overlaps an amplicon primer boundary instead, decrement `currentGvscnt` when possible rather than adding a bad-variant tuple.
   - If `flag` was set but primer overlap reduced `currentGvscnt`, clear the flag.
   - Recompute `vartype`, normalize complex variants with `adjComplex()`, and print `AmpliconOutputVariant(vref, rg, goodVariants, badVariants, position, currentGvscnt, nocov, flag)`.
10. Report per-position exceptions through `printExceptionAndContinue()` and continue.

#### `countVariantOnAmplicons(Variant, Map<Integer, List<Variant>>)`

1. Initialize `gvscnt = 0`.
2. Iterate every amplicon's good-variant list.
3. For each variant, compare only `refallele` and `varallele` against the target variant.
4. Increment the count for every exact allele match.
5. Return the total number of matching amplicons.

#### `fillVrefList(List<Tuple2<Variant, String>>, List<Variant>)`

1. Iterate `gvs` in their current order.
2. Because `gvs` is sorted by descending frequency beforehand, earlier entries are preferred representatives.
3. For each tuple, scan the existing `vrefList` for the same `refallele` and `varallele`.
4. If no existing representative has that allele pair, append the current variant.
5. The method ignores description-string differences once ref/alt alleles match.

#### `isAmpBiasFlag(Map<Integer, List<Variant>>)`

1. Return `false` immediately when no amplicons contain good variants.
2. Sort amplicon ids and compare adjacent amplicon lists.
3. If any next-amlicon list is missing or has a different size, return `true`.
4. Sort both variant lists in-place by descending total coverage.
5. Compare `descriptionString` pairwise at each index.
6. If any description differs, return `true`.
7. If all adjacent lists match in size and description order, return `false`.

## Cross-Module Dependencies

- Called by:
  - `SimpleMode` -> `new SimplePostProcessModule(out).accept(...)`
  - `SomaticMode` -> `new SomaticPostProcessModule(referenceResource, variantPrinter)` through `thenAcceptBoth(...)`
  - `AmpliconMode` -> `new AmpliconPostProcessModule().process(...)`
- Depends on `Variations.md` types:
  - `Variant` for all output payloads, in-place normalization, quality checks, and mutation side effects.
  - `Vars` for per-position grouping and description-string lookup.
- Depends on `OutputVariant.md` printer layer:
  - `SimpleOutputVariant`, `SomaticOutputVariant`, `AmpliconOutputVariant`
  - `VariantPrinter`
- Depends on shared scope/config state:
  - `Scope<AlignedVarsData>` and region/splice data
  - `GlobalReadOnlyScope.instance().conf`
  - `Utils.printExceptionAndContinue()`
- Somatic-specific dependencies:
  - `ReferenceResource` and `Reference` for rescue re-runs
  - `DirectThreadExecutor` and `getMode().pipeline(...)` for synchronous combined-BAM analysis
  - `VariationUtils.getVarMaybe(...)` and `VariationUtils.strandBias(...)`
  - `Patterns.MINUS_NUM_NUM` for indel-shape detection before combine rescue
- Amplicon-specific dependencies:
  - `Tuple.Tuple2` and `tuple(...)` for amplicon evidence packaging
  - `Region.insertStart/insertEnd` semantics for distinguishing bad evidence vs primer overlap

## Known Parity Traps

- `SomaticPostProcessModule.accept()` receives `(scopeFromBam2, scopeFromBam1)` and then treats the second argument as the primary region/BAM1 source. Porting code with the wrong argument order will silently swap sample roles.
- `printVariationsFromFirstSample()` uses a `while` condition that stops at the first BAM1 variant failing `isGoodVar(...)`. Later BAM1 variants are never considered in that branch.
- `determinateType()` calls `variantToCompare.isNoise()`, and `Variant.isNoise()` mutates counts/frequencies in place. Labeling and emitted numeric fields are therefore coupled.
- `SomaticOutputVariant` constructor arguments are deliberately reused in unusual ways. The same `Variant` object may be passed as begin, end, tumor, and comparison payload simultaneously; placeholders and `null` values are semantically meaningful.
- `combineAnalysis()` mutates `variant2` in place via subtraction from a freshly re-run combined callset, clamps negatives to zero, forces `pstd/qstd` to `true`, and copies `highQualityToLowQualityRatio` from `variant1` instead of recomputing it.
- `combineAnalysis()` returns three different signals with different meanings: `"Germline"` means successful rescue, `""` means keep the caller's existing label, and `"FALSE"` means suppress the row entirely.
- In the BAM1 fallback LOH path, complex normalization is triggered by `v2var.vartype.equals("Complex")` but applied to `v1nt`. That asymmetry is easy to miss in a port.
- `SimplePostProcessModule` may emit a null/no-coverage skeleton row in pileup mode when no reference variant exists, and it may emit an extra reference row when a lone variant starts away from the map key position.
- `SimplePostProcessModule` allows structural-variant positions outside the requested region to survive if `variantsOnPosition.sv` is non-empty.
- `AmpliconPostProcessModule.fillVrefList()` deduplicates only by `refallele` and `varallele`, so different descriptions with the same allele pair collapse to the first, highest-frequency representative.
- `AmpliconPostProcessModule.isAmpBiasFlag()` sorts the per-amplicon good-variant lists in place before comparing them.
- `AmpliconPostProcessModule` records only the first variant from a non-supporting amplicon in `badVariants`; it does not search for the best allele match.
- `nocov` uses `t < maxcov / 50.0`. When `maxcov` is zero, no amplicon increments `nocov`, which is counterintuitive but matches Java behavior.
- `maxaf` is computed during amplicon processing and never used. A Rust port that tries to "fix" or consume it risks behavioral drift.