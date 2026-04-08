#!/usr/bin/env bash
set -euo pipefail

# A-A Gate: Run VarDictJava twice with parity output, diff the results.
# Validates that module output is deterministic across runs.
#
# Usage: scripts/aa_gate.sh MODULE REGION BAM REF
#   MODULE  - module name matching env var suffix (e.g. CIGAR_PARSER)
#   REGION  - genomic region in chr:start-end format
#   BAM     - path to BAM file
#   REF     - path to reference FASTA

if [[ $# -ne 4 ]]; then
    echo "Usage: $0 MODULE REGION BAM REF" >&2
    echo "  MODULE  - e.g. CIGAR_PARSER" >&2
    echo "  REGION  - e.g. chr1:100-200" >&2
    echo "  BAM     - path to BAM file" >&2
    echo "  REF     - path to reference FASTA" >&2
    exit 1
fi

MODULE="$1"
REGION="$2"
BAM="$3"
REF="$4"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VARDICT_DIR="$PROJECT_ROOT/VarDictJava"

# Output dirs under ./tmp (per ops policy)
RUN_A="$PROJECT_ROOT/tmp/aa_gate/${MODULE}_a"
RUN_B="$PROJECT_ROOT/tmp/aa_gate/${MODULE}_b"

# Clean previous runs
rm -rf "$RUN_A" "$RUN_B"
mkdir -p "$RUN_A" "$RUN_B"

echo "=== A-A Gate: $MODULE ==="
echo "Region: $REGION"
echo "BAM:    $BAM"
echo "REF:    $REF"
echo ""

# Build VarDictJava if needed
echo "--- Building VarDictJava ---"
(cd "$VARDICT_DIR" && ./gradlew installDist -q)

VARDICT_BIN="$VARDICT_DIR/build/install/VarDict/bin/VarDict"

if [[ ! -x "$VARDICT_BIN" ]]; then
    echo "ERROR: VarDictJava binary not found at $VARDICT_BIN" >&2
    exit 1
fi

# Force single-threaded for determinism
COMMON_ARGS="-G $REF -b $BAM -th 1 -R $REGION"

echo "--- Run A ---"
export "VARDICT_PARITY_${MODULE}=$RUN_A"
$VARDICT_BIN $COMMON_ARGS > /dev/null 2>&1
unset "VARDICT_PARITY_${MODULE}"

echo "--- Run B ---"
export "VARDICT_PARITY_${MODULE}=$RUN_B"
$VARDICT_BIN $COMMON_ARGS > /dev/null 2>&1
unset "VARDICT_PARITY_${MODULE}"

# Diff the outputs
echo "--- Comparing outputs ---"
if diff -r "$RUN_A" "$RUN_B" > /dev/null 2>&1; then
    echo "PASS: A-A gate passed - outputs are identical."
    exit 0
else
    echo "FAIL: A-A gate failed - outputs differ:" >&2
    diff -r "$RUN_A" "$RUN_B" >&2 || true
    exit 1
fi