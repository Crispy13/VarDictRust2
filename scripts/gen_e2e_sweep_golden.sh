#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

# Phase 2 wrapper for e2e sweep cache regeneration.
# References: Phase 0b commit 04b0816 and /memories/session/subplan-e2e-sweep-phase2.md.
# Context confirmed before implementation:
# - sweep_fixtures_parallel.py already exposes --output-only, --config, --sweep-bed-root, and --tags.
# - Python treats omitted --config as the legacy default layout, so wrapper-level "default"
#   is translated to an omitted child --config flag.
# - Unknown configs are validated by the Python generator, not duplicated here.
# - Python rewrites tmp/sweep_fixtures/manifest.json with run metadata only, so this wrapper
#   merges per-(config, tag) cache fingerprint records back in after a successful run.

readonly DEFAULT_CONFIG="default"
readonly DEFAULT_TAGS="hg002,na12878_exome,na12878_lowcov"
readonly DEFAULT_SOMATIC_TAGS="wes_il_pair"
readonly DEFAULT_SWEEP_BED_ROOT="tmp/sweep_beds"
readonly OUTPUT_ROOT="tmp/sweep_fixtures"
readonly LOG_ROOT="tmp/sweep_fixtures/logs"
readonly TILE_RATE=250

declare -Ar TILE_COUNTS=(
    [hg002]=4991705
    [na12878_exome]=144853
    [na12878_lowcov]=4899382
    [wes_il_pair]=4880000
)

