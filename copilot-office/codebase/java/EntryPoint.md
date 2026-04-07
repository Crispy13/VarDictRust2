# EntryPoint

**Source**: `Main.java`, `VarDictLauncher.java`, `modules/Module.java`, `exception/*.java`
**LOC**: ~340
**Rust counterpart**: distributed across `src/bin/`, `src/mods/`, and `src/data/`
**Status**: complete

## Overview

This module covers the bootstrap path from CLI invocation into a concrete VarDict pipeline mode. `Main` is a thin handoff layer. `VarDictLauncher` does all startup work: validating region inputs, reading BAM header metadata, deriving sample names, loading regions, building adaptor seed tables, initializing `GlobalReadOnlyScope`, selecting the execution mode, and finally dispatching serial or parallel execution. The same entry-point area also defines the generic `Module<T, R>` pipeline contract and the three small unchecked exceptions used during startup and reference loading.

## Bootstrap Flow

```text
Main.main(args)
  -> CmdParser.parseParams(args)
  -> new ReferenceResource()
  -> new VarDictLauncher(referenceResource).start(config)
       -> initResources(config)
            -> validate BED or -R source
            -> readChr(conf.bam.getBamX())
            -> derive sample names
            -> build regions from -R or BED
            -> build adaptor seed lookup maps
            -> GlobalReadOnlyScope.init(...)
       -> choose mode (Splicing, Somatic, Simple, or Amplicon)
       -> GlobalReadOnlyScope.setMode(mode)
       -> mode.notParallel() or mode.parallel()
```

## Method Inventory

| Method | Class | Lines | Analyzed? | Summary |
|--------|-------|-------|-----------|---------|
| `main(String[] args)` | `Main` | L10-L15 | yes | Parse CLI options, create `ReferenceResource`, hand off to launcher |
| `start(Configuration)` | `VarDictLauncher` | L44-L65 | yes | Initialize resources, select mode, dispatch serial or parallel execution |
| `initResources(Configuration)` | `VarDictLauncher` | L73-L133 | yes | Validate inputs and populate `GlobalReadOnlyScope` |
| `readBedFile(Configuration)` | `VarDictLauncher` | L141-L186 | yes | Load BED lines and auto-detect amplicon mode |
| `readChr(String)` | `VarDictLauncher` | L194-L206 | yes | Read chromosome lengths from BAM header |
| `getSampleNames(Configuration)` | `VarDictLauncher` | L213-L245 | yes | Derive simple-mode sample name from `-N` or regex |
| `getSampleNamesSomatic(Configuration)` | `VarDictLauncher` | L254-L281 | yes | Derive tumor and matched sample names for paired mode |
| `process(Scope<T>)` | `Module<T, R>` | L12 | yes | Generic pipeline-stage contract: transform one scoped payload into another |

## Method Analyses

### Main.main()

**Source**: `Main.java:L10-L15`

`Main` intentionally contains no bootstrap logic of its own. It performs three operations in order:

1. `new CmdParser().parseParams(args)` builds a mutable `Configuration` from CLI arguments.
2. `new ReferenceResource()` creates the FASTA access helper that downstream stages share.
3. `new VarDictLauncher(referenceResource).start(config)` transfers control to the launcher.

There is no exception handling here beyond `throws ParseException`, so all runtime bootstrap failures propagate directly to the process boundary.

### VarDictLauncher.start()

**Source**: `VarDictLauncher.java:L44-L65`

`start()` is the bootstrap coordinator. Its sequence is:

1. Call `initResources(config)`.
2. Read `instance().conf` back out of `GlobalReadOnlyScope` rather than continuing to use the method parameter directly.
3. Select mode in this order:
   - `conf.outputSplicing` -> `SplicingMode`
   - else if `conf.regionOfInterest != null` or no amplicon mode was detected -> `SomaticMode` for paired BAMs, otherwise `SimpleMode`
   - else -> `AmpliconMode`
4. Call `setMode(mode)` exactly once.
5. Dispatch to `mode.notParallel()` when `threads == 1`, otherwise `mode.parallel()`.

Two startup behaviors are easy to miss:

- Explicit `-R` forces the non-amplicon branch even if BED-driven amplicon calling would otherwise be possible.
- Mode selection happens only after the global singleton is initialized, so later code treats `GlobalReadOnlyScope` as the authoritative runtime state.

