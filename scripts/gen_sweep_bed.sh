#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/gen_sweep_bed.sh [--min-interval N] <bam> <bam_tag>

Generate non-overlapping 700bp BED tiles from covered regions in a BAM.
Outputs per-chromosome BED files under tmp/sweep_beds/<bam_tag>/.

Options:
  --min-interval N  Drop merged intervals shorter than N bp before tiling.
                    Default: 0 (keep all). Recommended: 100 (drops off-target noise).
EOF
}

MIN_INTERVAL=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --min-interval)
            MIN_INTERVAL="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            echo "ERROR: unknown option: $1" >&2
            usage >&2
            exit 1
            ;;
        *)
            break
            ;;
    esac
done

if [[ $# -ne 2 ]]; then
    usage >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

resolve_project_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s\n' "$PROJECT_ROOT/$path"
    fi
}

BAM_INPUT="$1"
BAM_TAG="$2"
BAM_PATH="$(resolve_project_path "$BAM_INPUT")"
OUTPUT_DIR="$PROJECT_ROOT/tmp/sweep_beds/$BAM_TAG"
TEMP_ROOT="$PROJECT_ROOT/tmp"
TEMP_DIR="$(mktemp -d "$TEMP_ROOT/gen_sweep_bed.${BAM_TAG}.XXXXXX")"
FILTERED_BED="$TEMP_DIR/covered.filtered.bed"
MERGED_BED="$TEMP_DIR/covered.merged.bed"
WINDOWS_BED="$TEMP_DIR/windows.bed"
TARGET_CHROMS=(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 X Y MT)

cleanup() {
    rm -rf "$TEMP_DIR"
}

trap cleanup EXIT

if ! command -v bedtools >/dev/null 2>&1; then
    echo "ERROR: bedtools is not available in PATH" >&2
    exit 1
fi

if [[ ! -f "$BAM_PATH" ]]; then
    echo "ERROR: BAM not found: $BAM_PATH" >&2
    exit 1
fi

mkdir -p "$TEMP_ROOT"
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

bedtools genomecov -ibam "$BAM_PATH" -bg \
    | awk 'BEGIN {
            for (i = 1; i <= 22; i++) {
                keep[i] = 1
            }
            keep["X"] = 1
            keep["Y"] = 1
            keep["MT"] = 1
        }
        keep[$1] { print $1, $2, $3 }' OFS='\t' \
    > "$FILTERED_BED"

if [[ ! -s "$FILTERED_BED" ]]; then
    echo "No covered intervals found for target chromosomes." >&2
    echo "Total tiles: 0"
    exit 0
fi

bedtools merge -i "$FILTERED_BED" > "$MERGED_BED"

# Filter out small intervals (off-target noise)
if [[ "$MIN_INTERVAL" -gt 0 ]]; then
    awk -v min="$MIN_INTERVAL" '($3 - $2) >= min' "$MERGED_BED" > "$MERGED_BED.tmp"
    mv "$MERGED_BED.tmp" "$MERGED_BED"
fi

bedtools makewindows -b "$MERGED_BED" -w 700 > "$WINDOWS_BED"

awk -v outdir="$OUTPUT_DIR" '{ print > (outdir "/" $1 ".bed") }' "$WINDOWS_BED"

total_tiles=0
for chrom in "${TARGET_CHROMS[@]}"; do
    chrom_file="$OUTPUT_DIR/$chrom.bed"
    if [[ ! -f "$chrom_file" ]]; then
        continue
    fi

    tile_count="$(wc -l < "$chrom_file" | tr -d '[:space:]')"
    echo "$chrom: $tile_count"
    total_tiles=$((total_tiles + tile_count))
done

echo "Total: $total_tiles"