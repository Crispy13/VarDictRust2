#!/usr/bin/env bash
# Generic full-scope germline/somatic parity gate for one tag.
# Usage: full_gate_tag.sh <tag>   e.g. full_gate_tag.sh hg005_exome
# Runs vdr for every preset under testdata/fixtures/e2e_sweep2/<tag>/output against the
# fresh goldens in the ready folder, one preset per cargo invocation, WITHOUT fail-fast
# (enumerate all reds in one pass). Resumes: skips presets already GREEN in summary.tsv.
# Green = vdr == fresh golden.
#
# VARDICT_E2E_SWEEP_BED_ROOT, VARDICT_E2E_SWEEP_SPOOL_DIR, and VARDICT_E2E_SWEEP_GATE_OUT
# default to this environment's paths below and are env-overridable (set them before
# invoking this script to point at a different machine's BED cache, scratch disk, or the
# summary.tsv+logs output dir). GATE_OUT is where resume state (summary.tsv) is read/written.
set -uo pipefail
TAG="${1:?usage: full_gate_tag.sh <tag>}"

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

source /home/eck/software/miniconda3/etc/profile.d/conda.sh; conda activate vdr
export LIBCLANG_PATH=/home/eck/software/miniconda3/envs/vdr/lib
export VARDICT_E2E_SWEEP_BED_ROOT="${VARDICT_E2E_SWEEP_BED_ROOT:-/home/eck/workspace/vardict_rs2/tmp/sweep_beds}"
export VARDICT_E2E_SWEEP_FIXTURE_ROOT=$(pwd)/testdata/fixtures/e2e_sweep2/$TAG
export VARDICT_E2E_SWEEP_MAX_FAILURES=100000
export VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1
# Relocate the ~22GB/chunk pileup spool off /home (73G) onto a bigger disk.
export VARDICT_E2E_SWEEP_SPOOL_DIR="${VARDICT_E2E_SWEEP_SPOOL_DIR:-/hdd-disk1/eck/vardict_rs2/tmp_spool_gate}"
mkdir -p "$VARDICT_E2E_SWEEP_SPOOL_DIR"
# Bigger GNU-sort buffer: pileup rust output is ~2.3GB/chunk; 128M default = ~18 disk-spilling
# merge passes (D-state thrash). 1G = ~3 passes, near-RAM. 2 concurrent (pileup) x1G = 2G RAM.
export VARDICT_E2E_SWEEP_SORT_BUFFER_SIZE=1G
# Streaming+zstd comparator for CM-PILEUP: streams Rust rows into GNU sort over a pipe and streams
# the presorted Java .tsv.zst, eliminating full-size Java/Rust spool files (32x less disk I/O,
# ~2x wall). Reached ONLY by CM-PILEUP (use_disk_backed_diff = do_pileup, common.rs:1365); all
# non-pileup configs use the in-memory diff and are unaffected. Validated byte-identical (Tier A
# exome chr8; Tier B na12878 chr22_chunk0) — verdict-neutral.
export VARDICT_E2E_SWEEP_STREAMING_SORT=true

SEL="${TAG}_sweep::parity_e2e_sweep_${TAG}"
OUT="${VARDICT_E2E_SWEEP_GATE_OUT:-/hdd-disk1/eck/vardict_rs2/regen_hg002_20260630/full_gate_${TAG}_20260703}"
mkdir -p "$OUT/logs"
PRESETS=$(ls "$VARDICT_E2E_SWEEP_FIXTURE_ROOT/output")
n=$(wc -w <<<"$PRESETS"); i=0; green=0; red=0
touch "$OUT/summary.tsv"
DONE_GREEN=$(awk -F'\t' '$2=="GREEN"{print $1}' "$OUT/summary.tsv" | sort -u)
for cfg in $PRESETS; do
  i=$((i+1))
  if grep -qxF "$cfg" <<<"$DONE_GREEN"; then
    echo "[$i/$n] $cfg SKIP (already GREEN)"; green=$((green+1)); continue
  fi
  t0=$(date +%s)
  # Uniform 12 threads: the ThreadBudget auto-limits pileup (per-chunk cost=2 => ~6 concurrent);
  # thrash is controlled by SORT_BUFFER_SIZE=1G above, not by thread count.
  # CI=true turns a missing/mismatched-cache SKIP into a hard failure, so a skipped
  # config can never masquerade as GREEN (silent-skip false-green bug, wes_il_pair 2026-07-04).
  CI=true VARDICT_E2E_SWEEP_CONFIG="$cfg" \
    cargo test --profile debug-release --test parity_e2e_sweep -- \
    --include-ignored --exact "$SEL" --test-threads=12 \
    > "$OUT/logs/gate_${cfg}.log" 2>&1
  rc=$?
  dt=$(( $(date +%s) - t0 ))
  if [[ $rc -eq 0 ]]; then st=GREEN; green=$((green+1)); else st=RED; red=$((red+1)); fi
  fails=$(grep -oE 'failures?: [0-9]+' "$OUT/logs/gate_${cfg}.log" | tail -1)
  printf '%s\t%s\t%ds\t%s\n' "$cfg" "$st" "$dt" "$fails" | tee -a "$OUT/summary.tsv"
  echo "[$i/$n] $cfg $st ${dt}s $fails"
done
echo "FULL_GATE_DONE tag=$TAG green=$green red=$red total=$n" | tee -a "$OUT/summary.tsv"
