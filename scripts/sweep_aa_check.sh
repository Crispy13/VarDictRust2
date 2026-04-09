#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/sweep_aa_check.sh

Sample 100 deterministic regions from tmp/sweep_beds, run VarDictJava twice per
region with parity outputs enabled, and diff the two runs to check determinism.
EOF
}

if [[ $# -gt 0 ]]; then
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 1
            ;;
    esac
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TMP_ROOT="$PROJECT_ROOT/tmp"
SWEEP_BED_ROOT="$TMP_ROOT/sweep_beds"
OUTPUT_ROOT="$TMP_ROOT/sweep_aa"
VARDICT_DIR="$PROJECT_ROOT/VarDictJava"
VARDICT_BIN="$VARDICT_DIR/build/install/VarDict/bin/VarDict"
REF="$PROJECT_ROOT/testdata/hs37d5.fa"
SAMPLE_SIZE=100
SHUF_SEED=42

MODULE_DIRS=(cigar_parser realigner sv_processor tovars)
MODULE_ENVS=(
    VARDICT_PARITY_CIGAR_PARSER
    VARDICT_PARITY_REALIGNER
    VARDICT_PARITY_SV_PROCESSOR
    VARDICT_PARITY_TOVARS
)

declare -A BAM_MAP=(
    [na12878_exome]="testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam"
    [hg002]="testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam"
    [na12878_lowcov]="testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam"
)

TEMP_DIR=""
COMBINED_FILE=""
SAMPLED_FILE=""
RANDOM_SOURCE=""

total_regions=0
passed_regions=0
failed_regions=0
skipped_regions=0
run_failures=0

cleanup() {
    if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
        rm -rf "$TEMP_DIR"
    fi
}

trap cleanup EXIT

require_command() {
    local cmd="$1"
    if ! command -v "$cmd" > /dev/null 2>&1; then
        echo "ERROR: required command not found: $cmd" >&2
        exit 1
    fi
}

resolve_project_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s\n' "$PROJECT_ROOT/$path"
    fi
}

prepare_module_dirs() {
    local root="$1"
    local module_dir
    mkdir -p "$root"
    for module_dir in "${MODULE_DIRS[@]}"; do
        mkdir -p "$root/$module_dir"
    done
}

count_jsonl_files() {
    local dir="$1"
    find "$dir" -type f -name '*.jsonl' | wc -l | tr -d '[:space:]'
}

ensure_vardict_bin() {
    if [[ -x "$VARDICT_BIN" ]]; then
        return 0
    fi

    echo "--- Building VarDictJava ---"
    (cd "$VARDICT_DIR" && ./gradlew installDist -q)

    if [[ ! -x "$VARDICT_BIN" ]]; then
        echo "ERROR: VarDictJava binary not found at $VARDICT_BIN" >&2
        exit 1
    fi
}

init_temp_files() {
    mkdir -p "$TMP_ROOT"
    TEMP_DIR="$(mktemp -d "$TMP_ROOT/sweep_aa.XXXXXX")"
    COMBINED_FILE="$TEMP_DIR/all_regions.tsv"
    SAMPLED_FILE="$TEMP_DIR/sampled_regions.tsv"
    RANDOM_SOURCE="$TEMP_DIR/shuf_random_source.bin"
    : > "$COMBINED_FILE"
}

build_sampling_input() {
    local bed_file
    local bam_tag

    while IFS= read -r bed_file; do
        bam_tag="$(basename "$(dirname "$bed_file")")"
        awk -v bam_tag="$bam_tag" 'BEGIN { FS = OFS = "\t" } NF >= 3 { print bam_tag, $1, $2, $3 }' "$bed_file" >> "$COMBINED_FILE"
    done < <(find "$SWEEP_BED_ROOT" -mindepth 2 -maxdepth 2 -type f -name '*.bed' | LC_ALL=C sort)

    if [[ ! -s "$COMBINED_FILE" ]]; then
        echo "ERROR: no sweep BED regions found under $SWEEP_BED_ROOT" >&2
        exit 1
    fi
}

make_random_source() {
    # shuf does not have a seed flag, so provide deterministic random bytes for seed 42.
    python3 - "$SHUF_SEED" "$RANDOM_SOURCE" <<'PY'
import random
import sys

seed = int(sys.argv[1])
path = sys.argv[2]
rng = random.Random(seed)

with open(path, "wb") as handle:
    handle.write(bytes(rng.randrange(256) for _ in range(8 * 1024 * 1024)))
PY
}

sample_regions() {
    make_random_source
    shuf --random-source="$RANDOM_SOURCE" -n "$SAMPLE_SIZE" "$COMBINED_FILE" > "$SAMPLED_FILE"

    if [[ ! -s "$SAMPLED_FILE" ]]; then
        echo "ERROR: sampling produced no regions" >&2
        exit 1
    fi

    total_regions="$(grep -cve '^[[:space:]]*$' "$SAMPLED_FILE")"
}

run_vardict() {
    local out_root="$1"
    local region="$2"
    local bam="$3"
    local log_file="$4"

    local -a env_args=()
    local idx
    for idx in "${!MODULE_DIRS[@]}"; do
        env_args+=("${MODULE_ENVS[$idx]}=$out_root/${MODULE_DIRS[$idx]}")
    done

    if env "${env_args[@]}" \
        "$VARDICT_BIN" \
        -G "$REF" \
        -b "$bam" \
        -th 1 \
        -R "$region" \
        > /dev/null 2> "$log_file"; then
        return 0
    fi

    return 1
}

main() {
    local bam_tag
    local chrom
    local start
    local end
    local index=0
    local tag
    local bam_rel
    local bam
    local region
    local run_a_root
    local run_b_root
    local log_a
    local log_b
    local diff_file
    local run_a_ok
    local run_b_ok
    local files_a
    local files_b

    require_command awk
    require_command find
    require_command python3
    require_command shuf

    if [[ ! -d "$SWEEP_BED_ROOT" ]]; then
        echo "ERROR: sweep BED root not found: $SWEEP_BED_ROOT" >&2
        exit 1
    fi

    if [[ ! -f "$REF" ]]; then
        echo "ERROR: reference FASTA not found: $REF" >&2
        exit 1
    fi

    ensure_vardict_bin
    init_temp_files
    build_sampling_input
    sample_regions

    rm -rf "$OUTPUT_ROOT"
    mkdir -p "$OUTPUT_ROOT/run_a" "$OUTPUT_ROOT/run_b" "$OUTPUT_ROOT/logs" "$OUTPUT_ROOT/diffs"

    echo "=== sweep_aa_check ==="
    echo "Sweep BED root: $SWEEP_BED_ROOT"
    echo "Reference:      $REF"
    echo "Sample size:    $total_regions"
    echo "Shuf seed:      $SHUF_SEED"
    echo "Output root:    $OUTPUT_ROOT"
    echo ""

    while IFS=$'\t' read -r bam_tag chrom start end _rest; do
        [[ -z "${bam_tag:-}" ]] && continue

        index=$((index + 1))
        tag="$(printf '%03d' "$index")"
        bam_rel="${BAM_MAP[$bam_tag]:-}"

        if [[ -z "$bam_rel" ]]; then
            skipped_regions=$((skipped_regions + 1))
            echo "[$tag/$total_regions] SKIP unknown bam tag: $bam_tag"
            continue
        fi

        bam="$(resolve_project_path "$bam_rel")"
        if [[ ! -f "$bam" ]]; then
            skipped_regions=$((skipped_regions + 1))
            echo "[$tag/$total_regions] SKIP missing BAM: $bam"
            continue
        fi

        if ! [[ "$start" =~ ^[0-9]+$ && "$end" =~ ^[0-9]+$ ]] || (( end <= start )); then
            skipped_regions=$((skipped_regions + 1))
            echo "[$tag/$total_regions] SKIP invalid BED coordinates: $chrom $start $end"
            continue
        fi

        region="${chrom}:$((start + 1))-$end"
        run_a_root="$OUTPUT_ROOT/run_a/$tag"
        run_b_root="$OUTPUT_ROOT/run_b/$tag"
        log_a="$OUTPUT_ROOT/logs/${tag}_run_a.log"
        log_b="$OUTPUT_ROOT/logs/${tag}_run_b.log"
        diff_file="$OUTPUT_ROOT/diffs/${tag}.diff"

        prepare_module_dirs "$run_a_root"
        prepare_module_dirs "$run_b_root"

        echo "[$tag/$total_regions] RUN bam_tag=$bam_tag region=$region"

        run_a_ok=1
        run_b_ok=1
        if ! run_vardict "$run_a_root" "$region" "$bam" "$log_a"; then
            run_a_ok=0
            run_failures=$((run_failures + 1))
        fi

        if ! run_vardict "$run_b_root" "$region" "$bam" "$log_b"; then
            run_b_ok=0
            run_failures=$((run_failures + 1))
        fi

        if [[ "$run_a_ok" -eq 0 || "$run_b_ok" -eq 0 ]]; then
            failed_regions=$((failed_regions + 1))
            echo "[$tag/$total_regions] FAIL run error (logs: $log_a, $log_b)" >&2
            continue
        fi

        files_a="$(count_jsonl_files "$run_a_root")"
        files_b="$(count_jsonl_files "$run_b_root")"
        if [[ "$files_a" -eq 0 && "$files_b" -eq 0 ]]; then
            skipped_regions=$((skipped_regions + 1))
            echo "[$tag/$total_regions] SKIP empty output"
            continue
        fi

        if diff -rq "$run_a_root" "$run_b_root" > "$diff_file" 2>&1; then
            rm -f "$diff_file"
            passed_regions=$((passed_regions + 1))
            echo "[$tag/$total_regions] PASS"
        else
            failed_regions=$((failed_regions + 1))
            echo "[$tag/$total_regions] FAIL diff mismatch (see $diff_file)" >&2
        fi
    done < "$SAMPLED_FILE"

    echo ""
    echo "=== Summary ==="
    echo "Total sampled: $total_regions"
    echo "Passed:        $passed_regions"
    echo "Failed:        $failed_regions"
    echo "Skipped:       $skipped_regions"
    echo "Run failures:  $run_failures"

    if [[ "$failed_regions" -gt 0 ]]; then
        exit 1
    fi
}

main "$@"