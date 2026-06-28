# Sweep E2E Fixture Inventory (read this before running sweep parity)

Where the full-BAM sweep parity fixtures live and how to point a parity/readiness test at them.

**The fixture root you point a test at is a per-tag root under `testdata/fixtures/e2e_sweep2/<tag>`** —
a clean, one-folder-per-tag overlay (see §2) that this project owns. The heavy golden payloads live in
upstream caches on `/hdd-disk1` (§3) and are referenced by symlink. (`testdata/fixtures/e2e_sweep/` is a
parallel set owned/refreshed by another agent; `e2e_sweep2` is decoupled from it — they only share the
`/hdd-disk1` upstream.)

**Verified:** 2026-06-26 by the readiness tests (§6). VarDictJava commit recorded by every cache:
`4e362c0ccdae9bba4e378e9f40b15e85ed875d8e` (matches live).

## 1. Canonical per-tag fixture ROOTS (use these)

Point `VARDICT_E2E_SWEEP_FIXTURE_ROOT` (germline) / `…_SOMATIC_FIXTURE_ROOT` env at one of these:

| Tag | Mode | Fixture root (`testdata/fixtures/e2e_sweep2/`) | Entries | Readiness (2026-06-26) |
|-----|------|------------------------------------------------|---------|------------------------|
| `hg002` | germline | `hg002` | 58 | ✅ 58/58 PASS |
| `na12878_lowcov` | germline | `na12878_lowcov` | 57 | ✅ 57/57 PASS |
| `hg005_exome` | germline | `hg005_exome` | 58 | ✅ 58/58 PASS |
| `wes_il_pair` | somatic | `wes_il_pair` | 58 | ✅ 58/58 PASS |

