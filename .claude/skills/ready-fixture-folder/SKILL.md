---
name: ready-fixture-folder
description: >
  Assemble or refresh a tag's COMPLETE, gate-ready E2E-sweep fixture folder — the
  parity-workflow-ready structure that tests point at
  (testdata/fixtures/e2e_sweep2 -> /hdd-disk1/.../e2e_sweep2_fixtures/<tag>): payloads +
  chunks.json sidecars + manifest.json + provider-map, passing the 7 read-time gates and
  readiness. Use when: a completed fixture set exists (in a /hdd-disk1 source folder or a
  cache) and must be installed/organized into the parity-ready structure, the fixture
  folder is incomplete/stale/not-ready, new presets or chroms must be folded into the
  composite, manifest cache_entries or provider-map need rebuilding, or readiness/gate
  reports missing shards or mismatch_generator_flags. This skill is about STRUCTURE +
  READINESS, not producing goldens — regenerate only as an optional prerequisite when the
  source set is missing cells. Do NOT use to fix a real vdr bug (use mismatch-repair) or to
  decide which cells are stale (use config-e2e-diagnosis / dual_run first).
---

# Ready Fixture Folder (build the parity-workflow-ready structure)

Take a **completed fixture set** (payloads + `chunks.json` sidecars for every
`(preset, chrom)` cell of a tag) and assemble/refresh the **parity-workflow-ready
structure** so the E2E sweep gate + readiness pass. Read `tests/SWEEP_FIXTURES.md` first —
it is the authoritative inventory. The acceptance bar is the **7 read-time gates** + a
green readiness run, not "did we regenerate".

## The target structure (what "ready" means)

```
testdata/fixtures/e2e_sweep2                 # repo symlink ->
  -> /hdd-disk1/eck/vardict_rs2/e2e_sweep2_fixtures     # durable source folder
       <tag>/
         manifest.json          # gate provenance: cache_entries keyed "<config>:<tag>"
         output/<preset>/<chrom>/
           <tag>_<chrom>.tsv.zst     # payload (real file; hardlink where same-FS)
           <tag>_<chrom>.chunks.json # per-cell provenance sidecar (real, small)
         provider-map.tsv        # per (preset,chrom): source tsv/chunks path + provider label
         README.md, run_summary.json
```

- `manifest.json` **cache_entries** (one per `<config>:<tag>`) carry the gated provenance:
  `bam_stat` `[{path,size,mtime_unix}]`, `bed_sha256`, `generator_flags_hash`,
  `reference_sha256`, `tag`, `vardictjava_commit`.
- `chunks.json` (per cell) carry `monolithic_md5`/`monolithic_bytes`, `num_chunks`,
  `chunks`, `generator_flags` (the raw VarDict flags — must equal `config_presets.tsv[preset]`,
  germline / pair-form somatic), `bed_sha256`, `preset`, `vardict_commit`.

## The 7 read-time gates (the definition of "ready" — SWEEP_FIXTURES.md §5)
1 `vardictjava_commit` == live `git -C VarDictJava HEAD` (pinned `4e362c0`).
2 cache entry present for the cell. 3 `bed_sha256` (sha of `<bed_root>/<tag>/*.bed`).
4 `bam_stat {size, mtime_unix}` (⚠️ mtime-keyed — a touched BAM fails). 5 `reference_sha256`
(sha of `<ref>.fai`). 6 `generator_flags_hash` (embeds the bed-root path string). 7 output
shards present & non-empty. A separate content check also requires
`chunks.json.generator_flags == config_presets.tsv[preset]` (else `mismatch_generator_flags`
→ not-ready → blocks `config-e2e-diagnosis`).

## Environment
```bash
source "$(conda info --base)/etc/profile.d/conda.sh"; conda activate vdr
export LIBCLANG_PATH="$CONDA_PREFIX/lib"
export VARDICT_E2E_SWEEP_BED_ROOT=/home/eck/workspace/vardict_rs2/tmp/sweep_beds  # canonical BEDs (sibling, read-only)
```

## Procedure

