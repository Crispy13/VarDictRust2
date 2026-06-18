# VarDictRust

> **Unofficial Rust port of VarDictJava.** `VarDictRust` is under active development. It is intended to become a
> faster, VarDictJava-compatible implementation, but it is not yet a
> production-certified replacement for all VarDictJava workflows.

## Introduction

`VarDictRust` is a Rust implementation of VarDict for bioinformatics users who
want VarDict-compatible variant calling with improved runtime and resource
behavior.

The project follows VarDictJava as the behavioral baseline. The acceptance goal
is byte-for-byte parity for implemented workflows, including output fields,
formatting, filtering behavior, and edge-case handling.

VarDict is a sensitive variant caller for single-sample and paired-sample
variant calling from BAM files. It is commonly used for targeted sequencing and
cancer variant-calling workflows. Please cite the original VarDict publication:

Lai Z, Markovets A, Ahdesmaki M, Chapman B, Hofmann O, McEwen R, Johnson J,
Dougherty B, Barrett JC, and Dry JR. VarDict: a novel and versatile variant
caller for next-generation sequencing in cancer research. Nucleic Acids Res.
2016, pii: gkw227.

Article link:
https://academic.oup.com/nar/article/44/11/e108/2468301

## Purpose of porting
This port is for providing more performant VarDict Software. 
See [THIS](./docs/performance.md) for performance benchmark of VarDictRust vs VarDictJava.

## Current Status

Implemented calling modes:

- Simple mode
- Somatic mode

Not implemented and not currently planned:

- Amplicon mode
- Splicing mode

Current BAM-backed validation coverage:

- Simple mode: 2 BAM inputs
- Somatic mode: 1 BAM input

This means the implemented paths have real BAM-based parity coverage, but the
project should not yet be treated as a complete replacement for every
VarDictJava mode, option, and edge case. More fixtures, options, and workflows
still need to be validated.

Amplicon and splicing modes are not currently planned because suitable test data
for validating byte-for-byte parity is not readily available.

Performance is a project goal. Benchmark numbers are intentionally not listed
yet because the performance report is still being prepared.

## Disclaimer


## Requirements

For building and testing this repository:

1. Rust toolchain with Cargo
2. System libraries required by the Rust dependencies
3. Indexed reference FASTA files for real runs
4. Indexed BAM files for real runs

Unlike VarDictJava, this repository does not currently require JDK, Gradle, R,
or Perl for the Rust binary itself.

Depending on your system, `LIBCLANG_PATH` may need to be set to the directory
that contains `libclang`. On systems where libclang is already discoverable,
this is not necessary.

To see the help page for the program, run:

```bash
cargo run --profile deploy --bin vardict_rs -- --help
```

## Getting Started

### Getting Source Code

Clone this repository, then build from the repository root.

```bash
git clone https://github.com/Crispy13/VarDictRust2.git
cd VarDictRust2
```

### Compiling

Use the deployment profile for production builds:

```bash
cargo build --profile deploy
```

Use your own development profile or default Cargo settings for local debugging.

The main binary is `vardict_rs`.

### Distribution Package Structure

This repository currently documents direct Cargo builds. A packaged release
layout is not yet described here.

For local use, the deploy binary is built under Cargo's target directory. You
can run through Cargo:

```bash
cargo run --profile deploy --bin vardict_rs -- --help
```

or run the built binary directly after building:

```bash
target/deploy/vardict_rs --help
```

### Third-Party Libraries

The Rust implementation uses Rust crates for command-line parsing, BAM/FASTA
access, compression, statistics, and related functionality. See `Cargo.toml` for
the exact dependency list.

## Single Sample Mode

To run `vardict_rs` in single sample mode, use one BAM file without the `|`
separator.

Example with a BED file:

```bash
AF_THR="0.01"
target/deploy/vardict_rs \
  -G /path/to/hg19.fa \
  -f "$AF_THR" \
  -N sample_name \
  -b /path/to/sample.bam \
  -c 1 -S 2 -E 3 -g 4 \
  /path/to/targets.bed \
  > vars.tsv
```

The `-c`, `-S`, `-E`, and `-g` options identify the BED columns for chromosome,
start, end, and gene or annotation.

`vardict_rs` can also be invoked without a BED file if the region is specified
with `-R`:

```bash
target/deploy/vardict_rs \
  -G /path/to/hg19.fa \
  -f 0.001 \
  -N sample_name \
  -b /path/to/sample.bam \
  -R chr7:55270300-55270348:EGFR \
  > vars.tsv
```

