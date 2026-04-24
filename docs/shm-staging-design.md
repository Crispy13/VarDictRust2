# Hybrid Shm Staging Design

Target file: `scripts/lib/shm_slice.py` (primary), `scripts/sweep_fixtures_parallel.py` (minor adjustments)

## Context

Investigation established that `samtools view -b` + `samtools index` re-indexing
destroys BAI bin granularity (45% fewer chunks) for read-concentrated BAMs
like exomes, producing a 38% VarDict read penalty. See
`docs/bai-granularity-findings.md` for root-cause analysis.

## Strategy

Size-threshold-based hybrid:

| branch | trigger | mechanism | preserves dense BAI? |
|---|---|---|---|
| A. cp | source BAM ≤ 2 GB | `cp` BAM+BAI+FASTA+FAI into /dev/shm | yes (byte-identical) |
| B. slice | source BAM > 2 GB | `samtools view -b -@ 10 --write-index -o <shm>/<tag>.bam##idx##<shm>/<tag>.bam.bai <src> <chrom>` | no (but fast; acceptable for WGS; unavoidable for big exomes) |

### Size threshold: 2 GB

Rationale:
- NA12878 exome = 374 MB → branch A
- HG002 exome = 10 GB → branch B
- NA12878 low_cov WGS = 15 GB → branch B
- WES_IL pair = 12-14 GB each → branch B

Small enough to be fast to cp; large enough that big files always slice.
Configurable via `SHM_CP_MAX_BYTES` constant.

### Shm budget check

Before staging, check:
- For branch A: free shm space ≥ (source BAM + BAI size) × 1.1 safety margin
- For branch B: free shm space ≥ (est. chrom slice size) × 1.1
  - Chrom slice size estimated as `source_bam_size * chrom_read_fraction`
  - Use `samtools idxstats` output or a conservative 0.1 × source size heuristic

If budget insufficient → fall back to `--no-shm` path for this shard.

### Sample name handling

- Branch A: source basename preserved through cp → no `-N` override needed
- Branch B: slice file named `<tag>.bam`; `-N <source_basename>` override
  remains required (existing logic in `sweep_fixtures_parallel.py`)

### FASTA handling (unchanged)

- Copy FASTA region via `samtools faidx <ref> <chrom>` OR cp full FASTA
  depending on FASTA size vs shm budget.
- Validate header first line matches source (existing `_validate_fasta_header`).

## Implementation steps

1. Add `_cp_stage(...)` helper to `shm_slice.py` that copies BAM, BAI,
   FASTA, FAI into shm with content hashing / checksums if needed.
2. Add `_slice_stage(...)` helper that invokes
   `samtools view -b -@ 10 --write-index -o <path>##idx##<path>.bai <src> <chrom>`
   (replaces current `view -b` + `index` two-step).
3. Add `SHM_CP_MAX_BYTES = 2 * 1024**3` constant.
4. In `stage_chrom(...)`, branch on `bam_source.stat().st_size` ≤ threshold.
5. Add shm capacity pre-check.
6. In `sweep_fixtures_parallel.py`, conditionally drop `-N` override when
   branch A was used (lookup the staged BAM path and compare basename).

## Tests / validation

Before declaring done:

1. **T1-01 parity gate (byte-identical to legacy)**: md5
   `0442b62e0899cc2fa1e15251619e091b` on 42,931 rows.
2. **T1-01 `--no-shm` fallback**: same md5.
3. **Per-tag staging dry-runs**: one shard per tag (hg002, na12878_exome,
   na12878_lowcov, wes_il_pair) exercising both branches.
4. **Full 44-preset w=10 bench**: target < 50 min (yesterday's baseline).
   Compare to 56m38s Option F result.

## Acceptance criteria

- All T1-01 parity tests pass byte-identically (md5 match).
- Full 44-preset run completes with all shards success.
- 44-preset wall time < 56m38s (improvement over current Option F).
- No shm leakage on failure (atexit cleanup intact).
- Code passes lint/type checks.

## Non-goals

- Custom BAI generation (ruled out — no public htslib API; engineering cost
  too high for remaining marginal gain).
- Memfd backend (benchmarks showed ~equivalent to /dev/shm; complexity not
  justified).
- Replacing VarDict runner with Rust port (separate, ongoing work).
