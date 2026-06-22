# Sweep Runtime Resources & Fixture Generation

Companion to [`workflow-structure.md`](workflow-structure.md) (§8 there links here). This is the
authoritative entry point for **e2e parity fixture generation**: where the sweep caches and BEDs
live, and the additive-safe runbook to generate them. Any agent doing sweep generation or sweep
parity must read this first.

The full-BAM/somatic sweep tiers (`parity_e2e_sweep`, `parity_e2e_sweep_somatic`) and the
generation scripts (`gen_e2e_sweep_golden.sh` → `sweep_fixtures_parallel.py`, `gen_sweep_bed.sh`,
`gen_somatic_sweep_bed.sh`) consume two kinds of large runtime data that do **not** live in the git
repo: cached Java golden **fixtures** and the **sweep BEDs** that tile them. Both already exist on
this workstation. **Reuse them — do not regenerate.** Regenerating a BED with different window
params produces a different tiling and a different `bed_sha256`, which then silently diverges from
the cache it is supposed to match (see the hg002 trap in §3).

## 1. Path indirection

- `tmp/sweep_fixtures` in this repo is a **symlink**, repointed per tag to a canonical cache dir on
  the data disk. Generation drivers repoint it; the sweep harness reads through it. (`workflow-structure.md`
  §4 "reads sweep cache from `tmp/sweep_fixtures/output/` by default" means *through this symlink*.)
- Env overrides: `VARDICT_E2E_SWEEP_FIXTURE_ROOT` (cache root), `VARDICT_E2E_SWEEP_BED_ROOT` (bed root).
- The canonical **sweep BEDs were already generated** and live in the **sibling repo**
  `/home/eck/workspace/vardict_rs2/tmp/sweep_beds/<tag>/`. That repo is **read-only** per the
  workspace boundary (only `vardict_rs_claude` is editable). Point `--sweep-bed-root` /
  `VARDICT_E2E_SWEEP_BED_ROOT` there; never overwrite the local `tmp/sweep_beds`. The local
  `tmp/sweep_beds/hg002` is a *different, non-canonical* tiling — do not use it.

## 2. Canonical caches and bed_sha256 (provenance)

All pinned to VarDictJava commit `4e362c0`. On this workstation the canonical caches live under
`/hdd-disk1/eck/vardict_rs2/`:

| Tag | Mode | Canonical cache dir | bed_sha256 of canonical BED |
|-----|------|---------------------|-----------------------------|
| `hg002` | single | `sweep_fixtures_hg002_allchrom` | `3c12d28207dc…` |
| `na12878_lowcov` | single | `sweep_fixtures_na12878_lowcov_sorted_20260530` | `8ae429e54ab4…` |
| `wes_il_pair` | somatic | `sweep_fixtures_somatic_wes_il_pair_sorted_20260530` | `b1bd4f24db95…` |
| `hg005_exome` | single | `sweep_fixtures_hg005_exome` | `d63810c88aee…` |

The matching BED for each tag lives in the sibling `tmp/sweep_beds/<tag>/` and reproduces the
`bed_sha256` above. **Verify `bed_sha256` against the cache `manifest.json` before generating** — a
mismatch means wrong tiles and the run must abort. `manifest.json → cache_entries[*].bed_sha256`
holds the expected value; the generation drivers enforce a hard sha-gate.

## 3. Beware duplicate / non-canonical caches

Several **non-canonical** sweep dirs also exist on the data disk (`sweep_fixtures`,
`sweep_fixtures_hg002_sorted_20260529`, `…_lowcov_resume2`, `…_lowcov_t1_corewide_provenance_20260530`,
`sweep_fixtures_somatic`). Only write to the canonical dirs in §2. The same tag can appear in
multiple caches with a **different tiling**: hg002 is `3c12d28…` in `_allchrom` but `08818cd9…`
inside `…na12878_lowcov_sorted_20260530`. Picking the wrong one yields spurious diffs.
**Disambiguate by `bed_sha256`, not by tag name.**

## 4. Discovery (verify, don't trust a possibly-moved path)

