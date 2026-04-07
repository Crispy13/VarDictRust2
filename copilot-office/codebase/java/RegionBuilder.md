# RegionBuilder

**Source**: `RegionBuilder.java`
**LOC**: 205
**Rust counterpart**: not yet isolated in the current Rust workspace
**Status**: complete

## Overview

`RegionBuilder` is the startup-time translator from user-facing region inputs into VarDict's internal `List<List<Region>>` shape. It handles three entry paths: standard BED parsing, amplicon BED parsing, and `-R chr:start-end[:gene]` parsing. Along the way it applies BED column mapping from `Configuration`, normalizes chromosome names against BAM-header contigs, converts zero-based BED starts into VarDict's 1-based inclusive `Region.start`, and groups amplicon regions into contiguous segments for downstream mode execution.

## Method Inventory

| Method | Lines | Analyzed? | Summary |
|--------|-------|-----------|---------|
| `RegionBuilder()` | L25-L28 | yes | Test-oriented default constructor: creates empty chromosome map and fresh config |
| `RegionBuilder(Map<String, Integer>, Configuration)` | L30-L33 | yes | Normal constructor used by launcher setup |
| `buildRegions(List<String>, Boolean)` | L41-L96 | yes | Parse non-amplicon BED rows into one `List<Region>` per input row |
| `buildAmpRegions(List<String>, boolean)` | L104-L146 | yes | Parse amplicon BED rows and merge overlapping insert windows into segment groups |
| `buildRegionFromConfiguration()` | L152-L170 | yes | Parse the `-R` CLI string into a singleton `List<List<Region>>` |
| `correctChromosome(Map<String, Integer>, String)` | L178-L187 | yes | Toggle the `chr` prefix when BED naming and BAM-header naming disagree |
| `BedRowFormat(int, int, int, int, int, int)` | L197-L204 | yes | Store zero-based BED column indexes for one layout |

## Method Analyses

### `RegionBuilder()`

1. Construct a fresh `Configuration`.
2. Construct an empty `HashMap<String, Integer>` for chromosome lengths.

This constructor is mainly useful in tests or ad hoc instantiation. Real pipeline execution uses the two-argument constructor so `RegionBuilder` sees launcher-populated config and BAM-header lengths.

### `RegionBuilder(Map<String, Integer>, Configuration)`

1. Store the supplied chromosome-length map.
2. Store the supplied `Configuration`.

No defensive copies are made; the builder reads the same mutable configuration object produced by `CmdParser` / `VarDictLauncher`.

### `buildRegions(List<String>, Boolean)`

This is the non-amplicon BED parser.

1. Seed `isZeroBased` from `config.zeroBased` if the CLI explicitly defined it; otherwise default to `false`.
2. Start with `format = config.bedRowFormat`.
3. For each BED line, split on `config.delimiter`.
4. Detect the four-column custom format only when both of these are true:
   - `config.isColumnForChromosomeSet()` is false, and
   - the row has exactly four columns.
5. In that autodetect path, parse columns 2 and 3 as integers; if `a1 <= a2`, switch to `CUSTOM_BED_ROW_FORMAT` and, if the caller passed `zeroBased == null`, default `isZeroBased = true`.
6. Read chromosome, CDS start, CDS end, and gene using the selected `BedRowFormat`. If `geneColumn` is out of range, fall back to chromosome name.
7. Split the thick-start and thick-end columns on commas. Each BED row can therefore fan out into multiple `Region` objects.
8. For each thick interval:
   - parse `thickStart` / `thickEnd`
   - skip intervals entirely before `cdsStart`
   - stop scanning when `cdsEnd > thickEnd` per Java's current control flow
   - clamp the interval to `[cdsStart, cdsEnd]`
   - extend both sides by `config.numberNucleotideToExtend`
   - if coordinates are zero-based and `thickStart < thickEnd`, increment only the start coordinate
   - emit `new Region(chr, thickStart, thickEnd, gene)`
9. Append that row's `thickRegions` list to the outer `segs` list.
10. Return `segs`.

The outer `List<List<Region>>` preserves one inner list per BED line, which is important because modes treat those inner lists as a linked segment set.

### `buildAmpRegions(List<String>, boolean)`

This is the amplicon-specific BED parser.

