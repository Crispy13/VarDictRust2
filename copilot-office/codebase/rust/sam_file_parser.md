# sam_file_parser

**Source**: `src/mods/sam_file_parser.rs`, `src/mods/mod.rs`
**Java counterpart**: `SAMFileParser.java`, `RecordPreprocessor.java`, `SamView.java`
**Status**: complete

## Overview

BAM ingestion entry gate — splits a colon-delimited BAM path string, opens overlapping-query iterators via rust-htslib `IndexedReader`, and streams filtered reads through a two-layer filter cascade (SAM flag filter + preprocessRecord cascade) to CigarParser. Three tightly coupled Java classes are merged into a single Rust module. `RecordPreprocessor` wraps `InitialData` and provides a polling iterator (`next_record() → Option<Record>`) that CigarParser consumes.

## Method Inventory

| Method / Area | Covered? | Summary |
|---------------|----------|---------|
| `parse_samfilter(s)` | yes | `Integer.decode()` equivalent — hex/octal/decimal parsing |
| `get_mate_reference_name(record, header)` | yes | Shared utility: mtid<0→"*", tid==mtid→"=", else name |
| `get_chr_name(region, conf)` | yes | Strip "chr" prefix when -C flag set |
| `sam_file_parser_process(scope)` | yes | Pipeline entry: split BAM path, create RecordPreprocessor, wrap in Scope |
| `RecordPreprocessor::new(bams, region, data)` | yes | Constructor: parse filter, build BAM stack, open first reader |
| `RecordPreprocessor::next_record()` | yes | Public iterator: try current reader, switch BAM on exhaustion |
| `RecordPreprocessor::next_reader()` | yes | Open next BAM, reset dup state |
| `RecordPreprocessor::next_on_current_reader()` | yes | Inner loop: read + flag filter + preprocess cascade |
| `RecordPreprocessor::preprocess_record()` | yes | 6-step filter: downsampling→mapq→secondary→no-seq→totalReads++→dup |
| `RecordPreprocessor::close()` | yes | Explicit reader teardown (drops IndexedReader) |
| `RecordPreprocessor::get_chr_name()` | yes | Instance method delegating to free function |

## Java↔Rust Correspondence

| Java | Rust | Notes |
|------|------|-------|
| `SAMFileParser.process()` | `sam_file_parser_process()` | Free function, manual Scope destructure/reconstruct |
| `RecordPreprocessor` class | `RecordPreprocessor` struct | Wraps `InitialData` (owned) |
| `SamView` class + ThreadLocal cache | Integrated into `RecordPreprocessor` | No separate struct; no reader caching (Phase 1) |
| `ArrayDeque.push()` + `pollLast()` | `Vec::insert(0,...)` + `pop()` | Same processing order preserved |
| `Integer.decode(samfilter)` | `parse_samfilter()` | Full hex/octal/decimal support |
| `CigarParser.getMateReferenceName()` | `get_mate_reference_name()` | Takes explicit `&HeaderView` (rust-htslib Record.header is `pub(crate)`) |
| `record.getReadString()` == `"*"` | `record.seq().len() == 0` | Different no-seq representation |
| `Random(currentTimeMillis)` | `rand::random::<f64>()` | Both non-deterministic; disable -Z for parity tests |
| `record.getAlignmentStart()` (1-based) | `record.pos() + 1` | Explicit +1 for 0→1-based conversion |
| `record.getMateAlignmentStart()` (1-based) | `record.mpos() + 1` | Explicit +1 for 0→1-based conversion |

## Known Parity Traps

1. **Coordinate translation**: rust-htslib is 0-based; all dup keys and position comparisons add +1.
2. **Filter cascade order**: Must be exactly downsampling→mapq→secondary→no-seq→totalReads++→dup. `totalReads` counts before dup filtering.
3. **Secondary alignment check**: `conf.samfilter != "0"` is string equality, not numeric.
4. **Duplicate key format**: Branch 1: `"{pos}-{ref}-{matepos}"`, Branch 2: `"{pos}-{cigar}"` — all 1-based positions.
5. **`firstMatchingPosition` asymmetry**: Only updated in branches 1 and 2, NOT branch 3.
6. **No-sequence representation**: rust-htslib `seq.len() == 0` vs Java `"*"` (length 1).
7. **SamView flag filter ordering**: Flag filter runs BEFORE `preprocess_record()`.
8. **Reader caching**: Java ThreadLocal cache skipped in Phase 1 (performance-only, not parity-affecting).

## Divergences

- **Reader lifecycle**: Java `SamView` caches `SamReader` per thread via `ThreadLocal`; Rust opens fresh `IndexedReader` per BAM. Confirmed performance-only — same records returned in same order.
- **JSONL diagnostics**: `appendJsonlRecord()` / `escapeJson()` omitted — diagnostic-only, not parity-relevant.
- **Traceability comment style**: Uses `// Ported from:` instead of `/// Ported from:` doc-comment form.

## Cross-Module Dependencies

- **Upstream**: Called by pipeline modes (`AbstractMode.pipeline()`, `partialPipeline()`, `splicingPipeline()`)
- **Downstream**: `RecordPreprocessor` consumed by CigarParser via `scope.data.next_record()`
- **Shared function**: `get_mate_reference_name()` used by both RecordPreprocessor and CigarParser
- **Data flow**: `RecordPreprocessor.initial_data` provides CigarParser access to the five variant/coverage maps
- **Dependencies**: `src/config.rs` (Configuration), `src/data.rs` (InitialData, Region), `src/scope.rs` (Scope, GlobalReadOnlyScope)
