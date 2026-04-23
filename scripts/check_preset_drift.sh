#!/usr/bin/env bash
# check_preset_drift.sh — fail fast if the preset matrix is inconsistent
#
# Source of truth precedence:
#   1. scripts/config_presets.tsv (canonical preset registry)
#   2. tests/common/mod.rs::CONFIG_PRESETS (must mirror the TSV 1:1)
#   3. .github/skills/tiered-config-test/SKILL.md (documentation; must match counts)
#
# Exit status:
#   0 — no drift
#   1 — drift detected
#   2 — required file missing
#
# Invocation:
#   bash scripts/check_preset_drift.sh
#   bash scripts/check_preset_drift.sh --verbose
#
# Wired into: .github/workflows/parity.yml as a pre-test step.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TSV="${REPO_ROOT}/scripts/config_presets.tsv"
CONST_FILE="${REPO_ROOT}/tests/common/mod.rs"
SKILL_FILE="${REPO_ROOT}/.github/skills/tiered-config-test/SKILL.md"
SCOPE_DOC="${REPO_ROOT}/docs/parity-scope.md"

VERBOSE=0
for arg in "$@"; do
  case "$arg" in
    -v|--verbose) VERBOSE=1 ;;
    -h|--help)
      sed -n '1,20p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
  esac
done

log() {
  [[ "$VERBOSE" -eq 1 ]] && echo "[check_preset_drift] $*"
}
fail() {
  echo "[check_preset_drift] FAIL: $*" >&2
  exit 1
}

# Phase 1: existence check
for f in "$TSV" "$CONST_FILE" "$SKILL_FILE" "$SCOPE_DOC"; do
  if [[ ! -f "$f" ]]; then
    echo "[check_preset_drift] MISSING: $f" >&2
    exit 2
  fi
done

# Phase 2: extract TSV preset names (skip comment header "# name...")
TSV_NAMES=$(awk -F'\t' 'NR>1 && !/^[[:space:]]*$/ && $1 !~ /^#/ {print $1}' "$TSV" | sort)
TSV_COUNT=$(echo "$TSV_NAMES" | wc -l | tr -d ' ')
log "TSV preset count: $TSV_COUNT"

