#!/usr/bin/env bash
# make_cov_bed.sh — build a coverage-derived, representative-budget-capped BED from one or two BAMs.
#
# Method (the project's proven approach, see vardict_rs2/scripts/gen_sweep_bed.sh):
#   bedtools genomecov -bg  ->  merge  ->  makewindows -w 700
# then stratify the 700bp windows by mean coverage and systematically sample down to a size
# budget so a downstream -th 8 run finishes in minutes while still reflecting the real
# coverage distribution. With two BAMs (somatic tumor|normal) the covered region is the
# intersection (positions covered in BOTH), so the BED reflects work both pipelines do.
#
# Requires: samtools + bedtools (present in the `vdr` conda env).
#
# Usage:
#   make_cov_bed.sh --out FILE [--budget-mb N] [--window N] [--region CHR] BAM [BAM2]
#
set -euo pipefail

OUT=""
BUDGET_MB=20
WINDOW=700
REGION=""          # optional samtools region (e.g. "1") to keep genomecov tractable on WGS

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)        OUT="$2"; shift 2 ;;
    --budget-mb)  BUDGET_MB="$2"; shift 2 ;;
    --window)     WINDOW="$2"; shift 2 ;;
    --region)     REGION="$2"; shift 2 ;;
    -h|--help)    grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*)           echo "ERROR: unknown option: $1" >&2; exit 2 ;;
    *)            break ;;
  esac
done

[[ -n "$OUT" ]] || { echo "ERROR: --out is required" >&2; exit 2; }
[[ $# -ge 1 ]]  || { echo "ERROR: at least one BAM is required" >&2; exit 2; }
BAM1="$1"; BAM2="${2:-}"

for b in "$BAM1" ${BAM2:+"$BAM2"}; do
  [[ -f "$b" ]] || { echo "ERROR: BAM not found: $b" >&2; exit 1; }
done
command -v samtools >/dev/null || { echo "ERROR: samtools not in PATH (activate vdr env)" >&2; exit 1; }
command -v bedtools >/dev/null || { echo "ERROR: bedtools not in PATH (activate vdr env)" >&2; exit 1; }

if [[ -s "$OUT" ]]; then
  echo "make_cov_bed: cached $OUT ($(wc -l < "$OUT") windows) — reuse"; exit 0
fi

TMP="$(mktemp -d "${TMPDIR:-/tmp}/make_cov_bed.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$(dirname "$OUT")"

# per-BAM coverage bedgraph (region-limited via samtools for speed on large BAMs)
gencov() {  # $1=bam  $2=out_bg  (lexicographically sorted: bedtools merge/map need it; genomecov
            #                      emits BAM-header order, which breaks on many-contig references)
  if [[ -n "$REGION" ]]; then
    samtools view -b "$1" "$REGION" | bedtools genomecov -ibam - -bg | sort -k1,1 -k2,2n > "$2"
  else
    bedtools genomecov -ibam "$1" -bg | sort -k1,1 -k2,2n > "$2"
  fi
}

echo "make_cov_bed: genomecov $BAM1 ${REGION:+(region $REGION)}"
gencov "$BAM1" "$TMP/bg1"
bedtools merge -i "$TMP/bg1" > "$TMP/m1"

if [[ -n "$BAM2" ]]; then
  echo "make_cov_bed: genomecov $BAM2 (intersect)"
  gencov "$BAM2" "$TMP/bg2"
  bedtools merge -i "$TMP/bg2" > "$TMP/m2"
  bedtools intersect -a "$TMP/m1" -b "$TMP/m2" | sort -k1,1 -k2,2n > "$TMP/covered"
else
  cp "$TMP/m1" "$TMP/covered"
fi

[[ -s "$TMP/covered" ]] || { echo "ERROR: no covered intervals" >&2; exit 1; }

# 700bp windows over covered intervals, annotated with mean coverage (col4) from bam1's bedgraph.
# Both map inputs must be lexicographically sorted (sweep-based operation).
bedtools makewindows -b "$TMP/covered" -w "$WINDOW" | sort -k1,1 -k2,2n > "$TMP/windows"
bedtools map -a "$TMP/windows" -b "$TMP/bg1" -c 4 -o mean -null 0 2>/dev/null > "$TMP/wcov"

target=$(( BUDGET_MB * 1000000 / WINDOW ))
total=$(wc -l < "$TMP/wcov")
echo "make_cov_bed: $total covered windows; budget=${BUDGET_MB}Mb -> target $target windows"

if (( total <= target )); then
  cut -f1-3 "$TMP/wcov" | sort -k1,1 -k2,2n > "$OUT"
else
  # stratified systematic sample: sort by coverage, take every k-th window so the
  # selection spans the whole coverage distribution; then restore genomic order.
  sort -t$'\t' -k4,4g "$TMP/wcov" \
    | awk -v k="$(awk -v t="$total" -v g="$target" 'BEGIN{print (t+g-1)/g}')" \
        'NR % int(k+0.5) == 1 {print $1"\t"$2"\t"$3}' \
    | sort -k1,1 -k2,2n > "$OUT"
fi

echo "make_cov_bed: wrote $OUT ($(wc -l < "$OUT") windows)"
