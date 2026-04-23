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
Usage: scripts/gen_e2e_sweep_golden.sh [--config <name>] [--tags <csv>] [--somatic] [--force] [--output-only] [--dry-run]

Regenerate Java e2e sweep cache shards through scripts/sweep_fixtures_parallel.py.
Wall estimates are order-of-magnitude only and assume about 250 tiles/sec at 10 workers.

Options:
  --config <name>   Preset label forwarded to Python. Use "default" for the legacy layout.
  --tags <csv>      Comma-separated subset of hg002,na12878_exome,na12878_lowcov.
                    Default: hg002,na12878_exome,na12878_lowcov
                                        With --somatic, this is treated as pair tags.
    --somatic         Regenerate tumor/normal pair output-only shards.
  --force           Skip confirmation. Required when stdin has no response available.
  --output-only     Always enabled; exposed here for discoverability.
    --sweep-bed-root <path>
                                        Override the sweep BED root. Default: tmp/sweep_beds
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

    CONFIG_NAME="$config" \
    TAGS_CSV="$tags_csv" \
    LOGICAL_FLAGS="$logical_flags" \
    PROJECT_ROOT="$project_root" \
    MANIFEST_PATH="$manifest_path" \
    PRESERVE_PATH="$preserve_path" \
    SWEEP_BED_ROOT="$sweep_bed_root" \
    python3 <<'PY'
import glob
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

project_root = Path(os.environ["PROJECT_ROOT"])
manifest_path = Path(os.environ["MANIFEST_PATH"])
preserve_path = Path(os.environ["PRESERVE_PATH"])
config_name = os.environ["CONFIG_NAME"]
logical_flags = " ".join(os.environ["LOGICAL_FLAGS"].split())
sweep_bed_root = Path(os.environ["SWEEP_BED_ROOT"])
tags = [tag for tag in os.environ["TAGS_CSV"].split(",") if tag]

bam_paths = {
    "hg002": "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    "na12878_exome": "testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam",
    "na12878_lowcov": "testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam",
}