## Paired / Somatic Variant Calling

To run paired or somatic calling, provide tumor and normal BAM files as
`tumor.bam|normal.bam`.

Example with a BED file:

```bash
AF_THR="0.01"
target/deploy/vardict_rs \
  -G /path/to/hg19.fa \
  -f "$AF_THR" \
  -N tumor_sample_name \
  -b "/path/to/tumor.bam|/path/to/normal.bam" \
  -c 1 -S 2 -E 3 -g 4 \
  /path/to/targets.bed \
  > vars.tsv
```

Somatic mode currently has BAM-backed validation coverage for 1 BAM pair.
Validate output against VarDictJava before using results in production analysis.

## Amplicon Based Calling

VarDictJava treats 8-column BED input as amplicon-aware calling when `-R` is not
specified.

Amplicon mode is not implemented in `vardict_rs` and is not currently planned.
The blocker is practical validation: byte-for-byte parity requires suitable BAM,
FASTA, BED, and expected-output test data, and proper amplicon test data is not
currently available for this project.

If you need amplicon workflows, use VarDictJava.

## Splicing Mode

Splicing mode is not implemented in `vardict_rs` and is not currently planned.
As with amplicon mode, the project does not currently have suitable test data
for validating byte-for-byte parity for this workflow.

If you need splicing-specific behavior, use VarDictJava.

## R, Perl, And VCF Conversion

VarDictJava examples commonly pipe the intermediate output through
`teststrandbias.R`, `testsomatic.R`, `var2vcf_valid.pl`, or
`var2vcf_paired.pl`.

`vardict_rs` does not currently bundle or document local copies of those R and
Perl scripts. The examples in this README therefore write the VarDict-style TSV
output directly.

If you use external VarDict post-processing scripts, validate the full pipeline
against VarDictJava for your exact command line. The `--fisher` option is
available for in-process Fisher calculations where supported.

## Differences From VarDictJava CLI

`vardict_rs` is designed to be familiar to VarDictJava users, but it is not a
native Commons CLI parser. The Rust binary uses Clap and normalizes a few
Java-style flags before parsing.

| Area | VarDictJava | `vardict_rs` |
| --- | --- | --- |
| Executable | `VarDict` from the Java distribution | `vardict_rs` Rust binary |
| Build command | `./gradlew clean installDist` | `cargo build --profile deploy` |
| Help | `-H` or `-?` | `-H`, `-?`, or `--help` |
| Header flag | `-h` prints header | `-h/--header` prints header |
| VCF flag | `-v` participates in Java pipeline behavior | `-v` is accepted for compatibility but currently ignored; output remains VarDict-style TSV |
| Threads | `-th [threads]` | `-th` is accepted and normalized internally; `--th` is the Rust long-form equivalent |
| Multi-letter Java flags | Parsed directly by Commons CLI | `-UN`, `-DP`, and `-VS` are accepted and normalized internally |
| Optional-value flags | Java parser behavior | `-k`, `-z`, and parser-level `-a/--amplicon` can be supplied with omitted values and use defaults; amplicon mode itself is not implemented |
| R/Perl scripts | Distributed with VarDictJava release packages | Not currently bundled or documented in this repository |

## Running Tests

Run the full repository validation command with ignored tests included:

```bash
cargo test --profile deploy -- --include-ignored
```

Ignored tests are part of the parity workflow. They may represent cost-gated,
nightly, or temporary parity checks rather than tests that should be ignored
forever.

Important parity and runner documentation:

- [`docs/sweep-parity.md`](docs/sweep-parity.md)
- [`docs/self-hosted-runner.md`](docs/self-hosted-runner.md)
- [`scripts/ignored_tests_allowlist.txt`](scripts/ignored_tests_allowlist.txt)

Temporary and intermediate files should be created under `./tmp`.

## Program Workflow

The intended `vardict_rs` workflow follows the VarDictJava model for implemented
paths:

1. Read regions of interest from a BED file or from `-R`.
2. For each segment, read mapped BAM records overlapping the target region.
3. Apply read filters, CIGAR handling, local realignment behavior, and variant
   construction according to the implemented VarDictJava-compatible logic.
4. Apply filtering and classification rules.
5. Write VarDict-style tabular output.

Where behavior is implemented, parity with VarDictJava is the target. Where a
mode or option is not yet validated, use VarDictJava as the reference.

## Program Options

