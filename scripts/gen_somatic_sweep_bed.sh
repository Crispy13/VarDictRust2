#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/gen_somatic_sweep_bed.sh [--min-interval N] <tumor_bam> <normal_bam> <output_tag>

Generate non-overlapping 700bp BED tiles from the union of covered regions in a
tumor/normal BAM pair. Outputs per-chromosome BED files under
tmp/sweep_beds/<output_tag>/.

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

if [[ $# -ne 3 ]]; then
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

TUMOR_INPUT="$1"
NORMAL_INPUT="$2"
OUTPUT_TAG="$3"
TUMOR_BAM_PATH="$(resolve_project_path "$TUMOR_INPUT")"
NORMAL_BAM_PATH="$(resolve_project_path "$NORMAL_INPUT")"
OUTPUT_DIR="$PROJECT_ROOT/tmp/sweep_beds/$OUTPUT_TAG"
TEMP_ROOT="$PROJECT_ROOT/tmp"
TEMP_DIR="$(mktemp -d "$TEMP_ROOT/gen_somatic_sweep_bed.${OUTPUT_TAG}.XXXXXX")"
TUMOR_FILTERED_BED="$TEMP_DIR/tumor.covered.filtered.bed"
NORMAL_FILTERED_BED="$TEMP_DIR/normal.covered.filtered.bed"
COMBINED_SORTED_BED="$TEMP_DIR/covered.sorted.bed"
MERGED_BED="$TEMP_DIR/covered.merged.bed"
WINDOWS_BED="$TEMP_DIR/windows.bed"
TARGET_CHROMS=(chr1 chr2 chr3 chr4 chr5 chr6 chr7 chr8 chr9 chr10 chr11 chr12 chr13 chr14 chr15 chr16 chr17 chr18 chr19 chr20 chr21 chr22 chrX chrY)

cleanup() {
    rm -rf "$TEMP_DIR"
}

trap cleanup EXIT

if ! command -v bedtools >/dev/null 2>&1; then
    echo "ERROR: bedtools is not available in PATH" >&2
    exit 1
fi

if [[ ! -f "$TUMOR_BAM_PATH" ]]; then
    echo "ERROR: tumor BAM not found: $TUMOR_BAM_PATH" >&2
    exit 1
fi

if [[ ! -f "$NORMAL_BAM_PATH" ]]; then
    echo "ERROR: normal BAM not found: $NORMAL_BAM_PATH" >&2
    exit 1
fi

mkdir -p "$TEMP_ROOT"
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

bedtools genomecov -ibam "$TUMOR_BAM_PATH" -bg \
    | awk 'BEGIN {
            for (i = 1; i <= 22; i++) {
                keep["chr" i] = 1
            }
            keep["chrX"] = 1
            keep["chrY"] = 1
        }
        keep[$1] { print $1, $2, $3 }' OFS='\t' \
    > "$TUMOR_FILTERED_BED"

bedtools genomecov -ibam "$NORMAL_BAM_PATH" -bg \
    | awk 'BEGIN {
            for (i = 1; i <= 22; i++) {
                keep["chr" i] = 1
            }
            keep["chrX"] = 1
            keep["chrY"] = 1
        }
        keep[$1] { print $1, $2, $3 }' OFS='\t' \
    > "$NORMAL_FILTERED_BED"

cat "$TUMOR_FILTERED_BED" "$NORMAL_FILTERED_BED" \
    | LC_ALL=C sort -k1,1 -k2,2n \
    > "$COMBINED_SORTED_BED"

if [[ ! -s "$COMBINED_SORTED_BED" ]]; then
    echo "No covered intervals found for target chromosomes." >&2
    echo "Total tiles: 0"
    exit 0
fi

bedtools merge -i "$COMBINED_SORTED_BED" > "$MERGED_BED"

# Filter out small intervals (off-target noise)
if [[ "$MIN_INTERVAL" -gt 0 ]]; then
    awk -v min="$MIN_INTERVAL" '($3 - $2) >= min' "$MERGED_BED" > "$MERGED_BED.tmp"
    mv "$MERGED_BED.tmp" "$MERGED_BED"
fi

if [[ ! -s "$MERGED_BED" ]]; then
    echo "No covered intervals remain after filtering." >&2
    echo "Total tiles: 0"
    exit 0
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