def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def sha256_concat(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in paths:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()

if not manifest_path.exists():
    raise SystemExit(f"ERROR: manifest not found after generator run: {manifest_path}")

with manifest_path.open("r", encoding="utf-8") as handle:
    manifest = json.load(handle)

preserved_entries = {}
if preserve_path.exists():
    with preserve_path.open("r", encoding="utf-8") as handle:
        preserved_entries = json.load(handle)

reference_sha256 = sha256_file(project_root / "testdata/hs37d5.fa.fai")
generator_flags_hash = hashlib.sha256(logical_flags.encode("utf-8")).hexdigest()
vardict_commit = manifest.get("vardictjava_commit")
if not vardict_commit:
    vardict_commit = subprocess.run(
        ["git", "-C", str(project_root / "VarDictJava"), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()

cache_entries = dict(preserved_entries)
for tag in tags:
    bed_paths = sorted(
        Path(path) for path in glob.glob(str(project_root / sweep_bed_root / tag / "*.bed"))
    )
    if not bed_paths:
        raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
    bam_path = project_root / bam_paths[tag]
    bam_stat = [{
        "path": bam_paths[tag],
        "size": bam_path.stat().st_size,
        "mtime_unix": int(bam_path.stat().st_mtime),
    }]
    key = f"{config_name}:{tag}"
    cache_entries[key] = {
        "config": config_name,
        "tag": tag,
        "bed_sha256": sha256_concat(bed_paths),
        "bam_stat": bam_stat,
        "reference_sha256": reference_sha256,
        "generator_flags_hash": generator_flags_hash,
        "vardictjava_commit": vardict_commit,
    }

manifest["cache_entries"] = cache_entries

manifest_path.parent.mkdir(parents=True, exist_ok=True)
with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=manifest_path.parent, delete=False) as handle:
    json.dump(manifest, handle, indent=2, sort_keys=False)
    handle.write("\n")
    temp_name = handle.name

os.replace(temp_name, manifest_path)
if preserve_path.exists():
    preserve_path.unlink()
PY
}

merge_manifest_somatic_cache_entries() {
    local config="$1"
    local tags_csv="$2"
    local logical_flags="$3"
    local project_root="$4"
    local sweep_bed_root="$5"
    local manifest_path="$project_root/$OUTPUT_ROOT/manifest.json"
    local preserve_path="$project_root/$OUTPUT_ROOT/.manifest.cache_entries.before.json"

    CONFIG_NAME="$config" \
    TAGS_CSV="$tags_csv" \
    LOGICAL_FLAGS="$logical_flags" \
    PROJECT_ROOT="$project_root" \
    MANIFEST_PATH="$manifest_path" \
    PRESERVE_PATH="$preserve_path" \
    SWEEP_BED_ROOT="$sweep_bed_root" \
    python3 <<'PY'
import glob
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

project_root = Path(os.environ["PROJECT_ROOT"])
manifest_path = Path(os.environ["MANIFEST_PATH"])
preserve_path = Path(os.environ["PRESERVE_PATH"])
config_name = os.environ["CONFIG_NAME"]
logical_flags = " ".join(os.environ["LOGICAL_FLAGS"].split())
sweep_bed_root = Path(os.environ["SWEEP_BED_ROOT"])
tags = [tag for tag in os.environ["TAGS_CSV"].split(",") if tag]

pair_paths = {
    "wes_il_pair": (
        "testdata/WES_IL_T_1.bwa.dedup.bam",
        "testdata/WES_IL_N_1.bwa.dedup.bam",
    ),
}

def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def sha256_concat(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in paths:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()

if not manifest_path.exists():
    raise SystemExit(f"ERROR: manifest not found after generator run: {manifest_path}")

with manifest_path.open("r", encoding="utf-8") as handle:
    manifest = json.load(handle)

preserved_entries = {}
if preserve_path.exists():
    with preserve_path.open("r", encoding="utf-8") as handle:
        preserved_entries = json.load(handle)

reference_fai = project_root / "testdata/GRCh38.d1.vd1.fa.fai"
reference_target = reference_fai if reference_fai.exists() else project_root / "testdata/GRCh38.d1.vd1.fa"
reference_sha256 = sha256_file(reference_target)
generator_flags_hash = hashlib.sha256(logical_flags.encode("utf-8")).hexdigest()
vardict_commit = manifest.get("vardictjava_commit")
if not vardict_commit:
    vardict_commit = subprocess.run(
        ["git", "-C", str(project_root / "VarDictJava"), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()

cache_entries = dict(preserved_entries)
for tag in tags:
    bed_paths = sorted(
        Path(path) for path in glob.glob(str(project_root / sweep_bed_root / tag / "*.bed"))
    )
    if not bed_paths:
        raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
    tumor_rel, normal_rel = pair_paths[tag]
    tumor_path = project_root / tumor_rel
    normal_path = project_root / normal_rel
    bam_stat = [
        {
            "path": tumor_rel,
            "size": tumor_path.stat().st_size,
            "mtime_unix": int(tumor_path.stat().st_mtime),
            "role": "tumor",
        },
        {
            "path": normal_rel,
            "size": normal_path.stat().st_size,
            "mtime_unix": int(normal_path.stat().st_mtime),
            "role": "normal",
        },
    ]
    key = f"{config_name}:somatic:{tag}"
    cache_entries[key] = {
        "config": config_name,
        "tag": tag,
        "bed_sha256": sha256_concat(bed_paths),
        "bam_stat": bam_stat,
        "reference_sha256": reference_sha256,
        "generator_flags_hash": generator_flags_hash,
        "vardictjava_commit": vardict_commit,
    }

manifest["cache_entries"] = cache_entries

manifest_path.parent.mkdir(parents=True, exist_ok=True)
with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=manifest_path.parent, delete=False) as handle:
    json.dump(manifest, handle, indent=2, sort_keys=False)
    handle.write("\n")
    temp_name = handle.name

os.replace(temp_name, manifest_path)
if preserve_path.exists():
    preserve_path.unlink()
PY
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

config="$DEFAULT_CONFIG"
tags_csv="$DEFAULT_TAGS"
force=0
dry_run=0
somatic=0
tags_provided=0
sweep_bed_root="$DEFAULT_SWEEP_BED_ROOT"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --config)
            shift
            [[ $# -gt 0 ]] || die "--config requires a value"
            config="$1"
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

save_existing_cache_entries "$project_root"

set +e
"${actual_cmd[@]}" 2>&1 | tee "$log_file"
cmd_status=${PIPESTATUS[0]}
set -e

if [[ $cmd_status -ne 0 ]]; then
    echo "::error::e2e sweep cache regeneration failed; logfile=$log_file" >&2
    exit "$cmd_status"
fi

if [[ $somatic -eq 1 ]]; then
    merge_manifest_somatic_cache_entries "$config" "$tags_csv" "$logical_flags" "$project_root" "$sweep_bed_root"
else
    merge_manifest_cache_entries "$config" "$tags_csv" "$logical_flags" "$project_root" "$sweep_bed_root"
fi
