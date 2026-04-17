#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/gen_e2e_golden_tsv.sh [--push-only] [--force]

  --push-only  Generate only the 10-region push subset
  --force      Regenerate files even if they already exist
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGION_TSV="$PROJECT_ROOT/testdata/parity_regions.tsv"
VARDICT_DIR="$PROJECT_ROOT/VarDictJava"
VARDICT_BIN="$VARDICT_DIR/build/install/VarDict/bin/VarDict"

push_only=false
force=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --push-only)
            push_only=true
            ;;
        --force)
            force=true
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
    shift
done

cd "$PROJECT_ROOT"

if [[ ! -x "$VARDICT_BIN" ]]; then
    (cd "$VARDICT_DIR" && ./gradlew installDist -q)
fi

if [[ ! -x "$VARDICT_BIN" ]]; then
    echo "ERROR: VarDictJava launcher not found at $VARDICT_BIN" >&2
    exit 1
fi

OUTPUT_DIR="${VARDICT_E2E_FIXTURE_DIR:-tmp/e2e_fixtures}"
mkdir -p "$OUTPUT_DIR"

mapfile -t region_rows < <(awk 'NF' "$REGION_TSV")
if [[ ${#region_rows[@]} -eq 0 ]]; then
    echo "ERROR: No regions found in $REGION_TSV" >&2
    exit 1
fi

if [[ "$push_only" == true ]]; then
    indices=(0 1 2 3 4 35 36 37 70 71)
else
    indices=()
    for ((i = 0; i < ${#region_rows[@]}; i++)); do
        indices+=("$i")
    done
fi

total=${#indices[@]}
for ((position = 0; position < total; position++)); do
    index=${indices[$position]}
    IFS=$'\t' read -r region bam_path ref_path <<< "${region_rows[$index]}"
    safe_region=${region//:/_}
    safe_region=${safe_region//-/_}
    out_file="$OUTPUT_DIR/${safe_region}.tsv"

    printf '[%d/%d] %s -> %s\n' "$((position + 1))" "$total" "$region" "$out_file"

    if [[ -f "$out_file" && "$force" != true ]]; then
        continue
    fi

    "$VARDICT_BIN" \
        -G "$ref_path" \
        -b "$bam_path" \
        -N test_sample \
        -th 1 \
        -R "$region" \
        > "$out_file"
done