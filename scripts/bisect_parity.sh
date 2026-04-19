#!/usr/bin/env bash
set -uo pipefail

usage() {
    echo "Usage: $0 <module> <region_index>" >&2
    echo "  module       - one of: cigar_parser cigar_modifier realigner sam_file_parser sv_processor tovars" >&2
    echo "  region_index - zero-based parity region index" >&2
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ $# -ne 2 ]]; then
    usage
    exit 125
fi

MODULE="$1"
REGION_INDEX="$2"

case "$MODULE" in
    cigar_parser|cigar_modifier|realigner|sam_file_parser|sv_processor|tovars)
        ;;
    *)
        echo "ERROR: invalid module '$MODULE'" >&2
        usage
        exit 125
        ;;
esac

if [[ ! "$REGION_INDEX" =~ ^[0-9]+$ ]]; then
    echo "ERROR: region_index must be a non-negative integer, got '$REGION_INDEX'" >&2
    usage
    exit 125
fi

if [[ -z "${CONDA_PREFIX:-}" ]]; then
    echo "ERROR: CONDA_PREFIX is not set. Activate rust_build_env before running this script." >&2
    exit 125
fi

cd "$PROJECT_ROOT"

export LIBCLANG_PATH="$CONDA_PREFIX/lib"

# rust_build_env may leave a stray target triplet token in these flags; strip it so cargo can build.
for var_name in CFLAGS CPPFLAGS CXXFLAGS LDFLAGS; do
    current_value="${!var_name-}"
    if [[ -n "$current_value" ]]; then
        sanitized_value="$(printf '%s' "$current_value" | sed 's/\(^\| \)x86_64-conda_cos6-linux-gnu\($\| \)/ /g; s/  */ /g; s/^ //; s/ $//')"
        export "$var_name=$sanitized_value"
    fi
done

echo "==> Running parity_${MODULE} for region index ${REGION_INDEX}" >&2

PARITY_REGION_INDEX="$REGION_INDEX" cargo test \
    --profile debug-release \
    --color=never \
    --test "parity_${MODULE}" \
    -- \
    --include-ignored
cargo_status=$?

case "$cargo_status" in
    0)
        exit 0
        ;;
    101)
        exit 1
        ;;
    *)
        echo "ERROR: cargo test exited with status $cargo_status; treating commit as untestable." >&2
        exit 125
        ;;
esac