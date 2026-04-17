#!/usr/bin/env bash
# scripts/bisect_parity.sh — M8 git-bisect helper.
#
# Used as the `run` script for `git bisect run`. Exits:
#   0   — commit is GOOD (test passes)
#   1   — commit is BAD (test fails at runtime)
#   125 — commit is SKIP (build failed; bisect should skip)
#
# Usage:
#   git bisect start <bad-commit> <good-commit>
#   git bisect run scripts/bisect_parity.sh <test-name> [-- <test args>]
#
# Examples:
#   git bisect run scripts/bisect_parity.sh parity_e2e parity_e2e_push
#   git bisect run scripts/bisect_parity.sh parity_cigar_parser
#
# Requires rust_build_env to be activated in the caller's shell and
# LIBCLANG_PATH to be set.

set -uo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <test-binary> [test-filter] [-- extra args]" >&2
    exit 2
fi

TEST_BIN="$1"
shift

TEST_FILTER=""
if [[ $# -gt 0 && "$1" != "--" ]]; then
    TEST_FILTER="$1"
    shift
fi

if [[ "${1:-}" == "--" ]]; then
    shift
fi

# Build first; skip commit on build failure.
if ! cargo build --profile debug-release --test "$TEST_BIN" >/dev/null 2>&1; then
    echo "BUILD_FAIL at $(git rev-parse --short HEAD) — skipping commit" >&2
    exit 125
fi

# Run the test.
if [[ -n "$TEST_FILTER" ]]; then
    cargo test --profile debug-release --test "$TEST_BIN" "$TEST_FILTER" -- --test-threads=1 "$@"
else
    cargo test --profile debug-release --test "$TEST_BIN" -- --test-threads=1 "$@"
fi
status=$?

if [[ $status -eq 0 ]]; then
    echo "GOOD at $(git rev-parse --short HEAD)" >&2
    exit 0
else
    echo "BAD at $(git rev-parse --short HEAD) — exit $status" >&2
    exit 1
fi
