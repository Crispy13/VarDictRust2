#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/sweep_fixtures.sh [--bam-tags tag1,tag2,...] [--dry-run]

Generate parity fixture JSONL outputs from pre-generated sweep BED files.

Options:
  --bam-tags TAGS  Comma-separated BAM tags to process.
                   Default: na12878_exome,hg002,na12878_lowcov
  --dry-run        Print planned operations without running VarDictJava.
  -h, --help       Show this help message.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TMP_ROOT="$PROJECT_ROOT/tmp"
SWEEP_BED_ROOT="$TMP_ROOT/sweep_beds"
OUTPUT_ROOT="$TMP_ROOT/sweep_fixtures"
VARDICT_DIR="$PROJECT_ROOT/VarDictJava"
VARDICT_BIN="$VARDICT_DIR/build/install/VarDict/bin/VarDict"
REF_REL="testdata/hs37d5.fa"
REF="$PROJECT_ROOT/$REF_REL"

MODULE_DIRS=(cigar_parser realigner sv_processor tovars)
MODULE_ENVS=(
    VARDICT_PARITY_CIGAR_PARSER
    VARDICT_PARITY_REALIGNER
    VARDICT_PARITY_SV_PROCESSOR
    VARDICT_PARITY_TOVARS
)
ALL_BAM_TAGS=(na12878_exome hg002 na12878_lowcov)

declare -A BAM_MAP=(
    [na12878_exome]="testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam"
    [hg002]="testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam"
    [na12878_lowcov]="testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam"
)

DRY_RUN=0
SELECTED_BAM_TAGS=()
TEMP_DIR=""
REGION_BAM_FILE=""
FIXTURE_REGION_FILE=""
WINDOW_SIZES_FILE=""
COLLECTED_BED_FILES=()

planned_tags=0
planned_chromosomes=0
processed_tags=0
processed_chromosomes=0
generated_fixtures=0
compressed_fixtures=0
missing_bed_tags=0

cleanup() {
    if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
        rm -rf "$TEMP_DIR"
    fi
}

trap cleanup EXIT

resolve_project_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s\n' "$PROJECT_ROOT/$path"
    fi
}

json_escape() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//$'\n'/\\n}"
    value="${value//$'\r'/\\r}"
    value="${value//$'\t'/\\t}"
    printf '%s' "$value"
}

join_by_comma() {
    local first=1
    local item
    for item in "$@"; do
        if [[ "$first" -eq 0 ]]; then
            printf ','
        fi
        printf '%s' "$item"
        first=0
    done
}

init_temp_files() {
    mkdir -p "$TMP_ROOT"
    TEMP_DIR="$(mktemp -d "$TMP_ROOT/sweep_fixtures.XXXXXX")"
    REGION_BAM_FILE="$TEMP_DIR/region_bams.tsv"
    FIXTURE_REGION_FILE="$TEMP_DIR/fixture_regions.tsv"
    WINDOW_SIZES_FILE="$TEMP_DIR/window_sizes.tsv"
    : > "$REGION_BAM_FILE"
    : > "$FIXTURE_REGION_FILE"
    : > "$WINDOW_SIZES_FILE"
}

