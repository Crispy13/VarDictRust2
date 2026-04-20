#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/gen_e2e_golden_tsv.sh [--push-only] [--force] [--config <name> | --all-configs] [--tier <n>]

  --push-only    Generate only the 10-region push subset
  --force        Regenerate files even if they already exist
  --config NAME  Generate fixtures for a single config preset
  --all-configs  Generate fixtures for every preset from scripts/config_presets.tsv
    --tier N       Generate fixtures for configs in the specified tier only (1-4)
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGION_TSV="$PROJECT_ROOT/testdata/parity_regions.tsv"
PRESET_TSV="$PROJECT_ROOT/scripts/config_presets.tsv"
VARDICT_DIR="$PROJECT_ROOT/VarDictJava"
VARDICT_BIN="$VARDICT_DIR/build/install/VarDict/bin/VarDict"

push_only=false
force=false
selected_config=""
selected_tier=""
all_configs=false

list_preset_names() {
    if [[ -n "${selected_tier:-}" ]]; then
        awk -F '\t' -v tier="$selected_tier" 'NF && $0 !~ /^[[:space:]]*#/ && $4 == tier { print $1 }' "$PRESET_TSV"
    else
        awk -F '\t' 'NF && $0 !~ /^[[:space:]]*#/ { print $1 }' "$PRESET_TSV"
    fi
}

available_presets() {
    awk -F '\t' '
        NF && $0 !~ /^[[:space:]]*#/ {
            names[++count] = $1
        }
        END {
            for (i = 1; i <= count; i++) {
                printf "%s%s", names[i], (i < count ? ", " : "")
            }
        }
    ' "$PRESET_TSV"
}

lookup_preset_flags() {
    local preset_name="$1"

    awk -F '\t' -v preset_name="$preset_name" '
        NF && $0 !~ /^[[:space:]]*#/ && $1 == preset_name {
            print $2
            found = 1
            exit
        }
        END {
            if (!found) {
                exit 1
            }
        }
    ' "$PRESET_TSV"
}

build_region_indices() {
    if [[ "$push_only" == true ]]; then
        indices=(0 1 2 3 4 35 36 37 70 71)
    else
        indices=()
        for ((i = 0; i < ${#region_rows[@]}; i++)); do
            indices+=("$i")
        done
    fi
}

generate_for_config() {
    local config_name="$1"
    local extra_flags="$2"
    local output_dir="$BASE_OUTPUT_DIR"
    local progress_prefix=""

    if [[ -n "$config_name" ]]; then
        output_dir="$BASE_OUTPUT_DIR/$config_name"
        progress_prefix="[$config_name] "
    fi

    mkdir -p "$output_dir"

    local total=${#indices[@]}
    local position index region bam_path ref_path safe_region out_file
    for ((position = 0; position < total; position++)); do
        index=${indices[$position]}
        IFS=$'\t' read -r region bam_path ref_path <<< "${region_rows[$index]}"
        safe_region=${region//:/_}
        safe_region=${safe_region//-/_}
        out_file="$output_dir/${safe_region}.tsv"

        printf '%s[%d/%d] %s -> %s\n' "$progress_prefix" "$((position + 1))" "$total" "$region" "$out_file"

        if [[ -f "$out_file" && "$force" != true ]]; then
            continue
        fi

        # Intentional word-splitting: preset flags come from our repo-owned TSV.
        "$VARDICT_BIN" \
            -G "$ref_path" \
            -b "$bam_path" \
            -N test_sample \
            -th 1 \
            $extra_flags \
            -R "$region" \
            > "$out_file"
    done
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --push-only)
            push_only=true
            ;;
        --force)
            force=true
            ;;
        --config)
            shift
            selected_config="${1:-}"
            if [[ -z "$selected_config" ]]; then
                echo "ERROR: --config requires a preset name" >&2
                exit 1
            fi
            ;;
        --all-configs)
            all_configs=true
            ;;
        --tier)
            shift
            selected_tier="${1:-}"
            if [[ -z "$selected_tier" ]]; then
                echo "ERROR: --tier requires a tier number" >&2
                exit 1
            fi
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

if [[ ! -f "$PRESET_TSV" ]]; then
    echo "ERROR: Preset TSV not found at $PRESET_TSV" >&2
    exit 1
fi

if [[ "$all_configs" == true && -n "$selected_config" ]]; then
    echo "ERROR: --config and --all-configs are mutually exclusive" >&2
    exit 1
fi

if [[ -n "$selected_tier" && ! "$selected_tier" =~ ^[1-4]$ ]]; then
    echo "ERROR: --tier must be one of: 1, 2, 3, 4" >&2
    exit 1
fi

if [[ -n "$selected_tier" && "$all_configs" != true ]]; then
    echo "ERROR: --tier requires --all-configs" >&2
    exit 1
fi

if [[ ! -x "$VARDICT_BIN" ]]; then
    (cd "$VARDICT_DIR" && ./gradlew installDist -q)
fi

if [[ ! -x "$VARDICT_BIN" ]]; then
    echo "ERROR: VarDictJava launcher not found at $VARDICT_BIN" >&2
    exit 1
fi

BASE_OUTPUT_DIR="${VARDICT_E2E_FIXTURE_DIR:-tmp/e2e_fixtures}"
mkdir -p "$BASE_OUTPUT_DIR"

mapfile -t region_rows < <(awk 'NF && $0 !~ /^[[:space:]]*#/' "$REGION_TSV")
if [[ ${#region_rows[@]} -eq 0 ]]; then
    echo "ERROR: No regions found in $REGION_TSV" >&2
    exit 1
fi

build_region_indices

if [[ "$all_configs" == true ]]; then
    mapfile -t config_names < <(list_preset_names)
    if [[ ${#config_names[@]} -eq 0 ]]; then
        if [[ -n "$selected_tier" ]]; then
            echo "ERROR: No config presets found in $PRESET_TSV for tier $selected_tier" >&2
        else
            echo "ERROR: No config presets found in $PRESET_TSV" >&2
        fi
        exit 1
    fi

    for config_name in "${config_names[@]}"; do
        extra_flags=$(lookup_preset_flags "$config_name") || {
            echo "ERROR: Unknown config preset '$config_name' from $PRESET_TSV" >&2
            exit 1
        }
        generate_for_config "$config_name" "$extra_flags"
    done
elif [[ -n "$selected_config" ]]; then
    extra_flags=$(lookup_preset_flags "$selected_config") || {
        echo "ERROR: Unknown config preset '$selected_config'. Available presets: $(available_presets)" >&2
        exit 1
    }
    generate_for_config "$selected_config" "$extra_flags"
else
    generate_for_config "" ""
fi