The current `vardict_rs` binary exposes a VarDictJava-style option surface.
Use `--help` for the exact parser help for your checkout.

### Core Inputs

- `-G <FASTA>`: reference FASTA, indexed with `.fai`
- `-b <BAM>`: indexed BAM file; use `tumor.bam|normal.bam` for somatic mode
- `-N <NAME>`: sample name
- `<BED>`: positional BED file path
- `-R <REGION>`: region string, for example `chr7:55270300-55270348:EGFR`

### BED And Region Options

- `-c <INT>`: chromosome column, default `1`
- `-S <INT>`: region start column, default `2`
- `-E <INT>`: region end column, default `3`
- `-g <INT>`: gene or annotation column
- `-s <INT>`: insert start column
- `-e <INT>`: insert end column
- `-d <DELIM>`: BED delimiter
- `-z [0|1]`: BED zero-based coordinate handling
- `-x <INT>`: segment extension
- `-a/--amplicon [INT:FLOAT]`: parser compatibility only; amplicon mode is not implemented

### Calling And Filtering Options

- `-f <FLOAT>`: allele frequency threshold
- `-r <INT>`: minimum variant reads
- `-q <FLOAT>`: good base quality threshold
- `-m <INT>`: mismatch filter
- `-Q <INT>`: mapping quality read filter
- `-P <FLOAT>`: read position filter
- `-o <FLOAT>`: high-quality to low-quality read ratio
- `-O <FLOAT>`: mean mapping quality filter
- `-B <INT>`: minimum reads for strand bias
- `-V <FLOAT>`: normal-sample low-frequency threshold in paired mode
- `-X <INT>`: extension for nearby mismatch or indel handling
- `-I <INT>`: indel size
- `-M <INT>`: minimum matched bases

### Mode And Output Options

- `-p`: pileup regardless of frequency
- `-h/--header`: print a header row
- `-i/--splice`: parser compatibility only; splicing mode is not implemented
- `-D/--debug`: debug output
- `-y`: verbose mode
- `--fisher`: calculate Fisher values in-process where supported
- `-v`: accepted for compatibility; currently ignored

### Read Handling Options

- `-t/--dedup`: remove duplicated reads
- `-u`: unique mode using forward read handling for overlaps
- `-UN`: unique mode using first-read handling for overlaps
- `-K`: include `N` bases in total depth calculation
- `-F <BIT>`: SAM flag filter
- `-T/--trim <INT>`: trim reads after a position
- `-Z/--downsample <FLOAT>`: downsampling fraction
- `--chimeric`: turn off chimeric read filtering

### Realignment, SV, CRISPR, And Miscellaneous Options

- `-k [0|1]`: local realignment control
- `-3`: move indels to 3-prime when possible
- `-U/--nosv`: turn off structural variant calling
- `-L <INT>`: minimum structural variant length
- `-w/--insert-size <INT>`: insert size
- `-W/--insert-std <INT>`: insert size standard deviation
- `-A <INT>`: insert-size standard deviation amount
- `-Y/--ref-extension <INT>`: reference extension
- `--deldupvar`: delete duplicate variants in output
- `-J/--crispr <SITE>`: CRISPR cutting site
- `-j <INT>`: CRISPR filtering overlap
- `--mfreq <FLOAT>`: monomer MSI frequency threshold
- `--nmfreq <FLOAT>`: non-monomer MSI frequency threshold
- `--DP <TYPE>`: default printer compatibility option
- `--VS <STRICT|LENIENT|SILENT>`: SAM/BAM validation strictness
- `--adaptor <SEQ[,SEQ...]>`: adaptor sequences

## Input Files

### BED File - Regions

For simple and paired calling, provide BED columns for chromosome, start, end,
and gene or annotation with `-c`, `-S`, `-E`, and `-g`.

Lines beginning with common BED metadata prefixes such as comments, `browser`,
or `track` should be treated as metadata rather than target intervals.

### FASTA File - Reference Genome

The reference genome should be provided in FASTA format and indexed. Use `-G` to
set the FASTA path.

### BAM File - Aligned Reads

Input reads should be provided in indexed BAM format. Use a single BAM for
simple mode and a `tumor.bam|normal.bam` pair for somatic mode.

## Development Model

This repository has been written largely by coding agents with human monitoring.
Changes are expected to preserve VarDictJava behavior and to be validated
against the parity harness before being treated as accepted.

The near-term direction is to expand validated parity coverage, then publish
reproducible performance results for the implemented workflows.
