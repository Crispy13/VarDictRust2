# Self-Hosted Runner Setup

This repository's full parity workflow depends on large local test assets. A self-hosted GitHub Actions runner is the supported way to execute those jobs without checking multi-gigabyte reference data into git.

## Prerequisites

- Ubuntu 22.04 or newer
- `libclang-dev`
- `zlib1g-dev`
- `cmake`
- `openjdk-11-jdk`
- Rust stable toolchain

Install the system packages with:

```bash
sudo apt-get update
sudo apt-get install -y libclang-dev zlib1g-dev cmake openjdk-11-jdk
rustup toolchain install stable
rustup default stable
```

## Testdata

Provision the large reference inputs under `testdata/`. Small parity fixtures are already tracked in git and do not need manual setup.

```text
testdata/
├── hs37d5.fa                                         # ~3 GB
├── hs37d5.fa.fai                                    # reference index
├── NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam
├── NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam.bai
├── NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam
├── NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam.bai
├── 151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam
├── 151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bai
├── fixtures/                                        # tracked in git, no manual provisioning needed
└── parity_regions.tsv                               # tracked in git
```

Approximate sizes for the manually provisioned inputs:

- `testdata/hs37d5.fa` plus `testdata/hs37d5.fa.fai`: about 3 GB for the FASTA and a small index file
- `testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam` plus `.bai`: large exome BAM plus index
- `testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam` plus `.bai`: large low-coverage BAM plus index
- `testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam` plus `.bai`: large BAM plus index
- `testdata/fixtures/`: tracked in git, no manual provisioning needed
- `testdata/parity_regions.tsv`: tracked in git

## Sweep Fixtures

Tier 2 sweep validation needs an additional generated fixture cache:

```text
tmp/sweep_fixtures/    # ~240 GB
```

Generate it with:

```bash
python3 scripts/sweep_generate_v2.py
```

If you do not need Tier 2 coverage on a runner, you can skip this directory.

## Environment Variables

- `LIBCLANG_PATH`: path to the directory containing `libclang.so`
- `VARDICT_SWEEP_FIXTURE_DIR`: optional override for the sweep fixture cache directory; defaults to `tmp/sweep_fixtures`

Example:

```bash
export LIBCLANG_PATH=/usr/lib/llvm-14/lib
export VARDICT_SWEEP_FIXTURE_DIR=$PWD/tmp/sweep_fixtures
```

## GitHub Actions Runner Setup

Follow GitHub's self-hosted runner guide: <https://docs.github.com/actions/hosting-your-own-runners/about-self-hosted-runners>

Recommended labels for this repository:

- `self-hosted`
- `vardict-parity`

When provisioning the runner, make sure the repository checkout path has enough free space for:

- the large `testdata/` inputs
- `tmp/sweep_fixtures/` if Tier 2 is enabled
- normal Cargo build artifacts under `target/`

## Validation

Run these commands on the runner after provisioning data and environment variables:

```bash
cargo test --profile debug-release
cargo test --profile debug-release -- --include-ignored
```

The first command covers Tier 0 and Tier 1 checks. The second command includes ignored tests, including sweep coverage when sweep fixtures are available.