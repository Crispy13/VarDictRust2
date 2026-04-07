# VarDictJava Codebase Cache

> Progressive knowledge base built by `java-analyst` during source analysis.
> VarDictJava is **frozen at v1.8.3** — these analyses do not go stale.
> Each module has its own file to keep agent context small.

## Java Source Root

```
VarDictJava/src/main/java/com/astrazeneca/vardict/
```

## Pipeline Flow

```
Main.main()
  → CmdParser.parseParams()
  → new ReferenceResource()
  → VarDictLauncher.start()
    → readChr() + sample-name discovery + RegionBuilder + GlobalReadOnlyScope.init()
    → select mode
      → Simple | Somatic | Amplicon
        → SAMFileParser.process()
          → RecordPreprocessor + SamView
        → CigarParser.process()
          → [if performLocalRealignment] CigarModifier.modifyCigar()
        → VariationRealigner.process()
        → StructuralVariantsProcessor.process()
        → ToVarsBuilder.process()
        → SimplePostProcessModule | SomaticPostProcessModule | AmpliconPostProcessModule
        → OutputVariant + VariantPrinter
          → [if fisher output enabled] FisherExact
      → Splicing
        → SAMFileParser.process()
        → CigarParser.process() in splicing mode
        → direct intron output
```

## Module Index

Rows below use primary ownership for coverage accounting. Several files are intentionally discussed in more than one cache doc, but each Java source file is counted exactly once in this table.

| Module | Primary Java Files | File Count | LOC | Risk | Cache File | Status |
|--------|--------------------|------------|-----|------|------------|--------|
| EntryPoint | `Main.java`<br>`VarDictLauncher.java`<br>`modules/Module.java`<br>`exception/RegionBoundariesException.java`<br>`exception/RegionMissedSourceException.java`<br>`exception/WrongFastaOrBamException.java` | 6 | 336 | LOW | [EntryPoint.md](EntryPoint.md) | complete |
| Configuration and CLI | `CmdParser.java`<br>`Configuration.java` | 2 | 1,028 | LOW | [Configuration.md](Configuration.md) | complete |
| RegionBuilder | `RegionBuilder.java` | 1 | 206 | MEDIUM | [RegionBuilder.md](RegionBuilder.md) | complete |
| Modes | `modes/AbstractMode.java`<br>`modes/AmpliconMode.java`<br>`modes/SimpleMode.java`<br>`modes/SomaticMode.java`<br>`modes/SplicingMode.java` | 5 | 690 | LOW | [Modes.md](Modes.md) | complete |
| SAMFileParser | `modules/SAMFileParser.java`<br>`modules/RecordPreprocessor.java`<br>`data/SamView.java` | 3 | 245 | MEDIUM | [SAMFileParser.md](SAMFileParser.md) | complete |
| CigarModifier | `modules/CigarModifier.java` | 1 | 787 | MEDIUM | [CigarModifier.md](CigarModifier.md) | complete |
| CigarParser | `modules/CigarParser.java` | 1 | 2,376 | HIGH | [CigarParser.md](CigarParser.md) | complete |
| VariationRealigner | `modules/VariationRealigner.java` | 1 | 2,744 | HIGH | [VariationRealigner.md](VariationRealigner.md) | complete |
| StructuralVariantsProcessor | `modules/StructuralVariantsProcessor.java` | 1 | 2,097 | HIGH | [StructuralVariantsProcessor.md](StructuralVariantsProcessor.md) | complete |
| ToVarsBuilder | `modules/ToVarsBuilder.java` | 1 | 1,058 | MEDIUM | [ToVarsBuilder.md](ToVarsBuilder.md) | complete |
| PostProcessModules | `postprocessmodules/AmpliconPostProcessModule.java`<br>`postprocessmodules/SimplePostProcessModule.java`<br>`postprocessmodules/SomaticPostProcessModule.java` | 3 | 858 | HIGH | [PostProcessModules.md](PostProcessModules.md) | complete |
| OutputVariant Printers | `printers/AmpliconOutputVariant.java`<br>`printers/OutputVariant.java`<br>`printers/PrinterType.java`<br>`printers/SimpleOutputVariant.java`<br>`printers/SomaticOutputVariant.java`<br>`printers/SystemErrVariantPrinter.java`<br>`printers/SystemOutVariantPrinter.java`<br>`printers/VariantPrinter.java` | 8 | 975 | MEDIUM | [OutputVariant.md](OutputVariant.md) | complete |
| FisherExact | `data/fishertest/FisherExact.java`<br>`data/fishertest/UnirootZeroIn.java` | 2 | 304 | HIGH | [FisherExact.md](FisherExact.md) | complete |
| ScopeData | `data/BaseInsertion.java`<br>`data/CurrentSegment.java`<br>`data/Match.java`<br>`data/Match35.java`<br>`data/ModifiedCigar.java`<br>`data/Patterns.java`<br>`data/Reference.java`<br>`data/ReferenceResource.java`<br>`data/Region.java`<br>`data/SVStructures.java`<br>`data/Side.java`<br>`data/SortPositionSclip.java`<br>`data/scopedata/AlignedVarsData.java`<br>`data/scopedata/CombineAnalysisData.java`<br>`data/scopedata/GlobalReadOnlyScope.java`<br>`data/scopedata/InitialData.java`<br>`data/scopedata/RealignedVariationData.java`<br>`data/scopedata/Scope.java`<br>`data/scopedata/VariationData.java` | 19 | 946 | LOW | [ScopeData.md](ScopeData.md) | complete |
| VariationMap and Collections | `collection/ConcurrentHashSet.java`<br>`collection/DirectThreadExecutor.java`<br>`collection/Tuple.java`<br>`collection/VariationMap.java` | 4 | 231 | LOW | [VariationMap.md](VariationMap.md) | complete |
| Variations | `variations/Cluster.java`<br>`variations/Mate.java`<br>`variations/Sclip.java`<br>`variations/Variant.java`<br>`variations/Variation.java`<br>`variations/VariationUtils.java`<br>`variations/Vars.java` | 7 | 1,239 | MEDIUM | [Variations.md](Variations.md) | complete |
| Utils | `Utils.java` | 1 | 279 | LOW | [Utils.md](Utils.md) | complete |

Cross-reference note: `VarDictLauncher.java`, `GlobalReadOnlyScope.java`, `SamView.java`, `VariationMap.java`, `Utils.java`, and the post-process modules are also discussed in other cache docs where their behavior affects adjacent pipeline stages.

## Coverage Summary

- Source-of-truth inventory: 66 Java source files under `VarDictJava/src/main/java/com/astrazeneca/vardict/`
- Cache docs in this index: 17 module docs, all marked `complete`
- File coverage: 66/66 Java files have at least one cache doc and a primary index row
- Total indexed Java LOC: 16,399
- Stale-count correction: earlier stage notes said 68 Java files, but the current source tree contains 66 files across the requested directories

## Per-Module File Template

When creating a module file for the first time, use this structure:

```markdown
# ModuleName

**Source**: `path/relative/to/vardict/`
**LOC**: N
**Rust counterpart**: `src/mods/module_name.rs`
**Status**: partial | complete

## Overview
One-paragraph summary of the module's role in the pipeline.

## Method Inventory
| Method | Lines | Analyzed? | Summary |
|--------|-------|-----------|---------|
| methodName() | L100-L200 | yes/no | one-liner |

## Method Analyses
(Full step-by-step analyses following the java-analyst output format)

## Cross-Module Dependencies
- Calls: which modules this depends on
- Called by: which modules invoke this

## Known Parity Traps
- Specific Java behaviors that cause Rust translation bugs
```
