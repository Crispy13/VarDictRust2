#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_FILE="$PROJECT_ROOT/tmp/parity_status_output.txt"

MODULES=(
    cigar_parser
    cigar_modifier
    realigner
    sam_file_parser
    sv_processor
    tovars
)

cd "$PROJECT_ROOT"
mkdir -p tmp

cargo_output="$(
    cargo test --profile debug-release --color=never \
        --test parity_cigar_parser \
        --test parity_cigar_modifier \
        --test parity_realigner \
        --test parity_sam_file_parser \
        --test parity_sv_processor \
        --test parity_tovars \
        2>&1 || true
)"

printf '%s\n' "$cargo_output" > "$OUTPUT_FILE"

declare -A module_statuses=()
pass_count=0
fail_count=0
ignored_count=0
error_count=0

for module in "${MODULES[@]}"; do
    result_line="$(grep -E "^test[[:space:]]+parity_${module}_all_regions[[:space:]]+\.\.\.[[:space:]]+(ok|FAILED|ignored)$" "$OUTPUT_FILE" | tail -n 1 || true)"

    if [[ -z "$result_line" ]]; then
        module_statuses["$module"]="ERROR"
        error_count=$((error_count + 1))
        continue
    fi

    case "$result_line" in
        *" ok")
            module_statuses["$module"]="PASS"
            pass_count=$((pass_count + 1))
            ;;
        *" FAILED")
            module_statuses["$module"]="FAIL"
            fail_count=$((fail_count + 1))
            ;;
        *" ignored")
            module_statuses["$module"]="IGNORED"
            ignored_count=$((ignored_count + 1))
            ;;
        *)
            module_statuses["$module"]="ERROR"
            error_count=$((error_count + 1))
            ;;
    esac
done

printf '=== Parity Status Report ===\n\n'
printf '%-23s %s\n' 'Module' 'Status'
printf '%-23s %s\n' '-----------------------' '------'

for module in "${MODULES[@]}"; do
    printf '%-23s %s\n' "$module" "${module_statuses[$module]}"
done

printf '\nSummary: %d pass, %d fail, %d ignored, %d error\n' \
    "$pass_count" "$fail_count" "$ignored_count" "$error_count"

if [[ "$fail_count" -gt 0 || "$error_count" -gt 0 ]]; then
    exit 1
fi

exit 0