# Phase 3: extract CONFIG_PRESETS const entries from Rust source
# Matches lines like:    "T1-01",
CONST_NAMES=$(awk '
  /pub const CONFIG_PRESETS: &\[&str\] = &\[/ { inside=1; next }
  inside && /^\];/ { inside=0; next }
  inside && /^[[:space:]]*"/ {
    gsub(/^[[:space:]]*"/, "");
    gsub(/",?[[:space:]]*$/, "");
    print
  }
' "$CONST_FILE" | sort)
CONST_COUNT=$(echo "$CONST_NAMES" | wc -l | tr -d ' ')
log "CONFIG_PRESETS const count: $CONST_COUNT"

# Phase 4: TSV ↔ CONFIG_PRESETS diff
if [[ "$TSV_NAMES" != "$CONST_NAMES" ]]; then
  echo "[check_preset_drift] FAIL: TSV and CONFIG_PRESETS disagree." >&2
  echo "  Only in TSV:"         >&2
  comm -23 <(echo "$TSV_NAMES") <(echo "$CONST_NAMES") | sed 's/^/    /' >&2
  echo "  Only in CONFIG_PRESETS:" >&2
  comm -13 <(echo "$TSV_NAMES") <(echo "$CONST_NAMES") | sed 's/^/    /' >&2
  echo ""                       >&2
  echo "  Fix: update tests/common/mod.rs::CONFIG_PRESETS to match scripts/config_presets.tsv." >&2
  exit 1
fi
log "TSV ↔ CONFIG_PRESETS: OK ($TSV_COUNT rows)"

# Phase 5: count by tier (T1/T2/T3/PW/CM)
T1_COUNT=$(echo "$TSV_NAMES" | grep -c '^T1-' || true)
T2_COUNT=$(echo "$TSV_NAMES" | grep -c '^T2-' || true)
T3_COUNT=$(echo "$TSV_NAMES" | grep -c '^T3-' || true)
PW_COUNT=$(echo "$TSV_NAMES" | grep -c '^PW-' || true)
CM_COUNT=$(echo "$TSV_NAMES" | grep -c '^CM-' || true)
log "Tier counts: T1=$T1_COUNT T2=$T2_COUNT T3=$T3_COUNT PW=$PW_COUNT CM=$CM_COUNT"

# Phase 6: skill-doc PW ceiling assertion.
# Strategy: extract all PW-NNN tokens from the skill doc, take the max; compare to
# the actual TSV PW count. The max seen must equal (PW_COUNT - 1).
SKILL_PW_TOKENS=$(grep -oE 'PW-[0-9]{3}' "$SKILL_FILE" | sort -u | tail -n1 || true)
if [[ -z "$SKILL_PW_TOKENS" ]]; then
  log "skill-doc contains no PW-NNN references (acceptable — doc may describe tier generically)"
else
  SKILL_PW_CEILING="${SKILL_PW_TOKENS#PW-}"
  EXPECTED_CEILING=$(printf '%03d' $((PW_COUNT - 1)))
  if [[ "$SKILL_PW_CEILING" != "$EXPECTED_CEILING" ]]; then
    echo "[check_preset_drift] FAIL: skill-doc PW range disagrees with TSV." >&2
    echo "  TSV has $PW_COUNT PW-* rows (max: PW-$EXPECTED_CEILING)" >&2
    echo "  Skill mentions: PW-$SKILL_PW_CEILING as the highest token" >&2
    echo "  Fix: update .github/skills/tiered-config-test/SKILL.md to match." >&2
    exit 1
  fi
  log "skill-doc PW max token: OK (PW-$SKILL_PW_CEILING)"
fi

# Phase 7: skill-doc must not reference files that don't exist
# Known stale references to check
STALE_REFS=(
  "tests/pairwise_configs.tsv"
  "tests/generate_pairwise_configs.py"
)
for ref in "${STALE_REFS[@]}"; do
  abs="${REPO_ROOT}/${ref}"
  if grep -qF "$ref" "$SKILL_FILE" && [[ ! -e "$abs" ]]; then
    echo "[check_preset_drift] FAIL: skill-doc references nonexistent file: $ref" >&2
    echo "  Fix: remove the reference from .github/skills/tiered-config-test/SKILL.md" >&2
    echo "       or create the file." >&2
    exit 1
  fi
done
log "skill-doc stale-reference scan: OK"

# Phase 8: parity-scope doc total-count assertion
# The scope doc states "Total: 44 rows." — update this line if the matrix changes.
SCOPE_TOTAL=$(grep -oE '\*\*Total: [0-9]+ rows\.\*\*' "$SCOPE_DOC" | grep -oE '[0-9]+' || echo "")
if [[ -z "$SCOPE_TOTAL" ]]; then
  fail "parity-scope.md missing **Total: N rows.** assertion"
fi
if [[ "$SCOPE_TOTAL" != "$TSV_COUNT" ]]; then
  echo "[check_preset_drift] FAIL: parity-scope.md total disagrees with TSV." >&2
  echo "  TSV: $TSV_COUNT rows" >&2
  echo "  parity-scope.md claims: $SCOPE_TOTAL rows" >&2
  echo "  Fix: update docs/parity-scope.md" >&2
  exit 1
fi
log "parity-scope.md total: OK ($TSV_COUNT rows)"

# Phase 9: applies_to column validation (if present)
# Schema: col 5 = applies_to, must be in {germline, somatic, both}. Absent column
# (4-col row) is treated as "both" for backward compat.
INVALID_APPLIES=$(awk -F'\t' '
  NR>1 && !/^[[:space:]]*$/ && $1 !~ /^#/ {
    a = (NF >= 5 ? $5 : "both")
    if (a != "germline" && a != "somatic" && a != "both") {
      print NR": "$1" applies_to=\""a"\""
    }
  }
' "$TSV")
if [[ -n "$INVALID_APPLIES" ]]; then
  echo "[check_preset_drift] FAIL: invalid applies_to values in TSV:" >&2
  echo "$INVALID_APPLIES" | sed 's/^/    /' >&2
  echo "  Fix: applies_to must be one of: germline, somatic, both" >&2
  exit 1
fi
log "applies_to column: OK"

echo "[check_preset_drift] PASS ($TSV_COUNT presets: T1=$T1_COUNT T2=$T2_COUNT T3=$T3_COUNT PW=$PW_COUNT CM=$CM_COUNT)"