1. Start `segs` as an empty outer list and `tsegs` as `Map<String, List<Region>>` keyed by chromosome.
2. For each BED line, split by `config.delimiter` and read fields using the fixed `AMP_BED_ROW_FORMAT`:
   - `chr`, `start`, `end`, `gene`, `insertStart`, `insertEnd`
3. Normalize chromosome naming through `correctChromosome(...)`.
4. If `zeroBased` is true and `start < end`, increment `start` and `insertStart`. `end` and `insertEnd` are intentionally left unchanged.
5. Insert `new Region(chr, start, end, gene, insertStart, insertEnd)` into the per-chromosome list inside `tsegs`.
6. Initialize the first output bucket: `regions = new LinkedList<>(); segs.add(regions)`.
7. Iterate chromosome buckets, sort each chromosome's regions by `insertStart`, and walk them in that order.
8. Start a new output bucket whenever:
   - chromosome changes, or
   - the next amplicon's `insertStart` is greater than the previous bucket's `previousEnd`
9. Append each region into the current bucket and advance `previousChr` / `previousEnd`.
10. Return grouped amplicon segments.

The grouping rule means overlapping or touching insert windows remain in one inner list, while gaps split the outer list into separate amplicon segments.

### `buildRegionFromConfiguration()`

1. Split `config.regionOfInterest` on `:`.
2. Treat element 0 as chromosome and normalize it with `correctChromosome(...)`.
3. Treat element 2 as gene when present; otherwise use chromosome name as the gene label.
4. Split the middle token on `-` to parse start and optional end.
5. Remove commas from the numeric strings before `toInt(...)`.
6. If there is no explicit end, use a single-position region (`end = start`).
7. Expand both ends by `config.numberNucleotideToExtend`.
8. If zero-based mode is explicitly configured and `start < end`, increment only the start coordinate.
9. Clamp `start` down to `end` if expansion inverted the range.
10. Return `singletonList(singletonList(new Region(chr, start, end, gene)))`.

This method always produces exactly one segment with exactly one region.

### `correctChromosome(Map<String, Integer>, String)`

1. Check whether the incoming chromosome string exists in the BAM-header length map.
2. If it exists, return it unchanged.
3. If it does not exist and starts with `"chr"`, strip that prefix.
4. Otherwise prepend `"chr"`.
5. Return the toggled value.

This is a simple compatibility shim between BED naming and BAM naming; it does not verify that the toggled name actually exists either.

### `BedRowFormat(int, int, int, int, int, int)`

1. Store the six zero-based column indexes exactly as supplied.
2. These indexes drive both non-amplicon parsing (`config.bedRowFormat`) and the predefined default/custom/amplicon layouts.

## Cross-Module Dependencies

- Called by `VarDictLauncher.initResources()` to convert BED rows or `-R` into `List<List<Region>>` before mode selection.
- Reads `Configuration.delimiter`, `bedRowFormat`, `zeroBased`, `regionOfInterest`, `numberNucleotideToExtend`, and `columnForChromosome` state set by `CmdParser`.
- Uses `Utils.toInt()` for all numeric parsing.
- Uses BAM-header chromosome lengths from `VarDictLauncher.readChr()` to normalize chromosome names.
- Produces `Region` objects that are then consumed by modes, `ReferenceResource`, and every downstream pipeline stage through `Scope`.

## Known Parity Traps

- BED start conversion is asymmetric: when zero-based handling is active, only the start coordinate is incremented; the end coordinate is left unchanged because BED end already numerically matches 1-based inclusive end.
- `buildRegions()` may silently switch to `CUSTOM_BED_ROW_FORMAT` only for four-column rows and only when `-c` was not set.
- In `buildRegions()`, an explicit CLI `config.zeroBased` overrides the `zeroBased` argument; the argument mainly matters for BED autodetection.
- The non-amplicon exon loop breaks on `cdsEnd > thickEnd`, which is unintuitive but part of the frozen Java behavior.
- `buildAmpRegions()` groups per chromosome after storing rows in a `HashMap`, so chromosome-bucket iteration order is not guaranteed by the Java type.
- `correctChromosome()` blindly toggles the `chr` prefix on a miss; if neither spelling exists in the BAM header, the returned chromosome string is still the toggled guess.