```bash
ls -d /hdd-disk1/eck/vardict_rs2/sweep_fixtures*       # cache dirs on the data disk
ls /home/eck/workspace/vardict_rs2/tmp/sweep_beds/     # canonical BEDs (sibling repo, read-only)
# bed_sha256 of a tag's canonical BEDs (run from this repo's opt/claude):
python3 -c "import sys; sys.path.insert(0,'scripts'); from pathlib import Path; \
from lib.merge_manifest import _discover_bed_paths,_sha256_concat; \
print(_sha256_concat(_discover_bed_paths(Path('/home/eck/workspace/vardict_rs2'), \
Path('/home/eck/workspace/vardict_rs2/tmp/sweep_beds'),'hg002')))"
```
Cross-check the printed sha against the target cache's `manifest.json` `cache_entries[*].bed_sha256`
before any generation run. If they differ, stop — do not generate against mismatched tiles.

## 5. Start here: generate e2e parity fixtures (runbook)

This is the canonical entry point for generating VarDictJava golden sweep fixtures. The generator is
`scripts/gen_e2e_sweep_golden.sh` (→ `scripts/sweep_fixtures_parallel.py`). It writes **through the
`tmp/sweep_fixtures` symlink** (hardcoded `OUTPUT_ROOT`) and does **not** repoint that symlink — you
must point it at the target cache first. Generation is **additive-only**: never overwrite, delete,
or regenerate existing cache entries.

Per tag, run from `opt/claude` with the build env active
(`conda activate vdr; export LIBCLANG_PATH=$CONDA_PREFIX/lib`):

```bash
TAG=hg002                                                   # or na12878_lowcov / wes_il_pair / hg005_exome
CACHE=/hdd-disk1/eck/vardict_rs2/sweep_fixtures_hg002_allchrom   # canonical dir for TAG from §2
BEDROOT=/home/eck/workspace/vardict_rs2/tmp/sweep_beds      # sibling, read-only (§1)
EXPECT=3c12d28207dc...                                      # full bed_sha256 for TAG from §2

# 1. SHA-GATE (hard abort on mismatch — §4 prints the actual sha)
#    actual = _sha256_concat(_discover_bed_paths(Path('/home/eck/workspace/vardict_rs2'), Path($BEDROOT), '$TAG'))
#    if actual != $EXPECT: STOP. Wrong tiling → spurious diffs.

# 2. Snapshot manifest (additive-safety baseline) + repoint the symlink at the canonical cache
cp "$CACHE/manifest.json" "tmp/manifest.snapshot.$TAG.before.json"
ln -sfn "$CACHE" tmp/sweep_fixtures

# 3. Generate each preset additively (add --somatic ONLY for wes_il_pair; --parallel 6 here)
bash scripts/gen_e2e_sweep_golden.sh --config CM-SAMFILT --tags "$TAG" --sweep-bed-root "$BEDROOT" --parallel 6
#    ... repeat per preset, or use --all-configs --config-tier <N>; for wes_il_pair add --somatic.

# 4. Verify additive: entries only grew, pre-existing entries byte-identical to the snapshot.
# 5. Run sweep parity: parity_e2e_sweep (germline) / parity_e2e_sweep_somatic --test-threads=1 (somatic).
```

Failure recovery: a failed `gen_e2e_sweep_golden.sh` run overwrites `tmp/sweep_fixtures/manifest.json`
with a degenerate run-summary (no `cache_entries`). The wrapper writes a pre-run backup
`<cache>/.manifest.cache_entries.before.json` — restore from it (and/or your step-2 snapshot). Do not
relaunch on a premature background "completed" notification; verify with `ps` and on-disk artifacts.
Valid `--tags`: `hg002,na12878_exome,na12878_lowcov,hg005_exome` (`na12878_exome` is out of current
scope). The 12 flag-coverage presets are `CM-SAMFILT CM-UNIQ CM-QRATIO CM-MEANMAPQ CM-TRIM CM-3PRIME
CM-READPOS CM-MINMATCH CM-DELDUP CM-DEBUG CM-UNIQUN CM-EXTEND`.
