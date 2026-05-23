#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_SAMTOOLS="/home/eck/software/miniconda3/envs/vdr/bin/samtools"

if [[ -z "${SAMTOOLS:-}" ]]; then
    export SAMTOOLS="$DEFAULT_SAMTOOLS"
fi

exec python3 "$SCRIPT_DIR/sample_regions.py" "$@"