#!/usr/bin/env bash
# run_matrix.sh — production benchmark of VarDictJava (vdj) vs VarDict-rs (vdr).
#
# For each (workload x preset): build identical flags, run BOTH tools at -th 8 under
# /usr/bin/time -v for N+1 runs (1 warmup discarded), capture the raw TSV stdout and the
# time logs. vdr presets whose flags vdr's CLI does not support (e.g. CM-NOSV `-U`) are
# marked UNSUPPORTED and the vdr run is skipped. Then report.py renders the report.
#
# Workloads (fixed): germline WGS (NA12878 low-coverage) + somatic WES pair (WES_IL T|N).
# Presets (fixed):   T1-01, T1-02, T1-06, CM-NOSV.
#
# Usage:
#   run_matrix.sh [--runs N] [--budget-mb N] [--smoke] [--workloads "germline somatic"]
#                 [--presets "T1-01 T1-06"]
#   --smoke : tiny single cell (somatic x T1-01, runs=1, budget 1Mb) for quick validation.
#
set -euo pipefail

WS=/home/eck/workspace/vardict_rs_claude
CL="$WS/opt/claude"
TD="$CL/testdata"
VDR="$CL/target/debug-release/vardict_rs"
VDJ="$WS/VarDictJava/build/install/VarDict/bin/VarDict"
OUT="$CL/tmp/production-bench"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

RUNS=3
BUDGET_MB=20
SMOKE=0
WORKLOADS="germline somatic"
PRESETS="T1-01 T1-02 T1-06 CM-NOSV"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs)       RUNS="$2"; shift 2 ;;
    --budget-mb)  BUDGET_MB="$2"; shift 2 ;;
    --smoke)      SMOKE=1; shift ;;
    --workloads)  WORKLOADS="$2"; shift 2 ;;
    --presets)    PRESETS="$2"; shift 2 ;;
    -h|--help)    grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)            echo "ERROR: unknown option: $1" >&2; exit 2 ;;
  esac
done

SMOKE_REGION=""
if [[ "$SMOKE" == 1 ]]; then
  # cheap: germline over a tiny region so genomecov + run finish in ~1 min
  RUNS=1; BUDGET_MB=1; WORKLOADS="germline"; PRESETS="T1-01"
  SMOKE_REGION="20:29900000-30100000"
  echo "== SMOKE: germline x T1-01 over $SMOKE_REGION, runs=1, budget=1Mb =="
fi

# --- environment: vdr conda env provides samtools/bedtools (and matches the vdr build) ---
source /home/eck/software/miniconda3/etc/profile.d/conda.sh
conda activate vdr

[[ -x "$VDR" ]] || { echo "ERROR: vdr binary missing: $VDR  (build it first)" >&2; exit 1; }
[[ -x "$VDJ" ]] || { echo "ERROR: vdj launcher missing: $VDJ" >&2; exit 1; }

mkdir -p "$OUT/beds" "$OUT/out" "$OUT/time"

# --- preset flag table (from vardict_rs2/scripts/config_presets.tsv) ---
preset_flags() {
  case "$1" in
    T1-01)   echo "" ;;
    T1-02)   echo "-f 0.005 -r 1 -q 15" ;;
    T1-06)   echo "-f 0.001 -r 1 -q 20 -m 12" ;;
    CM-NOSV) echo "-U" ;;
    *)       echo "__UNKNOWN__" ;;
  esac
}

# --- does the vdr CLI accept every flag in this preset? ---
VDR_HELP="$($VDR --help 2>&1 || true)"
vdr_supports() {
  # clap prints flags as "-U, --nosv" or "-f <FREQ>" or "-z [<...>]" — allow space/comma/[/< or EOL
  local tok
  for tok in $1; do
    [[ "$tok" == -* ]] || continue
    grep -qE -- "(^|[[:space:]])${tok}([[:space:],<[]|$)" <<<"$VDR_HELP" || return 1
  done
  return 0
}