These roots are **cleaner and more complete than the raw `/hdd-disk1` caches**: one tag + one `bed_sha`
per root (the old na12878 hg002-contamination is gone), and they collect presets across providers (e.g.
hg002 has `CM-ADAPTOR` + `CM-TH4` → 58 vs the raw cache's 56). They also normalize the
`generator_flags_hash` convention so the formerly-split presets all pass together (§7). They were derived
from the other agent's `e2e_sweep/<tag>_current` composites but copy the small metadata locally and
symlink the heavy payloads to `/hdd-disk1`, so they are independent of `e2e_sweep/`.

BAM/ref inputs (in repo `testdata/`, the inputs vdr reads; **not** the fixtures):

| Tag | BAM(s) | Reference |
|-----|--------|-----------|
| `hg002` | `151002…HG002…posiSrt.markDup.bam` | `hs37d5.fa` |
| `na12878_lowcov` | `NA12878.mapped…low_coverage…bam` | `hs37d5.fa` |
| `hg005_exome` | `151002…HG005…posiSrt.markDup.bam` | `hs37d5.fa` |
| `wes_il_pair` | `WES_IL_T_1.bwa.dedup.bam` \| `WES_IL_N_1.bwa.dedup.bam` | `GRCh38.d1.vd1.fa` |

Code refs: `tests/common/mod.rs::BAM_TAG_MAP`, `…_somatic/somatic_common.rs::SOMATIC_BAM_PAIR_MAP`.

## 2. What a fixture root contains

Each `e2e_sweep2/<tag>/` is a **thin overlay** (~18M), not a copy of the GB-scale golden output:

- `manifest.json` — per-tag, single `bed_sha`, the gated provenance (copied, small).
- `output/<preset>/<chrom>/` — real `*.chunks.json` sidecars (copied) + the `*.tsv.zst` payload as a
  **symlink** into the upstream `/hdd-disk1` cache (so the heavy data isn't duplicated).
- `provider-map.tsv` — for each `(preset, chrom)`: the source `tsv`/`chunks` path + provider.
- `README.md` — what this root is.

Derived 2026-06-26 from the other agent's `e2e_sweep/<tag>_current` composites, but self-contained: the
small metadata is copied locally and the `*.tsv.zst` symlinks point straight at `/hdd-disk1` (absolute),
so `e2e_sweep2` does not depend on `e2e_sweep/`.

## 3. Durable per-tag payload home: `fixtures_by_tag/<tag>`

`e2e_sweep2`'s `*.tsv.zst` symlinks all resolve to **`/hdd-disk1/eck/vardict_rs2/fixtures_by_tag/<tag>`**
— a real, self-contained one-folder-per-tag payload store built 2026-06-26 (hardlinks where same-FS,
copies for cross-FS providers; ~+2G total). Every payload `e2e_sweep2` references lives here, so the
fixture set survives even if the original provider dirs are later cleaned. Sizes: hg002 19G,
na12878_lowcov 45G, hg005_exome 5.3G, wes_il_pair 7.3G.

Provenance — the **original providers** these were materialized from (per `e2e_sweep2/<tag>/provider-map.tsv`):
| Tag | Primary canonical cache | bed_sha256 | Notes |
|-----|-------------------------|-----------|-------|
| `hg002` | `sweep_fixtures_hg002_allchrom` | `3c12d282…` | **multi-provider** — also `sweep_fixtures` (337G) + `sweep_fixtures_hg002_sorted_20260529` + several `vardict_rs2/tmp/{sweep_fixtures,sweep_fixture_sources,sweep_stage}` (cross-FS, copied) |
| `na12878_lowcov` | `sweep_fixtures_na12878_lowcov_sorted_20260530` | `8ae429e5…` | single-provider |
| `hg005_exome` | `sweep_fixtures_hg005_exome` | `d63810c8…` | single-provider |
| `wes_il_pair` | `sweep_fixtures_somatic_wes_il_pair_sorted_20260530` | `b1bd4f24…` | single-provider |

Canonical BED content (for the `bed_sha256` gate): `/home/eck/workspace/vardict_rs2/tmp/sweep_beds/<tag>`
(read-only sibling). The local `tmp/sweep_beds/` is non-canonical/incomplete — don't use it.

## 4. Preset coverage

The 12 flag-coverage presets: `CM-SAMFILT CM-UNIQ CM-QRATIO CM-MEANMAPQ CM-TRIM CM-3PRIME CM-READPOS
CM-MINMATCH CM-DELDUP CM-DEBUG CM-UNIQUN CM-EXTEND` (plus the originals: default/T1/T2/T3, PW, other CM-*).

`CM-UNIQUN` is **absent on `na12878_lowcov` by design** (VarDictJava crashes on its 930k unpaired reads —
its own operand-order bug; see [known-parity-gaps.md](../docs/known-parity-gaps.md)). That's why the
na12878 composite is 57, not 58.

## 5. The 7 read-time gates

`check_e2e_sweep_manifest` (germline, `tests/parity_e2e_sweep/common.rs`) / `check_somatic_manifest`
(somatic, `tests/parity_e2e_sweep_somatic/somatic_common.rs`) recompute and require a match:
1. `vardictjava_commit` == live `git -C VarDictJava HEAD`.
2. cache entry present.
3. `bed_sha256` (sha of `*.bed` in `<bed_root>/<tag>`).
4. `bam_stat` `{size, mtime_unix}` — ⚠️ **mtime-keyed** (a touched BAM fails; no drift as of 2026-06-26).
5. `reference_sha256` (sha of `<ref>.fai`).
6. `generator_flags_hash` — embeds the bed-root path string (§7).
7. output shards present & non-empty (`output/<config_seg>/<chrom>/<tag>_<chrom>.tsv.zst`).

## 6. Readiness verification (fast: gates + shards, no vdr run)

Reuses the real gate fns (zero drift): germline `<tag>_sweep::readiness_all_configs` (libtest_mimic
trial, `tests/parity_e2e_sweep/common.rs`); somatic `wes_il_pair_sweep::readiness_all_configs`
(`#[test]`) → `somatic_common.rs::verify_readiness`.

```bash
source /home/eck/software/miniconda3/etc/profile.d/conda.sh; conda activate vdr
export LIBCLANG_PATH=/home/eck/software/miniconda3/envs/vdr/lib
export VARDICT_E2E_SWEEP_BED_ROOT=/home/eck/workspace/vardict_rs2/tmp/sweep_beds
VARDICT_E2E_SWEEP_FIXTURE_ROOT=$(pwd)/testdata/fixtures/e2e_sweep2/hg002 \
  cargo test --profile debug-release --test parity_e2e_sweep hg002_sweep::readiness -- --ignored --nocapture --test-threads=1
```
Latest result: **all 4 green** — hg002 58/58, na12878_lowcov 57/57, hg005_exome 58/58, wes_il_pair
(somatic) 58/58 PASS.

## 7. generator_flags_hash convention

Gate #6 hashes the bed-root *path string*.
- **In the germline composites** the recorded value is normalized to the **absolute**
  `/home/eck/workspace/vardict_rs2/tmp/sweep_beds`, uniformly across all presets — so with that
  `VARDICT_E2E_SWEEP_BED_ROOT` exported, **all presets pass together** (the raw-cache split is resolved).
- **Somatic gate — FIXED 2026-06-26.** `check_somatic_manifest`'s `compute_generator_flags_hash`
  (`somatic_common.rs`) reconstructed the generator's flag string with one space where
  `gen_e2e_sweep_golden.sh:161` emits **two** (`--tags  --sweep-bed-root`, from the empty `--tags ""`
  value), so every somatic entry failed gate #6. Corrected the template to two spaces → both `e2e_sweep`
  and `e2e_sweep2` now pass 58/58. No fixture/manifest change.
- For historical context: the **raw `/hdd-disk1` caches** carry a two-convention split (44 originals =
  relative `tmp/sweep_beds`; 12 backfilled = absolute). The composites supersede that for germline.

## 8. How to run a parity test (against a composite root)

```bash
export VARDICT_E2E_SWEEP_BED_ROOT=/home/eck/workspace/vardict_rs2/tmp/sweep_beds
# germline
VARDICT_E2E_SWEEP_FIXTURE_ROOT=$(pwd)/testdata/fixtures/e2e_sweep2/hg002 \
VARDICT_E2E_SWEEP_CONFIG=CM-EXTEND \
  cargo test --profile debug-release --test parity_e2e_sweep hg002_sweep:: -- --include-ignored
# somatic (--test-threads=1; GlobalReadOnlyScope) — note §7 gate gap currently
VARDICT_E2E_SWEEP_FIXTURE_ROOT=$(pwd)/testdata/fixtures/e2e_sweep2/wes_il_pair \
VARDICT_E2E_SWEEP_SOMATIC_CONFIG=CM-EXTEND \
  cargo test --profile debug-release --test parity_e2e_sweep_somatic wes_il_pair_sweep:: -- --include-ignored --test-threads=1
```

## 9. Other `/hdd-disk1` dirs — ⚠️ several are live PROVIDERS, not junk

**Correction (2026-06-26):** an earlier version of this file called `sweep_fixtures` (337G) and
`sweep_fixtures_hg002_sorted_20260529` "stale/junk, safe to delete." **That was wrong.** They are
**active hg002 providers** — the other agent's `e2e_sweep/hg002_current` composite still symlinks into
them (1043 + 25 hg002 payloads). Deleting them would break `e2e_sweep`. `e2e_sweep2` no longer needs
them (its payloads are materialized into `fixtures_by_tag/hg002`), but they are **not** deletable while
`e2e_sweep` references them.

Genuinely superseded/scratch (still verify before any action): `sweep_fixtures_somatic` (pre-backfill
44-entry wes_il_pair), `*_resume2`, `*_t1_corewide_provenance`. The raw na12878 cache manifest also holds
2 stray `hg002` keys (`CM-PILEUP`, `CM-TH4`) — excluded from `e2e_sweep2`.

`/hdd-disk1/eck/vardict_rs2/fixtures_by_tag/<tag>` is now the **durable real-file payload home** (§3), not
the old symlink view. The ~373G cleanup is **on hold** — re-audit providers (via every overlay's
provider-map) before moving/deleting anything.

## 10. Caveats

- Fixtures + upstream caches are on `/hdd-disk1` + the self-hosted runner; only the thin composite overlay
  (sidecars/symlinks/manifest) is under `testdata/` (untracked).
- The `vardictjava_commit` gate breaks if VarDictJava is bumped off `4e362c0c…`.
- Nightly (`.github/workflows/sweep.yml`) runs `default` config only — neither the 12 backfilled nor the
  originals are auto-iterated.
- `na12878_exome` is out of scope (its composite is empty).
