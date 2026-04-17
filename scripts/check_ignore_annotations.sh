#!/usr/bin/env bash
# scripts/check_ignore_annotations.sh
#
# Enforces that every #[ignore] in tests/ and src/ has a reason string.
# Preferred form: #[ignore = "<category prefix>: <description>"] or a GitHub issue URL.
#
# Exit codes:
#   0  — all #[ignore] annotations are documented
#   1  — one or more bare #[ignore] found
#
# Used in CI by .github/workflows/parity.yml (ignore-audit job).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

FAILED=0

while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    code="${rest#*:}"
    # Accept: #[ignore = "..."] or #[ignore("reason")]
    # Reject:  #[ignore] and #[ignore = ""] (empty reason)
    trimmed=$(echo "$code" | sed 's/^[[:space:]]*//')
    if [[ "$trimmed" =~ ^#\[ignore\][[:space:]]*$ ]] \
       || [[ "$trimmed" =~ ^#\[ignore\][[:space:]]*]$ ]]; then
        echo "::error file=$file,line=$lineno::bare #[ignore] without reason — add #[ignore = \"<category>: <reason>\"] and, when applicable, link to a tracking issue"
        FAILED=1
    elif [[ "$trimmed" =~ ^#\[ignore[[:space:]]*=[[:space:]]*\"\"[[:space:]]*\] ]]; then
        echo "::error file=$file,line=$lineno::empty reason in #[ignore = \"\"] — add a meaningful category prefix and description"
        FAILED=1
    fi
done < <(grep -rn --include='*.rs' "^[[:space:]]*#\[ignore" tests/ src/ 2>/dev/null || true)

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "FAILED: one or more #[ignore] annotations are undocumented."
    echo "Fix by adding a reason string: #[ignore = \"<category>: <description>\"]"
    exit 1
fi

# Also report a summary of documented ignores for visibility.
count=$(grep -rn --include='*.rs' "^[[:space:]]*#\[ignore" tests/ src/ 2>/dev/null | wc -l | tr -d ' ')
echo "OK: ${count} #[ignore] annotations checked, all documented."
