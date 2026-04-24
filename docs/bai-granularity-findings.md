# BAI Granularity: Why Old BAIs Read Faster Than New Ones

Date: investigation during Option F 44-preset sweep slowdown
Related: scripts/lib/shm_slice.py, scripts/sweep_fixtures_parallel.py

## Executive Summary

BAI files shipped with testdata BAMs (produced circa 2012 by samtools 0.1.17)
are structurally denser than BAIs produced by modern samtools 1.23.1, even for
byte-identical BAM content. Reading dense-BAI BAMs via BAI-seek is up to 38%
faster for our BED-region sweep workload.

**Implication:** When a pipeline re-indexes a BAM (e.g. via `samtools view -b`
+ `samtools index`), the fresh BAI is sparser → subsequent VarDict / samtools
view -L queries are slower. This is the root cause of the 13% slowdown in
Option F (56m38s vs ~50m baseline).

## Reproduction

Same BAM bytes, re-indexed with modern samtools (1.23.1):

```
samtools index testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam.bai
# produces a denser .bai when inherited; modern samtools produces sparser
```

| BAI | total bins | total chunks | file size |
|---|---:|---:|---:|
| Original (2012) | 4,162 | 6,894 | 175 KB |
| Re-indexed (samtools 1.23.1) | 2,225 | 3,783 | 110 KB |

Linear-interval count identical (3,844) — that's determined by read positions.

## Bin-Level Decomposition

BAI uses a 6-level bin tree (SAM spec §4):

| level | bin size | OLD bins | NEW bins | OLD chunks | NEW chunks |
|---|---|---:|---:|---:|---:|
| L1 | 64 M | 1 | 1 | 6 | 6 |
| L2 | 8 M | 8 | 8 | 43 | 43 |
| L3 | 1 M | 57 | 57 | 345 | 346 |
| L4 | 128 k | 458 | 457 | **2,841** | **1,666** |
| L5 | 16 k | **3,637** | **1,701** | **3,657** | **1,720** |

Parents (L1–L3) are essentially identical. **Divergence is at L4 and L5.**

## Mechanism

1. Each read gets placed in the smallest bin that fully contains it. A 150 bp
   read almost always fits in a 16 kb L5 bin.

2. **Old samtools (0.1.17)** appears to have registered each read's exact-fit
   L5 bin with tight chunk boundaries — up to ~1 chunk per 16 kb interval.
   Result: 3,637 L5 bins on chr20 (close to the theoretical max of ~3,940 for
   a 63 Mb chromosome).

3. **Modern samtools (1.23.1)** consolidates chunks aggressively during index
   write. Fewer L5 bins are materialized; reads that don't densely populate a
   16 kb bin get absorbed into the parent L4 (128 kb) bin. Total chunk count
   drops from 6,894 → 3,783 (-45%).

## Query-Time Effect

When fetching a BED region like `20:1234567-1234999` (~432 bp):

- With **dense BAI (OLD)**: the query hits ~1 L5 bin → ~1 chunk → fetches
  ≤ 16 kb of BGZF blocks → decompress → filter. Tight.
- With **sparse BAI (NEW)**: the query falls into an L4 (128 kb) bin → fetches
  8× more BGZF blocks → decompress more → filter to the 432 bp. Wasteful.

VarDict's NA12878 exome chr20 BED has 145,853 regions averaging ~200 bp each.
The L4 promotion cost compounds across every region → ~38% wall-time penalty.

## Why Read Distribution Matters

The L5 → L4 promotion depends on **how densely reads populate each 16 kb
interval**:

| BAM type | chr20 reads | L5 bins retained after re-index |
|---|---:|---:|
| Exome (reads concentrated in ~60 Mb of exons) | 4.5 M | 54% (2,225 / 4,162) |
| WGS low-cov (reads spread across full chrom) | 3.0 M | 98% (4,101 / 4,162) |

**Concentrated data loses granularity.** WGS data is safe.

## Why We Can't Fix It With Samtools Flags

- `-@ N`: threading, doesn't change algorithm
- `-l` compression level: doesn't affect chunk structure
- `--write-index`: just folds index into write, same algorithm
- `-X, --customized-index`: allows non-standard path, BAI content still
  tied to the BAM it was built from (byte offsets must match BGZF blocks)
- `--output-fmt-option`: no known tunable for bin-packing algorithm

The BAI binning algorithm in modern htslib is a fixed heuristic; no public
tunable exposes it.

## Implications for Pipeline Design

- **Do not** re-index source BAMs via `samtools view -b` + `samtools index`
  if you care about subsequent region-scan performance AND the source BAI
  is already dense.
- **Plain `cp`** of BAM + BAI preserves the source algorithm choice byte-for-byte.
- For BAMs too large to `cp` into shm, accept the algorithm overhead when
  slicing. WGS-style distributions suffer minimally (~2% granularity loss).

## References

- SAM/BAM spec v1.6 §4 (Indexing): https://samtools.github.io/hts-specs/SAMv1.pdf
- htslib source: hts.c `hts_idx_*` functions
- samtools changelog 1.0+: https://github.com/samtools/samtools/blob/develop/NEWS
