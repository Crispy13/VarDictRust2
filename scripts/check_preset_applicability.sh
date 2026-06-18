#!/usr/bin/env bash
# check_preset_applicability.sh — assert TSV presets use flags consistent with applies_to
#
# Scans scripts/config_presets.tsv. For each row:
# - applies_to=germline: flags must NOT include somatic-only ones (-M, -V, -I, -A)
# - applies_to=somatic: flags may include any
# - applies_to=both: flags must NOT include somatic-only ones (else they'd break germline lane)
#
# Also asserts:
# - Every flag used is in the covered-flag set from docs/parity-scope.md
# - Somatic-only flag rows use applies_to=somatic, not applies_to=both
#
# Exit status:
#   0 — no inconsistencies
#   1 — inconsistency detected
#
# Invocation:
#   bash scripts/check_preset_applicability.sh
#   bash scripts/check_preset_applicability.sh --verbose

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TSV="${REPO_ROOT}/scripts/config_presets.tsv"

VERBOSE=0
for arg in "$@"; do
  case "$arg" in
    -v|--verbose) VERBOSE=1 ;;
    -h|--help) sed -n '1,25p' "${BASH_SOURCE[0]}"; exit 0 ;;
  esac
done

log() { if [[ "$VERBOSE" -eq 1 ]]; then echo "[check_preset_applicability] $*"; fi; return 0; }

[[ -f "$TSV" ]] || { echo "[check_preset_applicability] MISSING: $TSV" >&2; exit 1; }

# Somatic-only flags (call-mode): -M min-tumor-mapq, -V tumor-vaf, -I tumor-indel-dist, -A ambiguous-ref
# These are meaningless in SimpleMode (germline) and must only appear in somatic or applies_to=somatic rows.
SOMATIC_ONLY_FLAGS="-M -V -I -A"

# Covered flag set (from docs/parity-scope.md). Unknown flags are flagged as warnings
# to encourage keeping the scope doc in sync.
COVERED_FLAGS="-f -r -q -m -X -B --fisher -p -U -k --chimeric -Q -th -M -V -I --adaptor"

fail_count=0
warn_count=0

# Use awk to emit NAME|FLAGS|APPLIES_TO (pipe-separated; empty flags preserved)
# because bash `read -r` with IFS=$'\t' collapses consecutive tabs.
while IFS='|' read -r name flags applies_to; do
  [[ -z "$name" ]] && continue

  # Default applies_to if absent (backward compat)
  applies_to="${applies_to:-both}"

  # Parse flag tokens (ignore values; only inspect the flag names)
  flag_tokens=()
  next_is_value=0
  for tok in $flags; do
    if [[ $next_is_value -eq 1 ]]; then
      next_is_value=0
      continue
    fi
    if [[ "$tok" =~ ^- ]]; then
      flag_tokens+=("$tok")
      # Flag takes a value unless it's one of the boolean flags
      case "$tok" in
        --fisher|-p|-U|--chimeric) next_is_value=0 ;;
        *) next_is_value=1 ;;
      esac
    fi
  done

  # Check 1: somatic-only flags only in applies_to=somatic rows
  for tok in "${flag_tokens[@]:-}"; do
    [[ -z "$tok" ]] && continue
    for bad in $SOMATIC_ONLY_FLAGS; do
      if [[ "$tok" == "$bad" && "$applies_to" != "somatic" ]]; then
        echo "[check_preset_applicability] FAIL: $name uses somatic-only flag $tok but applies_to=$applies_to" >&2
        echo "  Fix: change applies_to to 'somatic' or remove the flag." >&2
        fail_count=$((fail_count + 1))
      fi
    done
  done

  # Check 2: all flags are in the covered-flag set (warning, not error)
  for tok in "${flag_tokens[@]:-}"; do
    [[ -z "$tok" ]] && continue
    is_covered=0
    for covered in $COVERED_FLAGS; do
      if [[ "$tok" == "$covered" ]]; then
        is_covered=1
        break
      fi
    done
    if [[ $is_covered -eq 0 ]]; then
      echo "[check_preset_applicability] WARN: $name uses flag $tok not listed in docs/parity-scope.md covered-flag set" >&2
      warn_count=$((warn_count + 1))
    fi
  done

  log "$name: applies_to=$applies_to flags=(${flag_tokens[*]:-none}) OK"
done < <(awk -F'\t' '
  NR>1 && !/^[[:space:]]*$/ && $1 !~ /^#/ {
    applies = (NF >= 5 ? $5 : "both")
    print $1"|"$2"|"applies
  }
' "$TSV")

if [[ $fail_count -gt 0 ]]; then
  echo "[check_preset_applicability] FAIL: $fail_count inconsistency/ies found" >&2
  exit 1
fi

if [[ $warn_count -gt 0 ]]; then
  echo "[check_preset_applicability] PASS with $warn_count warning(s)"
else
  echo "[check_preset_applicability] PASS"
fi