### 1. Confirm the source set is complete
The input is a **completed fixture set** — a folder with `output/<preset>/<chrom>/*.tsv.zst`
+ `*.chunks.json` for every cell in scope (e.g. a `/hdd-disk1/.../regen_<tag>_<date>/output`
tree, or an existing cache). If cells are missing, produce them first (optional prerequisite
— generator `scripts/sweep_fixtures_parallel.py`; byte-identical output, records the correct
`generator_flags`). Verify byte-truth of any freshly produced cell with a scoped
`scripts/e2e_sweep_gate.sh` (PASS 0-fail/0-warn) before installing.

### 2. Install payloads into the durable per-tag folder
Place each cell's `*.tsv.zst` + `*.chunks.json` into
`/hdd-disk1/eck/vardict_rs2/e2e_sweep2_fixtures/<tag>/output/<preset>/<chrom>/`.
- Prefer **hardlink** when the source is on the same FS (cheap, shared inode; payloads show
  `links>1`); **copy** for cross-FS sources.
- `chunks.json` are `links=1` — safe to (re)write per cell (back up first).
- ⚠️ Never overlay a **shared provider** in place (`tmp/sweep_fixtures` is a symlink to one);
  the `e2e_sweep2_fixtures/<tag>` durable folder is the intended target.

### 3. Build / merge `manifest.json` cache_entries
Use the manifest merger (the same one `gen_e2e_sweep_golden.sh` and the gate use):
```bash
python3 -m scripts.lib.merge_manifest cache-entries \
  --config <PRESET> --tags <tag> --logical-flags "<config_presets.tsv flags>" \
  --project-root "$PWD" --sweep-bed-root "$VARDICT_E2E_SWEEP_BED_ROOT" \
  --manifest-path /hdd-disk1/.../e2e_sweep2_fixtures/<tag>/manifest.json
```
Subcommands: `cache-entries` / `cache-entries-many` (germline, one/many configs),
`cache-entries-somatic` / `cache-entries-somatic-many` (pair tags). This computes
`bed_sha256`, `generator_flags_hash`, `reference_sha256`, `bam_stat` for each `<config>:<tag>`.
Keep a backup (`.manifest.cache_entries.before.json`) — a failed merge can clobber the manifest.

### 4. Refresh `provider-map.tsv`
One row per `(preset, chrom)`: `preset  chrom  <tsv path>  <chunks path>  <provider label>`.
Records where each payload was sourced (provenance/audit); regenerate for changed cells.

### 5. Wire the repo pointer
Ensure `testdata/fixtures/e2e_sweep2` symlinks to
`/hdd-disk1/eck/vardict_rs2/e2e_sweep2_fixtures` (already the case; verify after any move).
Tests point `VARDICT_E2E_SWEEP_FIXTURE_ROOT` at `testdata/fixtures/e2e_sweep2/<tag>`.

### 6. Verify readiness + gate
```bash
VARDICT_E2E_SWEEP_FIXTURE_ROOT=$(pwd)/testdata/fixtures/e2e_sweep2/<tag> \
  cargo test --profile debug-release --test parity_e2e_sweep <tag>_sweep::readiness \
  -- --ignored --nocapture --test-threads=1     # 7 gates, no vdr run
```
All cells must pass; then run the full-scope parity gate (`scripts/e2e_sweep_gate.sh`) green.
Somatic: `wes_il_pair_sweep::readiness_all_configs`, `--test-threads=1`.

## Landmines
- Never bump VarDictJava off `4e362c0` (gate #1). Never touch input BAMs (gate #4, mtime) or
  regenerate BEDs (gate #3, `bed_sha256`). Canonical BEDs are the sibling
  `/home/eck/workspace/vardict_rs2/tmp/sweep_beds`.
- Never write into `tmp/sweep_fixtures` (shared provider symlink).
- The session scratchpad gets cleaned mid-task — keep inventories/scripts/backups on `/hdd-disk1`.
- A failed manifest merge can clobber `manifest.json` — keep the `.before.json` backup and restore.
- `na12878_lowcov` has 57 cells by design (no `CM-UNIQUN`); other tags 58.

## Related skills / files
`tests/SWEEP_FIXTURES.md` (structure inventory), `scripts/lib/merge_manifest.py` (manifest
builder), `scripts/gen_e2e_sweep_golden.sh` (generate+manifest orchestrator),
`config-e2e-diagnosis` (classifies stale cells, calls here to refresh the folder),
`mismatch-repair` (real vdr bug — not a fixture problem), `workflow-management` (register new
generator/gate tooling).