### VarDictLauncher.initResources()

**Source**: `VarDictLauncher.java:L73-L133`

`initResources()` does all real startup work before any pipeline module runs.

1. Reject runs with neither `-R` nor a BED file by throwing `RegionMissedSourceException`.
2. Read chromosome lengths from the first BAM header with `readChr(conf.bam.getBamX())`.
3. Derive sample names with `getSampleNames()` or `getSampleNamesSomatic()`.
4. Build `RegionBuilder(chrLengths, conf)`.
5. Load target regions:
   - `-R` -> `buildRegionFromConfiguration()`
   - BED -> `readBedFile(conf)` then `buildAmpRegions()` or `buildRegions()`
6. Build forward and reverse adaptor seed maps by sliding a 6-base window across each configured adaptor and storing offset `i + 1` for both orientations.
7. Call `GlobalReadOnlyScope.init(...)` with configuration, chromosome lengths, sample names, amplicon parameters, and adaptor maps.

`IOException` is wrapped in `RuntimeException`, but the domain-specific unchecked exceptions from region validation and reference loading are allowed to propagate unchanged.

### VarDictLauncher.readBedFile()

**Source**: `VarDictLauncher.java:L141-L186`

`readBedFile()` reads the BED file once and simultaneously decides whether it implies amplicon mode.

- Header lines beginning with `#`, `browser`, or `track` are skipped.
- If amplicon mode was not already forced by CLI, the method looks for exactly 8 columns and requires columns 7 and 8 to be integers.
- Amplicon mode turns on only when the amplicon interval is fully enclosed by the main region interval.
- When BED auto-detection enables amplicon mode and the user did not explicitly define zero-based coordinates, `zeroBased` is rewritten to `true`.

This BED scan therefore both loads data and mutates later execution semantics.

### Module<T, R>

**Source**: `modules/Module.java:L1-L13`

`Module<T, R>` is the minimal contract shared by VarDict pipeline stages:

```java
Scope<R> process(Scope<T> scope);
```

The type parameters describe only the stage-specific payload carried inside `Scope`. Shared context such as BAM path, region, reference, printer, and `ReferenceResource` stays on the outer `Scope` object and is inherited stage-to-stage. That design is why even trivial modules work on `Scope<T>` instead of raw payloads.

It is marked `@FunctionalInterface`, but in practice it is mainly used as a uniform implementation boundary for concrete pipeline classes such as `SAMFileParser`, `CigarParser`, `VariationRealigner`, and `ToVarsBuilder`.

## Exception Coverage

| Class | Trigger Condition | Thrown From | Notes |
|-------|-------------------|-------------|-------|
| `RegionMissedSourceException` | Neither `conf.regionOfInterest` nor `conf.bed` is set | `VarDictLauncher.initResources()` | Startup validation failure before any BAM or FASTA work begins |
| `WrongFastaOrBamException` | htsjdk reports `Unable to find entry for contig` while fetching reference sequence | `ReferenceResource.retrieveSubSeq()` | Indicates chromosome naming or reference mismatch between BAM, BED, and FASTA |
| `RegionBoundariesException` | htsjdk reports `Malformed query` while fetching reference sequence | `ReferenceResource.retrieveSubSeq()` | Wraps invalid start/end coordinates or off-contig fetches |

All three are thin `RuntimeException` wrappers. Their value is in preserving a domain-specific message at the point where startup or reference access fails, not in adding recovery logic.

## Cross-Module Dependencies

- Calls: `CmdParser`, `ReferenceResource`, `RegionBuilder`, `GlobalReadOnlyScope`, `SimpleMode`, `SomaticMode`, `AmpliconMode`, `SplicingMode`
- Called by: JVM process entry (`Main.main()`)
- Shared contract with pipeline: `Module<T, R>` is implemented by classes in `modules/` and consumed by mode pipelines
- Exception consumers: `ReferenceResource` and `VarDictLauncher`

## Known Parity Traps

- Bootstrap writes configuration into `GlobalReadOnlyScope` and immediately reads it back out before mode selection; ports should not assume launcher state stays purely local.
- BED parsing is not passive input loading: it can auto-enable amplicon mode and rewrite `zeroBased` when the user left that flag unspecified.
- `Module<T, R>` transforms `Scope` wrappers, not bare payload structs, so ports need to preserve shared per-region context between stages.