usage() {
    cat <<'EOF'
Usage: scripts/gen_e2e_sweep_golden.sh [--config <name> | --all-configs] [--config-tier <N|N-M>] [--tags <csv>] [--somatic] [--force] [--output-only] [--dry-run] [--parallel <N>] [--inverted] [--shm-root <path>] [--no-shm]

Regenerate Java e2e sweep cache shards through scripts/sweep_fixtures_parallel.py.
Wall estimates are order-of-magnitude only and assume about 250 tiles/sec at 10 workers.

Options:
  --config <name>       Preset label forwarded to Python. Use "default" for the legacy layout.
  --all-configs         Iterate over all presets in scripts/config_presets.tsv whose
                        applies_to matches the current lane (germline or somatic).
                        Mutually exclusive with --config. Respects --config-tier filter.
  --config-tier <N|N-M> Filter --all-configs to a tier or tier range (e.g., 1, 2, 1-3).
  --parallel <N>        With --all-configs, run up to N preset generations concurrently.
                        Default: 1 (sequential). Manifest merges are serialized via flock;
                        each invocation writes to a per-PID manifest staging file.
    --inverted            With --all-configs, run a single Python invocation with
                                                --presets ALL instead of spawning one child per preset.
                                                --parallel becomes preset-level worker count in Python.
  --tags <csv>      Comma-separated subset of hg002,na12878_exome,na12878_lowcov.
                    Default: hg002,na12878_exome,na12878_lowcov
                                        With --somatic, this is treated as pair tags.
    --somatic         Regenerate tumor/normal pair output-only shards.
  --force           Skip confirmation. Required when stdin has no response available.
  --output-only     Always enabled; exposed here for discoverability.
    --sweep-bed-root <path>
                                        Override the sweep BED root. Default: tmp/sweep_beds
    --shm-root <path>  Forward a staging root to the Python generator.
    --no-shm           Disable per-chrom shm staging in the Python generator.
  --dry-run         Print the logical command and exit without invoking Python.
  -h, --help        Show this help.

Approximate tile counts:
  hg002           4991705
  na12878_exome    144853
  na12878_lowcov  4899382

Notes:
  - The wrapper keeps cache fingerprints in tmp/sweep_fixtures/manifest.json.
  - Unknown configs are validated by scripts/sweep_fixtures_parallel.py.
  - Wrapper-level "default" maps to Python's legacy no-flag default layout.
  - CM-PILEUP canonical TSV fixtures are sorted during generation and old unsorted
    CM-PILEUP caches must be regenerated before the parity_e2e_sweep harness can use them.
    - --somatic writes cache_entries under {config}:somatic:{tag}.
EOF
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

join_by_comma() {
    local first=1
    local item
    for item in "$@"; do
        if [[ $first -eq 1 ]]; then
            printf '%s' "$item"
            first=0
        else
            printf ',%s' "$item"
        fi
    done
}

normalize_tags() {
    local raw_tags="$1"
    local sweep_bed_root="$2"
    local -n out_ref=$3
    local -A seen=()
    local part trimmed
    out_ref=()
    IFS=',' read -r -a parts <<< "$raw_tags"
    for part in "${parts[@]}"; do
        trimmed="${part//[[:space:]]/}"
        [[ -n "$trimmed" ]] || continue
        [[ -n "${TILE_COUNTS[$trimmed]+x}" ]] || die "unknown BAM tag: $trimmed"
        [[ -d "$sweep_bed_root/$trimmed" ]] || die "sweep BED directory not found for $trimmed: $sweep_bed_root/$trimmed"
        if [[ -z "${seen[$trimmed]+x}" ]]; then
            seen[$trimmed]=1
            out_ref+=("$trimmed")
        fi
    done
    [[ ${#out_ref[@]} -gt 0 ]] || die "no BAM tags selected"
}

estimate_runtime() {
    local -n tags_ref=$1
    local total_tiles=0
    local tag
    for tag in "${tags_ref[@]}"; do
        total_tiles=$((total_tiles + TILE_COUNTS[$tag]))
    done

    local total_seconds=$(( (total_tiles + TILE_RATE - 1) / TILE_RATE ))
    local days=$(( total_seconds / 86400 ))
    local remainder=$(( total_seconds % 86400 ))
    local hours=$(( remainder / 3600 ))
    local minutes=$(( (remainder % 3600 + 59) / 60 ))

    if [[ $minutes -eq 60 ]]; then
        minutes=0
        hours=$((hours + 1))
    fi
    if [[ $hours -eq 24 ]]; then
        hours=0
        days=$((days + 1))
    fi

    if [[ $days -gt 0 ]]; then
        printf '~%dd %dh %dm' "$days" "$hours" "$minutes"
    else
        printf '~%dh %dm' "$hours" "$minutes"
    fi
}

logical_flags_string() {
    local config="$1"
    local tags_csv="$2"
    local somatic="$3"
    local sweep_bed_root="$4"

    if [[ $somatic -eq 1 ]]; then
        printf '%s' "--output-only --config $config --pair-tags $tags_csv --tags  --sweep-bed-root $sweep_bed_root"
    else
        printf '%s' "--output-only --config $config --tags $tags_csv --sweep-bed-root $sweep_bed_root"
    fi
}

read_confirmation() {
    local reply
    printf 'Proceed? [y/N] ' >&2
    if ! read -r reply; then
        echo >&2
        echo "ERROR: non-interactive requires --force" >&2
        exit 2
    fi
    echo >&2

    case "$reply" in
        y|Y) return 0 ;;
        *) exit 0 ;;
    esac
}

merge_manifest_cache_entries() {
    local config="$1"
    local tags_csv="$2"
    local logical_flags="$3"
    local project_root="$4"
    local sweep_bed_root="$5"
    local manifest_path="$project_root/$OUTPUT_ROOT/manifest.json"
    local preserve_path="$project_root/$OUTPUT_ROOT/.manifest.cache_entries.before.json"

    python3 -m scripts.lib.merge_manifest cache-entries \
        --config "$config" \
        --tags "$tags_csv" \
        --logical-flags "$logical_flags" \
        --project-root "$project_root" \
        --sweep-bed-root "$sweep_bed_root" \
        --manifest-path "$manifest_path" \
        --preserve-path "$preserve_path"
}

merge_manifest_somatic_cache_entries() {
    local config="$1"
    local tags_csv="$2"
    local logical_flags="$3"
    local project_root="$4"
    local sweep_bed_root="$5"
    local manifest_path="$project_root/$OUTPUT_ROOT/manifest.json"
    local preserve_path="$project_root/$OUTPUT_ROOT/.manifest.cache_entries.before.json"

    python3 -m scripts.lib.merge_manifest cache-entries-somatic \
        --config "$config" \
        --tags "$tags_csv" \
        --logical-flags "$logical_flags" \
        --project-root "$project_root" \
        --sweep-bed-root "$sweep_bed_root" \
        --manifest-path "$manifest_path" \
        --preserve-path "$preserve_path"
}

merge_manifest_cache_entries_many() {
    local config_names_csv="$1"
    local tags_csv="$2"
    local project_root="$3"
    local sweep_bed_root="$4"
    local manifest_path="$project_root/$OUTPUT_ROOT/manifest.json"
    local preserve_path="$project_root/$OUTPUT_ROOT/.manifest.cache_entries.before.json"

    python3 -m scripts.lib.merge_manifest cache-entries-many \
        --configs "$config_names_csv" \
        --tags "$tags_csv" \
        --project-root "$project_root" \
        --sweep-bed-root "$sweep_bed_root" \
        --manifest-path "$manifest_path" \
        --preserve-path "$preserve_path"
}

merge_manifest_somatic_cache_entries_many() {
    local config_names_csv="$1"
    local tags_csv="$2"
    local project_root="$3"
    local sweep_bed_root="$4"
    local manifest_path="$project_root/$OUTPUT_ROOT/manifest.json"
    local preserve_path="$project_root/$OUTPUT_ROOT/.manifest.cache_entries.before.json"

    python3 -m scripts.lib.merge_manifest cache-entries-somatic-many \
        --configs "$config_names_csv" \
        --tags "$tags_csv" \
        --project-root "$project_root" \
        --sweep-bed-root "$sweep_bed_root" \
        --manifest-path "$manifest_path" \
        --preserve-path "$preserve_path"
}

save_existing_cache_entries() {
    local project_root="$1"
    local manifest_path="$project_root/$OUTPUT_ROOT/manifest.json"
    local preserve_path="$project_root/$OUTPUT_ROOT/.manifest.cache_entries.before.json"

    mkdir -p "$project_root/$OUTPUT_ROOT"

    if [[ ! -f "$manifest_path" ]]; then
        rm -f "$preserve_path"
        return 0
    fi

    MANIFEST_PATH="$manifest_path" PRESERVE_PATH="$preserve_path" python3 <<'PY'
import json
import os
from pathlib import Path

manifest_path = Path(os.environ["MANIFEST_PATH"])
preserve_path = Path(os.environ["PRESERVE_PATH"])

with manifest_path.open("r", encoding="utf-8") as handle:
    manifest = json.load(handle)

with preserve_path.open("w", encoding="utf-8") as handle:
    json.dump(manifest.get("cache_entries", {}), handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
}

# Parallel-mode additive merge: read the real manifest.json (if present) to
# preserve accumulated cache_entries from peer invocations, compute this
# invocation's new cache_entry, and write the merged manifest. Caller MUST
# hold the flock on $OUTPUT_ROOT/.manifest.lock for the duration of this call.
merge_staging_manifest_into_real() {
    local config="$1"
    local tags_csv="$2"
    local logical_flags="$3"
    local project_root="$4"
    local sweep_bed_root="$5"
    local staging_manifest="$6"
    local somatic="$7"
    local real_manifest="$project_root/$OUTPUT_ROOT/manifest.json"

    CONFIG_NAME="$config" \
    TAGS_CSV="$tags_csv" \
    LOGICAL_FLAGS="$logical_flags" \
    PROJECT_ROOT="$project_root" \
    REAL_MANIFEST="$real_manifest" \
    STAGING_MANIFEST="$staging_manifest" \
    SWEEP_BED_ROOT="$sweep_bed_root" \
    SOMATIC="$somatic" \
    python3 <<'PY'
import glob
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

project_root = Path(os.environ["PROJECT_ROOT"])
real_manifest_path = Path(os.environ["REAL_MANIFEST"])
staging_manifest_path = Path(os.environ["STAGING_MANIFEST"])
config_name = os.environ["CONFIG_NAME"]
logical_flags = " ".join(os.environ["LOGICAL_FLAGS"].split())
sweep_bed_root = Path(os.environ["SWEEP_BED_ROOT"])
tags = [tag for tag in os.environ["TAGS_CSV"].split(",") if tag]
somatic = os.environ["SOMATIC"] == "1"

bam_paths = {
    "hg002": "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    "na12878_exome": "testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam",
    "na12878_lowcov": "testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam",
}
somatic_pair_paths = {
    "wes_il_pair": {
        "tumor": "testdata/WES_IL_T_1.bwa.dedup.bam",
        "normal": "testdata/WES_IL_N_1.bwa.dedup.bam",
    },
}
somatic_ref = {
) as handle:
    json.dump(merged, handle, indent=2, sort_keys=False)
    handle.write("\n")
    temp_name = handle.name
os.replace(temp_name, real_manifest_path)
PY
}

config="$DEFAULT_CONFIG"
tags_csv="$DEFAULT_TAGS"
force=0
dry_run=0
somatic=0
tags_provided=0
sweep_bed_root="$DEFAULT_SWEEP_BED_ROOT"
all_configs=0
config_tier_filter=""
parallel_jobs=1
inverted=0
shm_root=""
no_shm=0
# Internal flag: when set, this invocation was spawned by --all-configs --parallel,
# and must use a per-PID manifest staging path with flock-serialized merge.
internal_parallel_mode=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --config)
            shift
            [[ $# -gt 0 ]] || die "--config requires a value"
            config="$1"
            ;;
        --all-configs)
            all_configs=1
            ;;
        --config-tier)
            shift
            [[ $# -gt 0 ]] || die "--config-tier requires a value (e.g., 1 or 1-2)"
            config_tier_filter="$1"
            ;;
        --parallel)
            shift
            [[ $# -gt 0 ]] || die "--parallel requires a positive integer"
            [[ "$1" =~ ^[1-9][0-9]*$ ]] || die "--parallel must be a positive integer, got: $1"
            parallel_jobs="$1"
            ;;
        --inverted)
            inverted=1
            ;;
        --_internal-parallel-mode)
            internal_parallel_mode=1
            ;;
        --tags)
            shift
            [[ $# -gt 0 ]] || die "--tags requires a value"
            tags_csv="$1"
            tags_provided=1
            ;;
        --somatic)
            somatic=1
            ;;
        --force)
            force=1
            ;;
        --output-only)
            ;;
        --sweep-bed-root)
            shift
            [[ $# -gt 0 ]] || die "--sweep-bed-root requires a value"
            sweep_bed_root="$1"
            ;;
        --shm-root)
            shift
            [[ $# -gt 0 ]] || die "--shm-root requires a value"
            shm_root="$1"
            ;;
        --no-shm)
            no_shm=1
            ;;
        --dry-run)
            dry_run=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die "unknown option: $1"
            ;;
    esac
    shift
done

if [[ $inverted -eq 1 && $all_configs -ne 1 ]]; then
    die "--inverted requires --all-configs"
fi

# --all-configs: loop over all TSV presets applicable to the current lane.
# Invokes this script recursively with --config <name> --force for each.
# Respects --dry-run, --tags, --sweep-bed-root, --somatic, --config-tier.
if [[ $all_configs -eq 1 ]]; then
    project_root="$(git rev-parse --show-toplevel)"
    preset_tsv="$project_root/scripts/config_presets.tsv"
    [[ -f "$preset_tsv" ]] || die "preset TSV not found: $preset_tsv"

    # applies_to filter: somatic lane takes rows with 'somatic' or 'both';
    # germline takes 'germline' or 'both'.
    if [[ $somatic -eq 1 ]]; then
        applies_filter='somatic|both'
    else
        applies_filter='germline|both'
    fi

    # Build list of (name, tier) from TSV, filtered by applies_to and optional tier.
    mapfile -t candidate_names < <(awk -F'\t' -v af="$applies_filter" -v tf="$config_tier_filter" '
        NR>1 && !/^[[:space:]]*$/ && $1 !~ /^#/ {
            applies = (NF >= 5 ? $5 : "both")
            tier = $4
            if (applies ~ "^("af")$") {
                if (tf == "" || tier == tf || tf ~ ("(^|-)"tier"(-|$)")) {
                    print $1
                }
            }
        }
    ' "$preset_tsv")

    [[ ${#candidate_names[@]} -gt 0 ]] || die "--all-configs: no presets matched (applies_to=$applies_filter, tier=$config_tier_filter)"

    echo "# --all-configs will invoke this script for ${#candidate_names[@]} preset(s):"
    for name in "${candidate_names[@]}"; do
        echo "#   $name"
    done

    # Forward flags (excluding --all-configs / --config / --config-tier / --parallel)
    forward_flags=()
    [[ -n "$tags_csv" && $tags_provided -eq 1 ]] && forward_flags+=(--tags "$tags_csv")
    [[ $somatic -eq 1 ]] && forward_flags+=(--somatic)
    [[ -n "$sweep_bed_root" && "$sweep_bed_root" != "$DEFAULT_SWEEP_BED_ROOT" ]] && forward_flags+=(--sweep-bed-root "$sweep_bed_root")
    [[ -n "$shm_root" ]] && forward_flags+=(--shm-root "$shm_root")
    [[ $no_shm -eq 1 ]] && forward_flags+=(--no-shm)
    [[ $dry_run -eq 1 ]] && forward_flags+=(--dry-run)

    script_path="${BASH_SOURCE[0]}"

    if [[ $inverted -eq 1 ]]; then
        project_root_abs="$(git rev-parse --show-toplevel)"
        presets_csv="$(join_by_comma "${candidate_names[@]}")"
        actual_cmd=(python3 scripts/sweep_fixtures_parallel.py --output-only --presets "$presets_csv" --workers "$parallel_jobs")
        if [[ $somatic -eq 1 ]]; then
            actual_cmd+=(--pair-tags "$tags_csv" --tags "")
        else
            actual_cmd+=(--tags "$tags_csv")
        fi
        if [[ -n "$sweep_bed_root" ]]; then
            actual_cmd+=(--sweep-bed-root "$sweep_bed_root")
        fi
        if [[ -n "$shm_root" ]]; then
            actual_cmd+=(--shm-root "$shm_root")
        fi
        if [[ $no_shm -eq 1 ]]; then
            actual_cmd+=(--no-shm)
        fi
        actual_cmd+=(--force)

        if [[ $dry_run -eq 1 ]]; then
            printf '%q ' "${actual_cmd[@]}"
            printf '\n'
            exit 0
        fi

        save_existing_cache_entries "$project_root_abs"
        mkdir -p "$project_root_abs/$LOG_ROOT"
        log_file="$project_root_abs/$LOG_ROOT/e2e_sweep_inverted_$(date -u +%Y%m%dT%H%M%SZ).log"

        set +e
        (
            cd "$project_root_abs"
            "${actual_cmd[@]}"
        ) 2>&1 | tee "$log_file"
        cmd_status=${PIPESTATUS[0]}
        set -e

        if [[ $cmd_status -ne 0 ]]; then
            echo "::error::e2e sweep inverted cache regeneration failed; logfile=$log_file" >&2
            exit "$cmd_status"
        fi

        if [[ $somatic -eq 1 ]]; then
            merge_manifest_somatic_cache_entries_many "$presets_csv" "$tags_csv" "$project_root_abs" "$sweep_bed_root"
        else
            merge_manifest_cache_entries_many "$presets_csv" "$tags_csv" "$project_root_abs" "$sweep_bed_root"
        fi
        exit 0
    fi

    if [[ $parallel_jobs -gt 1 ]]; then
        # Parallel mode: run up to $parallel_jobs recursive invocations concurrently.
        # Each invocation uses a per-PID manifest staging file; manifest merges are
        # serialized via flock on $OUTPUT_ROOT/.manifest.lock.
        project_root_abs="$(git rev-parse --show-toplevel)"
        mkdir -p "$project_root_abs/$OUTPUT_ROOT"
        : > "$project_root_abs/$OUTPUT_ROOT/.manifest.lock"
        echo "# Running ${#candidate_names[@]} presets with --parallel=$parallel_jobs"

        # Per-preset log dir so concurrent stdout doesn't interleave in console.
        parallel_log_root="$project_root_abs/$LOG_ROOT/parallel_$(date -u +%Y%m%dT%H%M%SZ)"
        mkdir -p "$parallel_log_root"
        echo "# Per-preset logs: $parallel_log_root/<preset>.log"

        exit_code=0
        active=0
        pids=()
        preset_by_pid=()
        for name in "${candidate_names[@]}"; do
            # Throttle: wait for a slot when at capacity.
            while [[ $active -ge $parallel_jobs ]]; do
                if wait -n 2>/dev/null; then
                    :
                else
                    # Fallback: wait for the first pid in the list.
                    wait "${pids[0]}" || exit_code=1
                    pids=("${pids[@]:1}")
                    preset_by_pid=("${preset_by_pid[@]:1}")
                fi
                active=$((active - 1))
            done
            log_file="$parallel_log_root/$name.log"
            (
                bash "$script_path" \
                    --config "$name" \
                    --force \
                    --_internal-parallel-mode \
                    "${forward_flags[@]}" \
                    >"$log_file" 2>&1
            ) &
            pid=$!
            pids+=("$pid")
            preset_by_pid+=("$name")
            active=$((active + 1))
            echo "  launched $name (pid=$pid, log=$log_file)"
        done

        # Drain remaining jobs.
        for pid in "${pids[@]}"; do
            if ! wait "$pid"; then
                exit_code=1
            fi
        done

        echo ""
        echo "=== Parallel regeneration complete (parallel=$parallel_jobs) ==="
        if [[ $exit_code -ne 0 ]]; then
            echo "::error::--all-configs: one or more presets failed; check $parallel_log_root/<preset>.log" >&2
        fi
        exit "$exit_code"
    fi

    exit_code=0
    for name in "${candidate_names[@]}"; do
        echo ""
        echo "=== Regenerating for config: $name ==="
        if ! bash "$script_path" --config "$name" --force "${forward_flags[@]}"; then
            echo "::error::--all-configs: regeneration failed for $name" >&2
            exit_code=1
            # Continue on failure to produce a full report; caller decides whether to retry.
        fi
    done
    exit "$exit_code"
fi

[[ -n "$config" ]] || die "--config must not be empty"

if [[ $somatic -eq 1 && $tags_provided -eq 0 ]]; then
    tags_csv="$DEFAULT_SOMATIC_TAGS"
fi

project_root="$(git rev-parse --show-toplevel)"
cd "$project_root"

command -v python3 >/dev/null 2>&1 || die "python3 not found in PATH"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum not found in PATH"

selected_tags=()
normalize_tags "$tags_csv" "$sweep_bed_root" selected_tags
tags_csv="$(join_by_comma "${selected_tags[@]}")"

estimate="$(estimate_runtime selected_tags)"
echo "Estimated wall time for $tags_csv: $estimate"

logical_flags="$(logical_flags_string "$config" "$tags_csv" "$somatic" "$sweep_bed_root")"
display_cmd="python3 scripts/sweep_fixtures_parallel.py $logical_flags"

if [[ $somatic -eq 1 ]]; then
    actual_cmd=(python3 scripts/sweep_fixtures_parallel.py --output-only --pair-tags "$tags_csv" --tags "" --sweep-bed-root "$sweep_bed_root")
else
    actual_cmd=(python3 scripts/sweep_fixtures_parallel.py --output-only --tags "$tags_csv" --sweep-bed-root "$sweep_bed_root")
fi
if [[ -n "$shm_root" ]]; then
    actual_cmd+=(--shm-root "$shm_root")
fi
if [[ $no_shm -eq 1 ]]; then
    actual_cmd+=(--no-shm)
fi
if [[ $force -eq 1 ]]; then
    actual_cmd+=(--force)
fi
if [[ "$config" != "$DEFAULT_CONFIG" ]]; then
    actual_cmd+=(--config "$config")
fi

if [[ $force -ne 1 ]]; then
    read_confirmation
fi

if [[ $dry_run -eq 1 ]]; then
    echo "$display_cmd"
    exit 0
fi

mkdir -p "$LOG_ROOT"
log_file="$LOG_ROOT/e2e_sweep_${config}_$(date -u +%Y%m%dT%H%M%SZ).log"

if [[ $internal_parallel_mode -eq 1 ]]; then
    # Parallel mode: python writes its manifest to a per-PID staging path,
    # and we merge into the real manifest.json under flock.
    staging_manifest="$project_root/$OUTPUT_ROOT/manifest.staging.$$.json"
    actual_cmd+=(--manifest-path "$staging_manifest")
else
    save_existing_cache_entries "$project_root"
fi

set +e
"${actual_cmd[@]}" 2>&1 | tee "$log_file"
cmd_status=${PIPESTATUS[0]}
set -e

if [[ $cmd_status -ne 0 ]]; then
    echo "::error::e2e sweep cache regeneration failed; logfile=$log_file" >&2
    [[ $internal_parallel_mode -eq 1 ]] && rm -f "$staging_manifest"
    exit "$cmd_status"
fi

if [[ $internal_parallel_mode -eq 1 ]]; then
    # Flock-serialized additive merge of staging manifest into real manifest.json.
    lock_file="$project_root/$OUTPUT_ROOT/.manifest.lock"
    (
        flock 9
        merge_staging_manifest_into_real \
            "$config" "$tags_csv" "$logical_flags" "$project_root" \
            "$sweep_bed_root" "$staging_manifest" "$somatic"
    ) 9>"$lock_file"
    rm -f "$staging_manifest"
elif [[ $somatic -eq 1 ]]; then
    merge_manifest_somatic_cache_entries "$config" "$tags_csv" "$logical_flags" "$project_root" "$sweep_bed_root"
else
    merge_manifest_cache_entries "$config" "$tags_csv" "$logical_flags" "$project_root" "$sweep_bed_root"
fi