# --- workload resolver: sets REF BAM COVBAMS NAME REGION ---
resolve_workload() {
  case "$1" in
    germline)
      REF="$TD/hs37d5.fa"
      BAM="$TD/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam"
      COVBAMS=("$BAM")
      NAME="NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211"
      REGION="${SMOKE_REGION:-1}" ;;   # limit WGS genomecov+run to chr1 (or smoke sub-region)
    somatic)
      REF="$TD/GRCh38.d1.vd1.fa"
      local T="$TD/WES_IL_T_1.bwa.dedup.bam" N="$TD/WES_IL_N_1.bwa.dedup.bam"
      BAM="$T|$N"
      COVBAMS=("$T" "$N")
      NAME="WES_IL"
      REGION="" ;;    # WES is targeted/small — no region limit needed
    *) echo "ERROR: unknown workload: $1" >&2; exit 2 ;;
  esac
}

# columns for 700bp window BEDs (matches the project's sweep invocation)
BEDCOLS="-c 1 -S 2 -E 3 -s 2 -e 3"

median_of() { sort -n | awk '{a[NR]=$1} END{print (NR%2)?a[(NR+1)/2]:(a[NR/2]+a[NR/2+1])/2}'; }

run_tool() {  # $1=label(vdj|vdr) $2=cell  $3...=cmd
  local label="$1" cell="$2"; shift 2
  local i tlog tsv
  for ((i=0; i<=RUNS; i++)); do
    tsv="$OUT/out/${cell}.${label}.tsv"
    tlog="$OUT/time/${cell}.${label}.run${i}.time"
    /usr/bin/time -v -o "$tlog" "$@" > "$tsv" 2> "$OUT/time/${cell}.${label}.run${i}.stderr" || {
      echo "    $label run$i FAILED (see $OUT/time/${cell}.${label}.run${i}.stderr)"; return 1; }
    if [[ $i -eq 0 ]]; then rm -f "$tlog"; fi   # discard warmup timing (keep its tsv for parity)
  done
  return 0
}

echo "== production-bench =="; uptime | tee "$OUT/uptime.txt"
echo "matrix: workloads=[$WORKLOADS] presets=[$PRESETS] runs=$RUNS budget=${BUDGET_MB}Mb"
echo "(estimate: ${RUNS}+1 runs x 2 tools per supported cell at -th 8 — expect minutes per cell)"

for wl in $WORKLOADS; do
  resolve_workload "$wl"
  bed="$OUT/beds/${wl}.b${BUDGET_MB}.bed"
  bash "$SCRIPT_DIR/make_cov_bed.sh" --out "$bed" --budget-mb "$BUDGET_MB" \
       ${REGION:+--region "$REGION"} "${COVBAMS[@]}"

  for ps in $PRESETS; do
    flags="$(preset_flags "$ps")"
    [[ "$flags" == "__UNKNOWN__" ]] && { echo "  skip unknown preset $ps"; continue; }
    cell="${wl}.${ps}"
    echo "  -- cell $cell  flags=[$flags] --"

    # vdj (always runnable — Java VarDict supports all flags). `-z 1` = zero-based BED,
    # matching bedtools output and vdr's fixed zero-based convention (vdr has no -z flag).
    run_tool vdj "$cell" "$VDJ" -G "$REF" -b "$BAM" -N "$NAME" -th 8 -z 1 $flags $BEDCOLS "$bed" \
      && echo "    vdj ok" || echo "    vdj error"

    # vdr (full VDJ CLI parity as of the CLI-completion change; -z 1 to match vdj/bedtools 0-based)
    if vdr_supports "$flags"; then
      rm -f "$OUT/out/${cell}.vdr.UNSUPPORTED"   # clear any stale marker from a prior run
      run_tool vdr "$cell" "$VDR" -G "$REF" -b "$BAM" -N "$NAME" -th 8 -z 1 $flags $BEDCOLS "$bed" \
        && echo "    vdr ok" || echo "    vdr error"
    else
      echo "    vdr UNSUPPORTED (CLI lacks a flag in [$flags]) — skipped"
      : > "$OUT/out/${cell}.vdr.UNSUPPORTED"
    fi
  done
done

echo "== rendering report =="
python3 "$SCRIPT_DIR/report.py" --root "$OUT" --runs "$RUNS" \
        --workloads "$WORKLOADS" --presets "$PRESETS"
echo "== done: $OUT/REPORT.md =="
