# Parity Fixture Sources

This file records known large sweep fixture sources used by the full-BAM and
somatic parity gates. These fixtures are not stored in git. Before regenerating
Java fixtures, use this inventory to decide whether a gate failure is caused by
missing TSV content, stale sidecar metadata, a sorted-overlay requirement, or
actual cache corruption.

## Required Triage Before Regeneration

When `scripts/e2e_sweep_gate.py` reports cache/provenance warnings such as
`missing_generator_flags`, `mismatch_generator_flags`, `missing_bed_sha256`, or
`mismatch_bed_sha256`, do not immediately regenerate VarDictJava fixtures.

1. Find the freshest prior pass or failure report for the same tag under
   `tmp/parity-iteration/`.
2. Extract the report's fixture source from `diagnosis_artifact.fixture_source`
   or from the `VARDICT_E2E_SWEEP_FIXTURE_ROOT` commands in `progress.log`.
3. Compare that report's matrix scope against the current
   `scripts/config_presets.tsv` and requested chromosomes.
4. Inspect whether each requested cell has both `*.tsv.zst` and
   `*.chunks.json`.
5. Classify the issue:
   - `missing TSV`: Java fixture generation may be required.
   - `stale metadata`: build a metadata-only overlay; do not regenerate TSVs.
   - `sorted overlay needed`: reuse the sorted TSV overlay and refresh sidecars
     if necessary.
   - `cache corruption`: verify fingerprints and regenerate only the corrupt
     cells.

Only run VarDictJava fixture generation after TSV content is proven missing or
invalid. A metadata-only repair should preserve the existing TSV bytes and
rewrite only sidecar fields required by the current gate contract.

## Shared Paths

| Purpose | Path | Notes |
|---|---|---|
| Canonical sweep BED root | `/home/eck/workspace/vardict_rs2/tmp/sweep_beds` | Use with `--sweep-bed-root` / `VARDICT_E2E_SWEEP_BED_ROOT`. |
| Default staged fixture root | `tmp/sweep_fixtures` | Local symlink; verify target before trusting it. |
| Current all-BAM merged source | `tmp/sweep_fixture_sources/all-bams-no-na12878-exome-20260625` | Working source tree for all BAMs except `na12878_exome`. |

## HG002 WGS

| Purpose | Path | Scope | Status | Evidence |
|---|---|---|---|---|
| Last known post-merge pass source | `/home/eck/workspace/vardict_rs2/tmp/hg002-wgs-postmerge-dcd3684-staged-fixtures-20260616-t8/output` | 45 presets x 25 chroms | TSV-ready; some current metadata checks may be stale | `/home/eck/workspace/vardict_rs2/tmp/parity-iteration/hg002-wgs-postmerge-dcd3684-full-20260616-t8/parity-failure-report.json` |
| Sorted CM-PILEUP overlay | `/hdd-disk1/eck/vardict_rs2/sweep_fixtures_hg002_sorted_20260529/output` | CM-PILEUP x 25 chroms | Sorted TSV overlay, not full HG002 source | Use only when CM-PILEUP sorted output is needed. |
| Historical allchrom source | `/hdd-disk1/eck/vardict_rs2/sweep_fixtures_hg002_allchrom/output` | Partial current preset coverage | Contains many TSVs; sidecar metadata may be mixed or stale | Verify cell-by-cell before staging. |
| Current metadata overlay | `tmp/sweep_fixture_sources/all-bams-no-na12878-exome-20260625/hg002-metadata-overlay-v1/output` | 58 presets x 25 chroms | Metadata-only overlay over existing TSVs | Created to adapt existing HG002 TSVs to the current gate contract. |

Notes:

- The June 16 pass covered 45 presets. The current preset matrix has more
  presets, so do not treat the June 16 source alone as a complete current
  all-preset source.
- The 13 newer HG002 presets have existing TSVs in the current merged source:
  `CM-ADAPTOR`, `CM-SAMFILT`, `CM-UNIQ`, `CM-UNIQUN`, `CM-QRATIO`,
  `CM-MEANMAPQ`, `CM-TRIM`, `CM-EXTEND`, `CM-3PRIME`, `CM-DEBUG`,
  `CM-READPOS`, `CM-MINMATCH`, and `CM-DELDUP`.
- The sorted root `sweep_fixtures_hg002_sorted_20260529` is not the complete
  HG002 all-preset source.

## HG005 Exome

| Purpose | Path | Scope | Status | Evidence |
|---|---|---|---|---|
| Existing fixture source | `/hdd-disk1/eck/vardict_rs2/sweep_fixtures_hg005_exome/output` | Current source has broad preset coverage | TSV-ready; verify metadata with the active gate before diagnosis | `manifest.json` under the source root |

## NA12878 Low-Coverage WGS

| Purpose | Path | Scope | Status | Evidence |
|---|---|---|---|---|
| Existing sorted fixture source | `/hdd-disk1/eck/vardict_rs2/sweep_fixtures_na12878_lowcov_sorted_20260530/output` | Broad preset/chromosome coverage | TSV-ready for known generated presets | `manifest.json` under the source root |

Notes:

- `CM-UNIQUN` for `na12878_lowcov` has failed Java generation on unpaired reads
  in prior current-workspace attempts. Treat that as fixture-generation
  infrastructure until revalidated, not as Rust parity evidence.

## Somatic WES IL Pair

| Purpose | Path | Scope | Status | Evidence |
|---|---|---|---|---|
| Existing sorted somatic source | `/hdd-disk1/eck/vardict_rs2/sweep_fixtures_somatic_wes_il_pair_sorted_20260530/output` | `wes_il_pair` pair fixtures | TSV-ready for known generated presets; somatic generator flags are diagnostic-only at chunk level | `manifest.json` under the source root |

## Metadata Overlay Rules

A metadata-only overlay is appropriate when TSV content exists and sidecar
fingerprints match the TSV bytes, but gate provenance fields are missing or
encoded in an older form.

Overlay sidecars must preserve or recompute:

- `monolithic_md5`
- `monolithic_bytes`
- `chunks`
- `vardict_commit`

Overlay sidecars may update only gate provenance fields:

- `preset`
- `generator_flags`
- `bed_sha256`

Record the source TSV and source sidecar path in a local overlay metadata field
so the overlay can be audited later.
