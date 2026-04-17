#!/usr/bin/env bash
# scripts/parity_status.sh — M8 progress tracker.
#
# Runs each Tier 1 parity test (push subset), counts pass/fail, and prints
# a summary per module. For full sweep coverage, use sweep.yml CI instead.
#
# Usage:
#   scripts/parity_status.sh                 # all modules
#   scripts/parity_status.sh cigar_parser    # single module
#
# Requires: rust_build_env activated, LIBCLANG_PATH set.

set -uo pipefail

MODULES=(cigar_modifier cigar_parser realigner sam_file_parser sv_processor tovars)
# E2E entries: "test_name:extra_args" (e.g., include-ignored for config-variant suite).
E2E_TESTS=(
    "parity_e2e:"
    "parity_e2e_config:--include-ignored"
)

targets=("$@")
if [[ ${#targets[@]} -eq 0 ]]; then
    targets=("${MODULES[@]}" e2e)
fi

print_header() {
    printf '\n%-24s  %6s  %6s  %6s  %s\n' "MODULE" "PASS" "FAIL" "IGNORE" "STATUS"
    printf '%-24s  %6s  %6s  %6s  %s\n' "------" "----" "----" "------" "------"
}

run_test() {
    local test_name="$1"
    local extra_args="${2:-}"
    local output
    if [[ -n "$extra_args" ]]; then
        output=$(cargo test --profile debug-release --test "$test_name" -- --test-threads=1 $extra_args 2>&1 || true)
    else
        output=$(cargo test --profile debug-release --test "$test_name" -- --test-threads=1 2>&1 || true)
    fi
    # Match lines like: "test result: ok. 6 passed; 0 failed; 0 ignored; ..."
    local line
    line=$(echo "$output" | grep -E "^test result:" | tail -1)
    if [[ -z "$line" ]]; then
        echo "BUILD_FAIL 0 0 0"
        return
    fi
    local passed failed ignored status
    passed=$(echo "$line" | grep -oE '[0-9]+ passed' | head -1 | awk '{print $1}')
    failed=$(echo "$line" | grep -oE '[0-9]+ failed' | head -1 | awk '{print $1}')
    ignored=$(echo "$line" | grep -oE '[0-9]+ ignored' | head -1 | awk '{print $1}')
    if [[ "${failed:-0}" -gt 0 ]]; then
        status="FAIL"
    elif [[ "${passed:-0}" -gt 0 ]]; then
        status="PASS"
    else
        status="EMPTY"
    fi
    echo "$status ${passed:-0} ${failed:-0} ${ignored:-0}"
}

print_header
overall_fail=0
for target in "${targets[@]}"; do
    if [[ "$target" == "e2e" ]]; then
        for entry in "${E2E_TESTS[@]}"; do
            t="${entry%%:*}"
            extra="${entry#*:}"
            read -r status p f i <<< "$(run_test "$t" "$extra")"
            printf '%-24s  %6s  %6s  %6s  %s\n' "$t" "$p" "$f" "$i" "$status"
            [[ "$status" == "FAIL" || "$status" == "BUILD_FAIL" ]] && overall_fail=1
        done
    else
        read -r status p f i <<< "$(run_test "parity_$target")"
        printf '%-24s  %6s  %6s  %6s  %s\n' "parity_$target" "$p" "$f" "$i" "$status"
        [[ "$status" == "FAIL" || "$status" == "BUILD_FAIL" ]] && overall_fail=1
    fi
done

echo ""
if [[ $overall_fail -eq 0 ]]; then
    echo "Overall: PASS"
    exit 0
else
    echo "Overall: FAIL — see module summary above"
    exit 1
fi
