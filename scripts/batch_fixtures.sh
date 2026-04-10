#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/batch_fixtures.sh --mode aa|generate

Modes:
  aa        Run VarDictJava twice per region and diff parity JSONL output.
  generate  Run VarDictJava once per region and install JSONL fixtures.
EOF
}

if [[ $# -ne 2 || "$1" != "--mode" ]]; then
    usage >&2
    exit 1
fi

MODE="$2"
if [[ "$MODE" != "aa" && "$MODE" != "generate" ]]; then
    echo "ERROR: --mode must be 'aa' or 'generate'" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGIONS_FILE="$PROJECT_ROOT/testdata/parity_regions.tsv"
TMP_ROOT="$PROJECT_ROOT/tmp/batch_fixtures"
VARDICT_DIR="$PROJECT_ROOT/VarDictJava"
VARDICT_BIN="$VARDICT_DIR/build/install/VarDict/bin/VarDict"

MODULE_DIRS=(cigar_parser realigner sv_processor tovars)
MODULE_ENVS=(
    VARDICT_PARITY_CIGAR_PARSER
    VARDICT_PARITY_REALIGNER
    VARDICT_PARITY_SV_PROCESSOR
    VARDICT_PARITY_TOVARS
)

total_regions=0
passed_regions=0
failed_regions=0
skipped_regions=0
run_failures=0
copied_fixtures=0
invalid_fixtures=0
module_counts=(0 0 0 0)
module_skips=(0 0 0 0)

resolve_project_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s\n' "$PROJECT_ROOT/$path"
    fi
}

region_tag() {
    local index="$1"
    local region="$2"
    local safe_region="${region//:/_}"
    safe_region="${safe_region//-/_}"
    printf '%03d_%s\n' "$index" "$safe_region"
}

count_jsonl_files() {
    local dir="$1"
    find "$dir" -type f -name '*.jsonl' | wc -l | tr -d '[:space:]'
}

prepare_module_dirs() {
    local root="$1"
    local module_dir
    mkdir -p "$root"
    for module_dir in "${MODULE_DIRS[@]}"; do
        mkdir -p "$root/$module_dir"
    done
}

run_vardict() {
    local out_root="$1"
    local region="$2"
    local bam="$3"
    local ref="$4"
    local log_file="$5"

    local -a env_args=()
    local idx
    for idx in "${!MODULE_DIRS[@]}"; do
        env_args+=("${MODULE_ENVS[$idx]}=$out_root/${MODULE_DIRS[$idx]}")
    done

    if env "${env_args[@]}" \
        "$VARDICT_BIN" \
        -G "$ref" \
        -b "$bam" \
        -th 1 \
        -R "$region" \
        > /dev/null 2> "$log_file"; then
        return 0
    fi

    return 1
}

install_region_fixtures() {
    local source_root="$1"
    local copied_any=0
    local idx

    for idx in "${!MODULE_DIRS[@]}"; do
        local module_dir="${MODULE_DIRS[$idx]}"
        local fixture_dir="$PROJECT_ROOT/testdata/fixtures/$module_dir"
        local source_file
        source_file="$(find "$source_root/$module_dir" -maxdepth 1 -type f -name '*.jsonl' | head -n 1)"

        if [[ -z "$source_file" ]]; then
            module_skips[$idx]=$((module_skips[$idx] + 1))
            continue
        fi

        cp "$source_file" "$fixture_dir/"
        zstd --rm -q "$fixture_dir/$(basename "$source_file")"
        copied_any=1
        copied_fixtures=$((copied_fixtures + 1))
        module_counts[$idx]=$((module_counts[$idx] + 1))

        local dest_file="$fixture_dir/$(basename "$source_file").zst"
        local line_count
        line_count="$(zstd -dcq "$dest_file" | wc -l | tr -d '[:space:]')"
        if [[ "$line_count" -ne 2 ]]; then
            echo "WARN: fixture does not have 2 lines: $dest_file" >&2
            invalid_fixtures=$((invalid_fixtures + 1))
        fi
    done

    if [[ "$copied_any" -eq 1 ]]; then
        return 0
    fi

    return 1
}

echo "=== batch_fixtures: $MODE ==="
echo "Project root: $PROJECT_ROOT"
echo "Regions file: $REGIONS_FILE"
echo "Output root:  $TMP_ROOT"
echo ""

if ! command -v zstd &>/dev/null; then
    echo "ERROR: zstd is required but not found in PATH." >&2
    echo "       Activate the conda environment first: conda activate rust_build_env" >&2
    exit 1
fi

if [[ ! -f "$REGIONS_FILE" ]]; then
    echo "ERROR: regions file not found: $REGIONS_FILE" >&2
    exit 1
fi

echo "--- Building VarDictJava ---"
(cd "$VARDICT_DIR" && ./gradlew installDist -q)

if [[ ! -x "$VARDICT_BIN" ]]; then
    echo "ERROR: VarDictJava binary not found at $VARDICT_BIN" >&2
    exit 1
fi

MODE_ROOT="$TMP_ROOT/$MODE"
rm -rf "$MODE_ROOT"
mkdir -p "$MODE_ROOT"

if [[ "$MODE" == "generate" ]]; then
    local_fixture_dir=""
    echo "--- Resetting fixture directories ---"
    for local_fixture_dir in "${MODULE_DIRS[@]}"; do
        mkdir -p "$PROJECT_ROOT/testdata/fixtures/$local_fixture_dir"
        rm -f "$PROJECT_ROOT/testdata/fixtures/$local_fixture_dir"/*.jsonl.zst
    done
fi

total_regions="$(grep -cve '^[[:space:]]*$' "$REGIONS_FILE")"
current_region=0

while IFS=$'\t' read -r region bam ref _rest; do
    if [[ -z "${region:-}" ]]; then
        continue
    fi

    current_region=$((current_region + 1))
    bam="$(resolve_project_path "$bam")"
    ref="$(resolve_project_path "$ref")"
    tag="$(region_tag "$current_region" "$region")"

    printf '[%d/%d] %s %s\n' "$current_region" "$total_regions" "$MODE" "$region"

    if [[ "$MODE" == "aa" ]]; then
        run_a_root="$MODE_ROOT/run_a/$tag"
        run_b_root="$MODE_ROOT/run_b/$tag"
        prepare_module_dirs "$run_a_root"
        prepare_module_dirs "$run_b_root"

        log_a="$MODE_ROOT/logs/${tag}_run_a.log"
        log_b="$MODE_ROOT/logs/${tag}_run_b.log"
        mkdir -p "$MODE_ROOT/logs" "$MODE_ROOT/diffs"

        run_a_ok=1
        run_b_ok=1
        if ! run_vardict "$run_a_root" "$region" "$bam" "$ref" "$log_a"; then
            run_a_ok=0
            run_failures=$((run_failures + 1))
            echo "  FAIL run_a: $region (see $log_a)" >&2
        fi

        if ! run_vardict "$run_b_root" "$region" "$bam" "$ref" "$log_b"; then
            run_b_ok=0
            run_failures=$((run_failures + 1))
            echo "  FAIL run_b: $region (see $log_b)" >&2
        fi

        if [[ "$run_a_ok" -eq 0 || "$run_b_ok" -eq 0 ]]; then
            failed_regions=$((failed_regions + 1))
            continue
        fi

        files_a="$(count_jsonl_files "$run_a_root")"
        files_b="$(count_jsonl_files "$run_b_root")"
        if [[ "$files_a" -eq 0 && "$files_b" -eq 0 ]]; then
            skipped_regions=$((skipped_regions + 1))
            echo "  SKIP empty output"
            continue
        fi

        diff_file="$MODE_ROOT/diffs/${tag}.diff"
        if diff -ru "$run_a_root" "$run_b_root" > "$diff_file" 2>&1; then
            rm -f "$diff_file"
            passed_regions=$((passed_regions + 1))
            echo "  PASS"
        else
            failed_regions=$((failed_regions + 1))
            echo "  FAIL diff mismatch (see $diff_file)" >&2
        fi
    else
        region_root="$MODE_ROOT/$tag"
        prepare_module_dirs "$region_root"
        mkdir -p "$MODE_ROOT/logs"
        log_file="$MODE_ROOT/logs/${tag}.log"

        if ! run_vardict "$region_root" "$region" "$bam" "$ref" "$log_file"; then
            failed_regions=$((failed_regions + 1))
            run_failures=$((run_failures + 1))
            echo "  FAIL generate run (see $log_file)" >&2
            continue
        fi

        if install_region_fixtures "$region_root"; then
            passed_regions=$((passed_regions + 1))
            echo "  INSTALLED"
        else
            skipped_regions=$((skipped_regions + 1))
            echo "  SKIP no fixture files"
        fi
    fi
done < "$REGIONS_FILE"

echo ""
echo "=== Summary ==="
echo "Mode:           $MODE"
echo "Total regions:  $total_regions"
echo "Passed:         $passed_regions"
echo "Failed:         $failed_regions"
echo "Skipped:        $skipped_regions"
echo "Run failures:   $run_failures"

if [[ "$MODE" == "generate" ]]; then
    echo "Fixtures copied: $copied_fixtures"
    echo "Invalid fixtures: $invalid_fixtures"
    echo "Module counts:"
    for idx in "${!MODULE_DIRS[@]}"; do
        printf '  %-13s copied=%d skipped=%d\n' \
            "${MODULE_DIRS[$idx]}" \
            "${module_counts[$idx]}" \
            "${module_skips[$idx]}"
    done
fi

if [[ "$failed_regions" -gt 0 || "$invalid_fixtures" -gt 0 ]]; then
    exit 1
fi

exit 0