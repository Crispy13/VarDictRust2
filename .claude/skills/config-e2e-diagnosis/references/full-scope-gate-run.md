# Full-Scope Gate Run Runbook

This is the concrete HOW for Canonical Contract Step 1 ("Run the active gate at its
full declared scope"). A cold session should be able to read this file and run the
full 4-tag/all-presets E2E parity gate to a verified 100% with zero external research.

## 4-Tag Scope Table

| tag | kind | expected green |
|---|---|---|
| hg002 | germline | 58/58 |
| hg005_exome | germline | 58/58 |
| na12878_lowcov | germline | **57/57** — CM-UNIQUN is a justified/documented gap (VarDictJava's own `--UN` unpaired-read crash), NOT a fail. See [[cm-uniqun-unpaired-crash]] |
| wes_il_pair | somatic, run **via the germline harness** (trial prefix `wes_il_pair_sweep::`), NOT a serial somatic binary | 58/58 |

## Two Entry Points, and When to Use Each

- **Canonical `scripts/e2e_sweep_gate.sh` → `python -m scripts.e2e_sweep_gate`**: stages
  fixtures, validates cache against `*.chunks.json` monolithic_md5, runs scoped
  `parity_e2e_sweep`, and writes the schema-v2 `parity-failure-report.json` with the
  `diagnosis_artifact` metadata **this skill consumes**. Use when you need the
  diagnosis handoff / the governing parity claim. It does `env = dict(os.environ)` and
  sets `CI=true` itself. Key flags: `--tag`, `--preset`, `--chrom`, `--report-dir`,
  `--fixture-source`, `--allow-extra-beds`, `--dry-run`, `--unstage`.
- **Lightweight `scripts/full_gate_tag.sh <tag>`**: loops all presets under
  `testdata/fixtures/e2e_sweep2/<tag>/output`, resume-skips GREEN from `summary.tsv`,
  one `cargo test` per preset, appends `cfg<TAB>STATUS<TAB>Ns<TAB>fails` and a final
  `FULL_GATE_DONE`. Use to drive a tag to green with resume over long runs.
- Both invoke the same `parity_e2e_sweep` cargo target and read the same env knobs;
  `SEL=${TAG}_sweep::parity_e2e_sweep_${TAG}` works uniformly for all 4 tags.

## Env Matrix (copy-pasteable)

```bash
source /home/eck/software/miniconda3/etc/profile.d/conda.sh
conda activate vdr
export LIBCLANG_PATH=$CONDA_PREFIX/lib

export VARDICT_E2E_SWEEP_BED_ROOT=/home/eck/workspace/vardict_rs2/tmp/sweep_beds
export VARDICT_E2E_SWEEP_FIXTURE_ROOT=$(pwd)/testdata/fixtures/e2e_sweep2/<tag>
export CI=true                       # silent-skip -> hard-fail
# --test-threads=12
export VARDICT_E2E_SWEEP_SPOOL_DIR=/hdd-disk1/eck/vardict_rs2/tmp_spool_gate  # big disk, not /home
export VARDICT_E2E_SWEEP_SORT_BUFFER_SIZE=1G
export VARDICT_E2E_SWEEP_STREAMING_SORT=true
export VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1
export VARDICT_E2E_SWEEP_MAX_FAILURES=100000
```

Resource locations (canonical BED cache, per-tag fixture homes, duplicate-cache traps)
→ [[sweep-resource-locations]].

## CM-PILEUP Streaming+zstd Lever

**What:** streams Rust rows into GNU `sort` over a pipe and streams the presorted Java
`.tsv.zst`; sort spill temps are zstd-compressed.

**Why:** CM-PILEUP is disk-bound TEST-HARNESS I/O (~42 GB decompressed per chunk), not a
vdr problem — enabling this lever took the sweep from **9h+ → ~75 min** in this session
(261/261 chunks, 4531s), byte-identical.

**Safety:** `use_disk_backed_diff(config) = config.do_pileup` at
`tests/parity_e2e_sweep/common.rs:1365` ⇒ the streaming/disk path is reached **ONLY by
CM-PILEUP**; every other config uses the in-memory diff and is untouched, so
`STREAMING_SORT=true` as a global env flag is safe. Streaming requires a presorted Java
fixture (only CM-PILEUP fixtures are presorted). Because streaming is a pure env knob
and the canonical wrapper inherits ambient env, it works through BOTH gates with no
wrapper code change.

## Real-Green Verification Checklist (per config)

- `test result: ok. N passed; 0 failed; 0 ignored`
- ZERO `status=skipped` lines
- ALL `mismatches=0` (grep for lines NOT matching `mismatches=0` must be empty)
- `phase=diff-complete` count == chunk count, and for CM-PILEUP each shows
  `mode=stream-sort`
- durations minutes-scale (**a ≤5 s "green" is a resume/silent skip, not a real run**)
- all chroms present
- final tally: unique GREEN configs == expected count, RED=0, `FULL_GATE_DONE` present

## False-Green Traps

- Stale-summary resume re-reports pre-fix greens ⇒ archive `summary.tsv` before
  re-gating after a *verdict-changing* fix (NOT needed for verdict-neutral knobs like
  streaming). [[gate-summary-resume-stale-falsegreen]]
- Without `CI=true` a missing/mismatched cache SKIPs and masquerades as GREEN.
  [[somatic-harness-thread-local-scope]]
- A background "completed" notification fires early — confirm with `ps` + artifacts.
  [[bg-task-completion-spurious]]
- CM-UNIQUN is a justified gap, not a fail. [[cm-uniqun-unpaired-crash]]
- wes_il_pair somatic runs via the germline harness. [[trust-user-repeated-claim]]

## Monitor Pattern

```bash
tail -F summary.tsv | grep -E $'\tGREEN\t|\tRED\t|FULL_GATE_DONE'
```
