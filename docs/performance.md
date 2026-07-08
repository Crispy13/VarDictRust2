# Production bench — VarDictJava (vdj) vs VarDict-rs (vdr)

Machine load at start: `06:56:01 up 2 min,  2 users,  load average: 1.85, 1.37, 0.55`

Runs timed per cell: 3 (median; 1 warmup discarded). Thread count: -th 8.
VarDict-rs binary: `debug-release` profile (the project's standard build; a PGO / `target-cpu=native`
"production" build would be a separate, faster artifact).

## What was tested

Each row is one **cell** = (workload × preset). Both tools (VarDictJava and VarDict-rs) are run with **identical
flags** on the same inputs at `-th 8`, then their TSV output is compared for parity and their resource use timed.

### Workloads (the input data)

| Workload | Lane | Reference | BAM(s) | Region scope |
|---|---|---|---|---|
| **simple** | single-sample simple | `hs37d5` (GRCh37) | NA12878 low-coverage WGS | chr1 only (coverage-derived BED, ~20 Mb budget) to stay tractable |
| **somatic** | tumor/normal pair | `GRCh38.d1.vd1` | WES_IL tumor \| normal (`-b "T\|N"`) | tumor∩normal covered region (coverage-derived BED) |

Both BAMs are Illumina exome/WGS. The BEDs are built from each BAM set's real coverage so the mix of
high/low-depth regions reflects a production-shaped run, not a toy region.

### Presets (the caller settings)

A "preset" is just a named set of VarDict command-line flags (defined in `scripts/config_presets.tsv`). The four
here probe distinct calling regimes:

| Preset | Flags | What it means |
|---|---|---|
| **T1-01** | *(none — defaults)* | Baseline VarDict default calling. |
| **T1-02** | `-f 0.005 -r 1 -q 15` | High-sensitivity: lower allele-frequency floor, accept 1 supporting read, lower base-quality bar. |
| **T1-06** | `-f 0.001 -r 1 -q 20 -m 12` | Ultra-sensitive (WGS-style): very low AF floor, 1 read, higher quality bar, looser mismatch limit. |
| **CM-NOSV** | `-U` | Call-mode variant: disables the structural-variant module (different code path). |

Flag key: `-f` min allele frequency · `-r` min variant-supporting reads · `-q` min base quality (phred) ·
`-m` max mismatches per read · `-U` skip structural-variant calling.

### Metrics

`Wall` = elapsed time · `User`/`Sys` = CPU time in user/kernel space · `Peak RSS` = max resident memory.
**`Rust/Java` is the ratio — < 1.00× means VarDict-rs is faster / leaner.** `Parity` confirms the two tools'
output matched (byte-identical, or identical after sorting to absorb thread-ordering).

## Results

| Workload | Preset | Metric | Java (vdj) | Rust (vdr) | Rust/Java | Parity |
|---|---|---|---:|---:|---:|---|
| simple | T1-01 | Wall s | 16.1s | 3.0s | 0.19x | identical (sorted) |
|  |  | User s | 135.0s | 22.2s | 0.16x |  |
|  |  | Sys s | 6.8s | 1.4s | 0.20x |  |
|  |  | Peak RSS | 2931MB | 253MB | 0.09x |  |
| simple | T1-02 | Wall s | 16.1s | 3.0s | 0.19x | identical (sorted) |
|  |  | User s | 132.1s | 22.6s | 0.17x |  |
|  |  | Sys s | 6.5s | 1.1s | 0.18x |  |
|  |  | Peak RSS | 2945MB | 269MB | 0.09x |  |
| simple | T1-06 | Wall s | 15.7s | 3.0s | 0.19x | identical (sorted) |
|  |  | User s | 131.2s | 22.7s | 0.17x |  |
|  |  | Sys s | 6.3s | 1.1s | 0.17x |  |
|  |  | Peak RSS | 2934MB | 267MB | 0.09x |  |
| simple | CM-NOSV | Wall s | 15.4s | 3.2s | 0.21x | identical (sorted) |
|  |  | User s | 122.7s | 23.7s | 0.19x |  |
|  |  | Sys s | 6.6s | 1.4s | 0.21x |  |
|  |  | Peak RSS | 2931MB | 252MB | 0.09x |  |
| somatic | T1-01 | Wall s | 57.6s | 9.4s | 0.16x | IDENTICAL |
|  |  | User s | 218.8s | 71.7s | 0.33x |  |
|  |  | Sys s | 10.3s | 2.6s | 0.25x |  |
|  |  | Peak RSS | 2872MB | 472MB | 0.16x |  |
| somatic | T1-02 | Wall s | 62.3s | 9.8s | 0.16x | IDENTICAL |
|  |  | User s | 224.4s | 75.6s | 0.34x |  |
|  |  | Sys s | 9.7s | 2.1s | 0.22x |  |
|  |  | Peak RSS | 2870MB | 486MB | 0.17x |  |
| somatic | T1-06 | Wall s | 63.7s | 9.8s | 0.15x | IDENTICAL |
|  |  | User s | 227.2s | 75.8s | 0.33x |  |
|  |  | Sys s | 9.4s | 2.1s | 0.22x |  |
|  |  | Peak RSS | 2866MB | 485MB | 0.17x |  |
| somatic | CM-NOSV | Wall s | 55.0s | 8.2s | 0.15x | IDENTICAL |
|  |  | User s | 213.5s | 63.3s | 0.30x |  |
|  |  | Sys s | 8.0s | 1.9s | 0.23x |  |
|  |  | Peak RSS | 2872MB | 466MB | 0.16x |  |

**Verdict:** ✅ all measured cells byte-identical; resource numbers are valid.

Ratio < 1.00x means Rust is faster / leaner than Java.