validate_bam_tag() {
    local tag="$1"
    if [[ -z "${BAM_MAP[$tag]+x}" ]]; then
        echo "ERROR: unknown BAM tag: $tag" >&2
        exit 1
    fi
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --bam-tags)
                if [[ $# -lt 2 ]]; then
                    echo "ERROR: --bam-tags requires a value" >&2
                    usage >&2
                    exit 1
                fi
                IFS=',' read -r -a SELECTED_BAM_TAGS <<< "$2"
                shift 2
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                echo "ERROR: unknown option: $1" >&2
                usage >&2
                exit 1
                ;;
        esac
    done

    if [[ "${#SELECTED_BAM_TAGS[@]}" -eq 0 ]]; then
        SELECTED_BAM_TAGS=("${ALL_BAM_TAGS[@]}")
    fi

    local deduped=()
    local seen=()
    local tag
    for tag in "${SELECTED_BAM_TAGS[@]}"; do
        if [[ -z "$tag" ]]; then
            continue
        fi
        validate_bam_tag "$tag"
        if [[ " ${seen[*]} " != *" $tag "* ]]; then
            deduped+=("$tag")
            seen+=("$tag")
        fi
    done
    SELECTED_BAM_TAGS=("${deduped[@]}")

    if [[ "${#SELECTED_BAM_TAGS[@]}" -eq 0 ]]; then
        echo "ERROR: no BAM tags selected" >&2
        exit 1
    fi
}

check_dependencies() {
    if ! command -v zstd &>/dev/null; then
        echo "ERROR: zstd is required but not found in PATH." >&2
        echo "       Activate the conda environment first: conda activate rust_build_env" >&2
        exit 1
    fi
}

ensure_vardict_bin() {
    if [[ -x "$VARDICT_BIN" ]]; then
        if [[ "$DRY_RUN" -eq 1 ]]; then
            echo "[dry-run] would use existing VarDictJava binary: $VARDICT_BIN"
        fi
        return 0
    fi

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[dry-run] would build VarDictJava with ./gradlew installDist"
        return 0
    fi

    echo "--- Building VarDictJava ---"
    (cd "$VARDICT_DIR" && ./gradlew installDist -q)

    if [[ ! -x "$VARDICT_BIN" ]]; then
        echo "ERROR: VarDictJava binary not found at $VARDICT_BIN" >&2
        exit 1
    fi
}

collect_bed_files() {
    local tag="$1"
    local bed_dir="$SWEEP_BED_ROOT/$tag"
    COLLECTED_BED_FILES=()

    if [[ ! -d "$bed_dir" ]]; then
        if [[ "$DRY_RUN" -eq 1 ]]; then
            echo "[dry-run] missing sweep BED directory for $tag: $bed_dir"
            missing_bed_tags=$((missing_bed_tags + 1))
            return 1
        fi

        echo "ERROR: sweep BED directory not found for $tag: $bed_dir" >&2
        exit 1
    fi

    shopt -s nullglob
    local bed_files=("$bed_dir"/*.bed)
    shopt -u nullglob

    if [[ "${#bed_files[@]}" -eq 0 ]]; then
        if [[ "$DRY_RUN" -eq 1 ]]; then
            echo "[dry-run] no BED files found for $tag: $bed_dir"
            missing_bed_tags=$((missing_bed_tags + 1))
            return 1
        fi

        echo "ERROR: no BED files found for $tag: $bed_dir" >&2
        exit 1
    fi

    COLLECTED_BED_FILES=("${bed_files[@]}")
}

record_bed_regions() {
    local tag="$1"
    local bed_file="$2"
    local bam_rel="${BAM_MAP[$tag]}"
    local bed_chrom
    local start
    local end

    while IFS=$'\t' read -r bed_chrom start end _rest; do
        [[ -z "${bed_chrom:-}" ]] && continue
        printf '%s:%d-%d\t%s\n' "$bed_chrom" "$((start + 1))" "$end" "$bam_rel" >> "$REGION_BAM_FILE"
        printf '%d\n' "$((end - start))" >> "$WINDOW_SIZES_FILE"
    done < "$bed_file"
}

prepare_output_root() {
    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[dry-run] would reset output root: $OUTPUT_ROOT"
        return 0
    fi

    rm -rf "$OUTPUT_ROOT"
    mkdir -p "$OUTPUT_ROOT"
}

count_jsonl_files() {
    local dir="$1"
    find "$dir" -maxdepth 1 -type f -name '*.jsonl' | wc -l | tr -d '[:space:]'
}

run_vardict_for_bed() {
    local tag="$1"
    local bed_file="$2"
    local chrom="$3"
    local bam="$(resolve_project_path "${BAM_MAP[$tag]}")"
    local log_dir="$OUTPUT_ROOT/logs/$tag"
    local log_file="$log_dir/$chrom.log"
    local -a env_assignments=()
    local idx

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[dry-run] tag=$tag chrom=$chrom bed=$bed_file"
        for idx in "${!MODULE_DIRS[@]}"; do
            echo "[dry-run]   would set ${MODULE_ENVS[$idx]}=$OUTPUT_ROOT/${MODULE_DIRS[$idx]}/$chrom"
        done
        echo "[dry-run]   would run: $VARDICT_BIN -G $REF -b $bam -th 1 -c 1 -S 2 -E 3 -s 2 -e 3 $bed_file"
        echo "[dry-run]   would write log: $log_file"
        echo "[dry-run]   would compress JSONL files under $OUTPUT_ROOT/{${MODULE_DIRS[*]}}/$chrom/"
        return 0
    fi

    mkdir -p "$log_dir"
    # Stage to temporary directories; promote atomically after compression
    for idx in "${!MODULE_DIRS[@]}"; do
        mkdir -p "$OUTPUT_ROOT/${MODULE_DIRS[$idx]}/$chrom.staging"
        env_assignments+=("${MODULE_ENVS[$idx]}=$OUTPUT_ROOT/${MODULE_DIRS[$idx]}/$chrom.staging")
    done

    env "${env_assignments[@]}" \
        "$VARDICT_BIN" -G "$REF" -b "$bam" -th 1 -c 1 -S 2 -E 3 -s 2 -e 3 "$bed_file" \
        > /dev/null 2> "$log_file"

    local generated_count=0
    local compressed_count=0
    local fixture_dir
    local count
    for fixture_dir in "${MODULE_DIRS[@]}"; do
        local staging_dir="$OUTPUT_ROOT/$fixture_dir/$chrom.staging"
        local final_dir="$OUTPUT_ROOT/$fixture_dir/$chrom"
        count="$(count_jsonl_files "$staging_dir")"
        generated_count=$((generated_count + count))
        if [[ "$count" -gt 0 ]]; then
            find "$staging_dir" -maxdepth 1 -type f -name '*.jsonl' -exec zstd --rm -q {} +
        fi
        compressed_count=$((compressed_count + count))
        # Atomic promotion: staging → final
        rm -rf "$final_dir"
        mv "$staging_dir" "$final_dir"
    done

    generated_fixtures=$((generated_fixtures + generated_count))
    compressed_fixtures=$((compressed_fixtures + compressed_count))
    printf '[%s] %s: %d fixtures generated, %d compressed\n' "$tag" "$chrom" "$generated_count" "$compressed_count"
}

scan_fixture_regions() {
    local module
    local file
    local chrom
    local base
    local remainder
    local start
    local end

    : > "$FIXTURE_REGION_FILE"
    for module in "${MODULE_DIRS[@]}"; do
        if [[ ! -d "$OUTPUT_ROOT/$module" ]]; then
            continue
        fi

        while IFS= read -r file; do
            chrom="$(basename "$(dirname "$file")")"
            base="$(basename "$file")"
            remainder="${base#${module}_${chrom}_}"
            remainder="${remainder%.jsonl.zst}"
            start="${remainder%%_*}"
            end="${remainder##*_}"

            if [[ -n "$start" && -n "$end" && "$start" =~ ^[0-9]+$ && "$end" =~ ^[0-9]+$ ]]; then
                printf '%s:%s-%s\n' "$chrom" "$start" "$end" >> "$FIXTURE_REGION_FILE"
            fi
        done < <(find "$OUTPUT_ROOT/$module" -type f -name '*.jsonl.zst' | sort)
    done
}

write_regions_tsv() {
    local regions_file="$OUTPUT_ROOT/regions.tsv"
    local unique_fixture_regions="$TEMP_DIR/fixture_regions.unique.tsv"
    local unique_region_bams="$TEMP_DIR/region_bams.unique.tsv"

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[dry-run] would write regions.tsv to $regions_file by scanning fixture filenames and deduplicating"
        return 0
    fi

    scan_fixture_regions
    sort -u "$FIXTURE_REGION_FILE" > "$unique_fixture_regions"
    sort -u "$REGION_BAM_FILE" > "$unique_region_bams"

    : > "$regions_file"
    while IFS= read -r region; do
        [[ -z "$region" ]] && continue
        awk -F '\t' -v region="$region" '$1 == region { print $2 }' "$unique_region_bams" \
            | sort -u \
            | while IFS= read -r bam_rel; do
                [[ -z "$bam_rel" ]] && continue
                printf '%s\t%s\t%s\n' "$region" "$bam_rel" "$REF_REL" >> "$regions_file"
            done
    done < "$unique_fixture_regions"
}

write_manifest() {
    local manifest_file="$OUTPUT_ROOT/manifest.json"
    local vardict_commit="$1"
    local region_count="$2"
    local fixture_count="$3"
    local unique_window_sizes="$TEMP_DIR/window_sizes.unique.tsv"
    local first=1
    local tag
    local size

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[dry-run] would write manifest.json to $manifest_file"
        return 0
    fi

    sort -n -u "$WINDOW_SIZES_FILE" > "$unique_window_sizes"

    {
        printf '{\n'
        printf '  "generated_at": "%s",\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
        printf '  "vardictjava_commit": "%s",\n' "$(json_escape "$vardict_commit")"
        printf '  "output_root": "%s",\n' "$(json_escape "tmp/sweep_fixtures")"
        printf '  "reference": "%s",\n' "$(json_escape "$REF_REL")"
        printf '  "bam_tags": ['
        first=1
        for tag in "${SELECTED_BAM_TAGS[@]}"; do
            if [[ "$first" -eq 0 ]]; then
                printf ', '
            fi
            printf '"%s"' "$(json_escape "$tag")"
            first=0
        done
        printf '],\n'
        printf '  "bam_paths": {'
        first=1
        for tag in "${SELECTED_BAM_TAGS[@]}"; do
            if [[ "$first" -eq 0 ]]; then
                printf ', '
            fi
            printf '"%s": "%s"' "$(json_escape "$tag")" "$(json_escape "${BAM_MAP[$tag]}")"
            first=0
        done
        printf '},\n'
        printf '  "region_count": %s,\n' "$region_count"
        printf '  "fixture_count": %s,\n' "$fixture_count"
        printf '  "window_sizes_bp": ['
        first=1
        while IFS= read -r size; do
            [[ -z "$size" ]] && continue
            if [[ "$first" -eq 0 ]]; then
                printf ', '
            fi
            printf '%s' "$size"
            first=0
        done < "$unique_window_sizes"
        printf ']\n'
        printf '}\n'
    } > "$manifest_file"
}

main() {
    parse_args "$@"
    init_temp_files

    echo "=== sweep_fixtures ==="
    echo "Project root:  $PROJECT_ROOT"
    echo "Sweep BEDs:    $SWEEP_BED_ROOT"
    echo "Output root:   $OUTPUT_ROOT"
    echo "BAM tags:      $(join_by_comma "${SELECTED_BAM_TAGS[@]}")"
    echo "Dry run:       $DRY_RUN"
    echo ""

    check_dependencies
    ensure_vardict_bin
    prepare_output_root

    local tag
    local bed_file
    local chrom
    local vardict_commit
    local regions_file
    local region_count=0
    local fixture_count=0

    for tag in "${SELECTED_BAM_TAGS[@]}"; do
        planned_tags=$((planned_tags + 1))
        if ! collect_bed_files "$tag"; then
            continue
        fi

        for bed_file in "${COLLECTED_BED_FILES[@]}"; do
            [[ -z "$bed_file" ]] && continue
            chrom="$(basename "$bed_file" .bed)"
            planned_chromosomes=$((planned_chromosomes + 1))
            record_bed_regions "$tag" "$bed_file"
            run_vardict_for_bed "$tag" "$bed_file" "$chrom"
            if [[ "$DRY_RUN" -eq 0 ]]; then
                processed_chromosomes=$((processed_chromosomes + 1))
            fi
        done

        if [[ "$DRY_RUN" -eq 0 ]]; then
            processed_tags=$((processed_tags + 1))
        fi
    done

    if [[ "$DRY_RUN" -eq 0 ]]; then
        vardict_commit="$(git -C "$VARDICT_DIR" rev-parse HEAD)"
        write_regions_tsv
        regions_file="$OUTPUT_ROOT/regions.tsv"
        if [[ -f "$regions_file" ]]; then
            region_count="$(grep -cve '^[[:space:]]*$' "$regions_file")"
        fi
        fixture_count="$(find "$OUTPUT_ROOT" -type f -name '*.jsonl.zst' | wc -l | tr -d '[:space:]')"
        write_manifest "$vardict_commit" "$region_count" "$fixture_count"
    else
        echo "[dry-run] would resolve VarDictJava commit with: git -C $VARDICT_DIR rev-parse HEAD"
        write_regions_tsv
        write_manifest "DRY_RUN" 0 0
    fi

    echo ""
    echo "=== Summary ==="
    echo "Planned tags:        $planned_tags"
    echo "Planned chromosomes: $planned_chromosomes"
    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "Missing BED tags:    $missing_bed_tags"
    else
        echo "Processed tags:      $processed_tags"
        echo "Processed chroms:    $processed_chromosomes"
        echo "Fixtures generated:  $generated_fixtures"
        echo "Fixtures compressed: $compressed_fixtures"
        echo "Regions written:     $region_count"
        echo "Manifest:            $OUTPUT_ROOT/manifest.json"
    fi
}

main "$@"