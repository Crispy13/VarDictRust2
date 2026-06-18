# Production bench — VarDictJava (vdj) vs VarDict-rs (vdr)

Machine load at start: `19:14:48 up 11:50,  2 users,  load average: 0.45, 1.55, 4.59`

Runs timed per cell: 3 (median; 1 warmup discarded). Thread count: -th 8.

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
| simple | T1-01 | Wall s | 14.6s | 3.0s | 0.21x | identical (sorted) |
|  |  | User s | 124.8s | 22.8s | 0.18x |  |
|  |  | Sys s | 5.1s | 1.0s | 0.20x |  |
|  |  | Peak RSS | 2922MB | 253MB | 0.09x |  |
| simple | T1-02 | Wall s | 14.8s | 3.1s | 0.21x | identical (sorted) |
|  |  | User s | 124.8s | 23.1s | 0.19x |  |
|  |  | Sys s | 5.0s | 1.1s | 0.22x |  |
|  |  | Peak RSS | 2928MB | 268MB | 0.09x |  |
| simple | T1-06 | Wall s | 14.6s | 3.2s | 0.22x | identical (sorted) |
|  |  | User s | 126.4s | 24.3s | 0.19x |  |
|  |  | Sys s | 5.3s | 1.0s | 0.19x |  |
|  |  | Peak RSS | 2958MB | 268MB | 0.09x |  |
| simple | CM-NOSV | Wall s | 14.2s | 2.9s | 0.20x | identical (sorted) |
|  |  | User s | 116.8s | 21.7s | 0.19x |  |
|  |  | Sys s | 4.9s | 0.9s | 0.19x |  |
|  |  | Peak RSS | 2935MB | 252MB | 0.09x |  |
| somatic | T1-01 | Wall s | 57.9s | 9.3s | 0.16x | IDENTICAL |
|  |  | User s | 221.9s | 71.8s | 0.32x |  |
|  |  | Sys s | 8.1s | 2.2s | 0.27x |  |
|  |  | Peak RSS | 2866MB | 471MB | 0.16x |  |
| somatic | T1-02 | Wall s | 59.3s | 9.8s | 0.16x | IDENTICAL |
|  |  | User s | 222.7s | 75.3s | 0.34x |  |
|  |  | Sys s | 7.6s | 2.2s | 0.29x |  |
|  |  | Peak RSS | 2872MB | 482MB | 0.17x |  |
| somatic | T1-06 | Wall s | 62.4s | 9.7s | 0.16x | IDENTICAL |
|  |  | User s | 226.5s | 75.1s | 0.33x |  |
|  |  | Sys s | 8.0s | 2.1s | 0.26x |  |
|  |  | Peak RSS | 2898MB | 485MB | 0.17x |  |
| somatic | CM-NOSV | Wall s | 52.6s | 9.3s | 0.18x | IDENTICAL |
|  |  | User s | 208.9s | 71.7s | 0.34x |  |
|  |  | Sys s | 7.0s | 2.1s | 0.31x |  |
|  |  | Peak RSS | 2879MB | 469MB | 0.16x |  |

**Verdict:** ✅ all measured cells byte-identical; resource numbers are valid.

Ratio < 1.00x means Rust is faster / leaner than Java.

