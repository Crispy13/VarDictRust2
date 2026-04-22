#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export VARDICT_IMPL="${VARDICT_IMPL:-rust}"
ALLOWLIST_FILE="${ALLOWLIST_FILE:-$SCRIPT_DIR/ignored_tests_allowlist.txt}"

if [[ ! -f "$ALLOWLIST_FILE" ]]; then
    echo "ERROR: allowlist not found: $ALLOWLIST_FILE" >&2
    exit 1
fi

cd "$PROJECT_ROOT"

echo "Running ignored tests audit..."

cargo_output="$(cargo test --profile debug-release --color=never -- --ignored --test-threads=1 2>&1 || true)"

declare -A allowlisted_tests=()
declare -a allowlisted_prefixes=()
declare -a passing_tests=()
declare -a unexpected_passes=()

while IFS= read -r test_name; do
    [[ -z "$test_name" ]] && continue
    if [[ "$test_name" == prefix:* ]]; then
        test_name="${test_name#prefix:}"
        if [[ -z "$test_name" ]]; then
            echo "ERROR: malformed prefix entry: prefix:" >&2
            exit 1
        fi
        allowlisted_prefixes+=("$test_name")
        continue
    fi
    allowlisted_tests["$test_name"]=1
done < <(grep -Ev '^[[:space:]]*($|#)' "$ALLOWLIST_FILE")

total_results=0
while IFS= read -r line; do
    if [[ "$line" =~ ^test[[:space:]]+([^[:space:]]+)[[:space:]]+\.\.\.[[:space:]]+(ok|FAILED|ignored)$ ]]; then
        test_name="${BASH_REMATCH[1]}"
        status="${BASH_REMATCH[2]}"
        total_results=$((total_results + 1))
        if [[ "$status" == "ok" ]]; then
            passing_tests+=("$test_name")
            is_allowlisted="${allowlisted_tests[$test_name]+x}"
            if [[ -z "$is_allowlisted" ]]; then
                for prefix in "${allowlisted_prefixes[@]}"; do
                    if [[ "$test_name" == "$prefix"* ]]; then
                        is_allowlisted=1
                        break
                    fi
                done
            fi
            if [[ -z "$is_allowlisted" ]]; then
                unexpected_passes+=("$test_name")
            fi
        fi
    fi
done <<< "$cargo_output"

if (( total_results == 0 )); then
    echo "WARN: no ignored test results were parsed from cargo output." >&2
    echo "$cargo_output"
    exit 2
fi

if (( ${#unexpected_passes[@]} > 0 )); then
    echo "Unexpected ignored tests passed:" >&2
    printf '  %s\n' "${unexpected_passes[@]}" >&2
    echo ""
    echo "Passing ignored tests: ${#passing_tests[@]}" >&2
    exit 1
fi

echo "Ignored test audit complete: ${#passing_tests[@]} passing ignored test(s), all allowlisted."
if (( ${#passing_tests[@]} > 0 )); then
    printf '  %s\n' "${passing_tests[@]}"
fi