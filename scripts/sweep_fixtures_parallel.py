#!/usr/bin/env python3

from __future__ import annotations

import argparse
import concurrent.futures
import csv
import hashlib
import json
import math
import os
import shlex
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import TextIO

try:
    from lib.shm_slice import ShmCapacityError, StagedChrom, evict_chrom, locate_samtools, stage_chrom
except ModuleNotFoundError:
    from scripts.lib.shm_slice import (
        ShmCapacityError,
        StagedChrom,
        evict_chrom,
        locate_samtools,
        stage_chrom,
    )


ALL_BAM_TAGS = ["na12878_exome", "hg002", "na12878_lowcov"]
BAM_MAP = {
    "na12878_exome": "testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam",
    "hg002": "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    "na12878_lowcov": "testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam",
}
SOMATIC_PAIR_MAP = {
    "wes_il_pair": (
        "testdata/WES_IL_T_1.bwa.dedup.bam",
        "testdata/WES_IL_N_1.bwa.dedup.bam",
    ),
}
SOMATIC_REF_MAP = {
    "wes_il_pair": "testdata/GRCh38.d1.vd1.fa",
}
ALL_SOMATIC_TAGS = list(SOMATIC_PAIR_MAP.keys())
MODULES = (
    ("cigar_parser", "VARDICT_PARITY_CIGAR_PARSER"),
    ("realigner", "VARDICT_PARITY_REALIGNER"),
    ("sv_processor", "VARDICT_PARITY_SV_PROCESSOR"),
    ("tovars", "VARDICT_PARITY_TOVARS"),
)
OUTPUT_ROOT_REL = Path("tmp/sweep_fixtures")
RUN_LOG_REL = Path("tmp/sweep_fixtures_run.log")
SWEEP_BED_ROOT_REL = Path("tmp/sweep_beds")
CONFIG_PRESETS_REL = Path("scripts/config_presets.tsv")
REF_REL = Path("testdata/hs37d5.fa")
VARDICT_BIN_REL = Path("VarDictJava/build/install/VarDict/bin/VarDict")
DEFAULT_SHM_ROOT = Path("/dev/shm/sweep_fixtures")
FALLBACK_SHM_ROOT = Path("/tmp/sweep_fixtures_shm")


@dataclass(frozen=True)
class Shard:
    tag: str
    chrom: str
    bed_path: Path
    kind: str = "single"


@dataclass(frozen=True)
class ShardResult:
    tag: str
    chrom: str
    status: str
    config_name: str | None = None
    fixture_count: int = 0
    error: str = ""
    log_path: str = ""
    num_chunks: int = 0
    wall_s: float = 0.0
    chunk_wall_p50: float = 0.0
    chunk_wall_p99: float = 0.0
    jvm_invocations: int = 0


@dataclass(frozen=True)
class ConfigPreset:
    name: str
    flags: tuple[str, ...]


TIMINGS_HEADER = [
    "preset",
    "tag",
    "chrom",
    "num_chunks",
    "wall_s",
    "chunk_wall_p50",
    "chunk_wall_p99",
    "jvm_invocations",
    "status",
]
VARDICT_COMMIT_CACHE: dict[Path, str] = {}


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Generate sweep parity fixtures in parallel.",
    )
    parser.add_argument(
        "--workers",
        type=positive_int,
        default=10,
        help=(
            "Maximum parallel shard workers, or preset workers when --presets is used "
            "(default: 10)."
        ),
    )
    parser.add_argument(
        "--tags",
        help="Comma-separated single-sample BAM tags to process. Defaults to all single tags when omitted.",
    )
    parser.add_argument(
        "--pair-tags",
        default="",
        help="Comma-separated tumor/normal pair tags to process.",
    )
    parser.add_argument(
        "--chroms",
        help="Comma-separated chromosome names to process. Defaults to all discovered sweep BED chroms.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate shards even if the final output TSV already exists.",
    )
    parser.add_argument(
        "--output-only",
        action="store_true",
        help="Write only output TSVs and skip module JSONL fixture generation.",
    )
    config_group = parser.add_mutually_exclusive_group()
    config_group.add_argument(
        "--config",
        help="Preset slug from scripts/config_presets.tsv to append to VarDict.",
    )
    config_group.add_argument(
        "--presets",
        help="Preset slug CSV or ALL. Inverts execution to outer (tag,chrom) and inner presets.",
    )
    parser.add_argument(
        "--sweep-bed-root",
        default=SWEEP_BED_ROOT_REL.as_posix(),
        help="Override sweep BED root for testing (default: tmp/sweep_beds).",
    )
    parser.add_argument(
        "--output-dir",
        default=OUTPUT_ROOT_REL.as_posix(),
        help=f"Override output root for testing (default: {OUTPUT_ROOT_REL}).",
    )
    parser.add_argument(
        "--shm-root",
        default=DEFAULT_SHM_ROOT.as_posix(),
        help=f"Stage per-chrom BAM/FASTA slices here when shm is enabled (default: {DEFAULT_SHM_ROOT}).",
    )
    parser.add_argument(
        "--no-shm",
        action="store_true",
        help="Disable per-chrom staging and use source BAM/FASTA paths directly.",
    )
    parser.add_argument(
        "--manifest-path",
        default=None,
        help="Override manifest.json output path (default: <output_root>/manifest.json). "
        "Use a per-invocation staging path when running multiple presets in parallel.",
    )
    parser.add_argument(
        "--chunk-size",
        type=non_negative_int,
        default=CHUNK_SIZE,
        help=(
            f"Max tiles per VarDict invocation (default: {CHUNK_SIZE}). Use 0 to disable "
            "chunking and pass the source BED directly to VarDict."
        ),
    )
    parser.add_argument(
        "--timings-path",
        default="tmp/pilot_p1_timings.tsv",
        help="Per-shard timings TSV path (default: tmp/pilot_p1_timings.tsv).",
    )
    return parser


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def non_negative_int(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be at least 0")
    return parsed


def parse_tags(
    raw_tags: str | None,
    parser: argparse.ArgumentParser,
    *,
    allow_empty: bool = False,
) -> list[str]:
    if raw_tags is None:
        return [] if allow_empty else list(ALL_BAM_TAGS)

    selected: list[str] = []
    seen: set[str] = set()
    for part in raw_tags.split(","):
        tag = part.strip()
        if not tag:
            continue
        if tag not in BAM_MAP:
            parser.error(f"unknown BAM tag: {tag}")
        if tag not in seen:
            seen.add(tag)
            selected.append(tag)
    if not selected:
        if allow_empty:
            return []
        parser.error("no BAM tags selected")
    return selected


def parse_pair_tags(raw_tags: str | None, parser: argparse.ArgumentParser) -> list[str]:
    if raw_tags is None:
        return []

    selected: list[str] = []
    seen: set[str] = set()
    for part in raw_tags.split(","):
        tag = part.strip()
        if not tag:
            continue
        if tag not in SOMATIC_PAIR_MAP:
            parser.error(f"unknown pair tag: {tag}")
        if tag not in seen:
            seen.add(tag)
            selected.append(tag)
    return selected


def parse_chroms(
    raw_chroms: str | None,
    parser: argparse.ArgumentParser,
) -> set[str] | None:
    if raw_chroms is None:
        return None

    selected: set[str] = set()
    for part in raw_chroms.split(","):
        chrom = part.strip()
        if chrom:
            selected.add(chrom)
    if not selected:
        parser.error("no chromosomes selected")
    return selected


def normalize_chrom_label(chrom: str) -> str:
    chrom_lower = chrom.lower()
    if chrom_lower.startswith("chr"):
        return chrom_lower[3:]
    return chrom_lower


def chrom_selected(chroms: set[str] | None, chrom: str) -> bool:
    if chroms is None:
        return True
    normalized = normalize_chrom_label(chrom)
    return any(normalize_chrom_label(selected) == normalized for selected in chroms)


def project_root() -> Path:
    return Path(__file__).resolve().parent.parent


def load_all_presets(root: Path) -> dict[str, ConfigPreset]:
    preset_path = root / CONFIG_PRESETS_REL
    presets: dict[str, ConfigPreset] = {}
    with preset_path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if reader.fieldnames is None:
            raise SystemExit(f"ERROR: missing header in {preset_path}")
        reader.fieldnames = [field.lstrip("#").strip() for field in reader.fieldnames]
        for row in reader:
            name = row["name"].strip()
            presets[name] = ConfigPreset(
                name=name,
                flags=tuple(shlex.split(row["flags"].strip())),
            )
    if not presets:
        raise SystemExit(f"ERROR: no presets found in {preset_path}")
    return presets


def parse_presets(
    raw_presets: str | None,
    presets: dict[str, ConfigPreset],
    parser: argparse.ArgumentParser,
) -> list[ConfigPreset] | None:
    if raw_presets is None:
        return None

    if raw_presets.strip().upper() == "ALL":
        return list(presets.values())

    selected: list[ConfigPreset] = []
    seen: set[str] = set()
    for part in raw_presets.split(","):
        name = part.strip()
        if not name:
            continue
        if name == "DEFAULT":
            preset = ConfigPreset(name="DEFAULT", flags=())
        else:
            preset = presets.get(name)
        if preset is None:
            valid = ", ".join(["DEFAULT", *sorted(presets)])
            parser.error(f"unknown preset: {name}. Valid presets: {valid}")
        if name not in seen:
            seen.add(name)
            selected.append(preset)
    if not selected:
        parser.error("no presets selected")
    return selected


def resolve_path(root: Path, raw_path: str) -> Path:
    path = Path(raw_path)
    if not path.is_absolute():
        path = root / path
    return path.resolve()


def compress_path(path: Path) -> None:
    subprocess.run(["zstd", "--rm", "-q", str(path)], check=True)


def compute_file_md5(path: Path) -> str:
    digest = hashlib.md5()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def compute_file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_chunks_json(
    path: Path,
    *,
    monolithic_md5: str,
    monolithic_bytes: int,
    num_chunks: int,
    chunk_size: int,
    chunks: list[dict[str, object]],
    vardict_commit: str,
    generator_flags: list[str],
    preset: str,
    bed_sha256: str,
) -> None:
    payload = {
        "monolithic_md5": monolithic_md5,
        "monolithic_bytes": monolithic_bytes,
        "num_chunks": num_chunks,
        "chunk_size": chunk_size,
        "chunks": chunks,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "vardict_commit": vardict_commit,
        # Additive provenance for the A6 gate; validate_chunks_json intentionally ignores these.
        "generator_flags": generator_flags,
        "preset": preset,
        "bed_sha256": bed_sha256,
    }
    tmp_path = path.with_name(f"{path.name}.tmp")
    path.parent.mkdir(parents=True, exist_ok=True)
    with tmp_path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2, sort_keys=False)
        handle.write("\n")
    os.replace(tmp_path, path)


def is_plain_int(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def invalid_chunks_json(path: Path, reason: str) -> bool:
    print(f"DEBUG: invalid chunks.json {path}: {reason}", file=sys.stderr, flush=True)
    return False


def validate_chunks_json(path: Path) -> bool:
    required_keys = {
        "monolithic_md5",
        "monolithic_bytes",
        "num_chunks",
        "chunk_size",
        "chunks",
        "generated_at",
        "vardict_commit",
    }
    try:
        with path.open("r", encoding="utf-8") as handle:
            payload = json.load(handle)
    except Exception as exc:
        return invalid_chunks_json(path, str(exc))

    if not isinstance(payload, dict):
        return invalid_chunks_json(path, "top-level value is not an object")

    missing_keys = sorted(required_keys - payload.keys())
    if missing_keys:
        return invalid_chunks_json(path, f"missing required keys: {', '.join(missing_keys)}")

    monolithic_md5 = payload["monolithic_md5"]
    if not isinstance(monolithic_md5, str) or len(monolithic_md5) != 32:
        return invalid_chunks_json(path, "monolithic_md5 is not a 32-character string")
    if any(char not in "0123456789abcdefABCDEF" for char in monolithic_md5):
        return invalid_chunks_json(path, "monolithic_md5 contains non-hex characters")

    monolithic_bytes = payload["monolithic_bytes"]
    if not is_plain_int(monolithic_bytes) or monolithic_bytes < 0:
        return invalid_chunks_json(path, "monolithic_bytes is not a non-negative int")

    num_chunks = payload["num_chunks"]
    if not is_plain_int(num_chunks) or num_chunks <= 0:
        return invalid_chunks_json(path, "num_chunks is not a positive int")

    chunk_size = payload["chunk_size"]
    if not is_plain_int(chunk_size) or chunk_size < 0:
        return invalid_chunks_json(path, "chunk_size is not a non-negative int")

    chunks = payload["chunks"]
    if not isinstance(chunks, list):
        return invalid_chunks_json(path, "chunks is not a list")
    if num_chunks != len(chunks):
        return invalid_chunks_json(path, "num_chunks does not match chunks length")

    next_offset = 0
    for chunk_idx, chunk in enumerate(chunks):
        if not isinstance(chunk, dict):
            return invalid_chunks_json(path, f"chunk {chunk_idx} is not an object")
        byte_range = chunk.get("byte_range")
        if not isinstance(byte_range, list) or len(byte_range) != 2:
            return invalid_chunks_json(path, f"chunk {chunk_idx} has invalid byte_range")
        chunk_offset, chunk_length = byte_range
        if not is_plain_int(chunk_offset) or not is_plain_int(chunk_length):
            return invalid_chunks_json(path, f"chunk {chunk_idx} byte_range is not integer-valued")
        if chunk_offset < 0 or chunk_length < 0:
            return invalid_chunks_json(path, f"chunk {chunk_idx} byte_range is negative")
        if chunk_offset != next_offset:
            return invalid_chunks_json(path, f"chunk {chunk_idx} starts at {chunk_offset}, expected {next_offset}")
        next_offset += chunk_length

    if next_offset != monolithic_bytes:
        return invalid_chunks_json(path, "chunk byte ranges do not sum to monolithic_bytes")

    return True


def shard_output_paths(final_dir: Path, tag: str, chrom: str) -> tuple[Path, Path]:
    stem = f"{tag}_{chrom}"
    return final_dir / f"{stem}.tsv.zst", final_dir / f"{stem}.chunks.json"


def shard_is_complete(output_dir: Path, tag: str, chrom: str) -> bool:
    output_path, chunks_path = shard_output_paths(output_dir, tag, chrom)
    return output_path.exists() and chunks_path.exists() and validate_chunks_json(chunks_path)


def cleanup_partial_shard(final_dir: Path, tag: str, chrom: str) -> None:
    output_path, chunks_path = shard_output_paths(final_dir, tag, chrom)
    output_path.unlink(missing_ok=True)
    chunks_path.unlink(missing_ok=True)


def compress_and_promote_chunks(
    stdout_path: Path,
    chunks_path: Path,
    final_chunks_path: Path,
    cleanup_dir: Path | None = None,
) -> None:
    compress_path(stdout_path)
    os.replace(chunks_path, final_chunks_path)
    if cleanup_dir is not None:
        cleanup_dir.rmdir()


def resolve_direct_inputs(root: Path, shard: Shard) -> tuple[str, Path]:
    if shard.kind == "pair":
        tumor_rel, normal_rel = SOMATIC_PAIR_MAP[shard.tag]
        bam_arg = f"{(root / tumor_rel).resolve()}|{(root / normal_rel).resolve()}"
        reference = (root / SOMATIC_REF_MAP[shard.tag]).resolve()
        return bam_arg, reference

    return str((root / BAM_MAP[shard.tag]).resolve()), (root / REF_REL).resolve()


def resolve_vardict_inputs(
    root: Path,
    shard: Shard,
    use_shm: bool,
    shm_root: Path,
    samtools_bin: str | None,
) -> tuple[str, Path, StagedChrom | None]:
    bam_arg, reference = resolve_direct_inputs(root, shard)
    if not use_shm or shard.kind != "single":
        return bam_arg, reference, None

    if samtools_bin is None:
        raise RuntimeError("samtools path is required when shm staging is enabled")

    bam_source = (root / BAM_MAP[shard.tag]).resolve()
    for candidate_root in (shm_root, FALLBACK_SHM_ROOT):
        try:
            staged = stage_chrom(
                tag=shard.tag,
                chrom=shard.chrom,
                bam_source=bam_source,
                fasta_source=(root / REF_REL).resolve(),
                shm_root=candidate_root,
                samtools_bin=Path(samtools_bin),
            )
            return str(staged.bam_path), staged.fasta_path, staged
        except ShmCapacityError:
            continue

    staged = stage_chrom(
        tag=shard.tag,
        chrom=shard.chrom,
        bam_source=bam_source,
        fasta_source=(root / REF_REL).resolve(),
        shm_root=FALLBACK_SHM_ROOT,
        samtools_bin=Path(samtools_bin),
    )
    return str(staged.bam_path), staged.fasta_path, staged


def discover_shards(
    root: Path,
    tags: list[str],
    pair_tags: list[str],
    sweep_bed_root: Path,
    chroms: set[str] | None,
) -> list[Shard]:
    shards: list[Shard] = []
    for tag in tags:
        tag_dir = sweep_bed_root / tag
        if not tag_dir.is_dir():
            raise SystemExit(f"ERROR: sweep BED directory not found for {tag}: {tag_dir}")
        bed_files = [
            bed_file
            for bed_file in sorted(tag_dir.glob("*.bed"))
            if chrom_selected(chroms, bed_file.stem)
        ]
        if not bed_files:
            raise SystemExit(f"ERROR: no BED files found for {tag}: {tag_dir}")
        for bed_file in bed_files:
            shards.append(
                Shard(tag=tag, chrom=bed_file.stem, bed_path=bed_file.resolve(), kind="single")
            )
    for tag in pair_tags:
        tag_dir = sweep_bed_root / tag
        if not tag_dir.is_dir():
            raise SystemExit(f"ERROR: sweep BED directory not found for {tag}: {tag_dir}")
        bed_files = [
            bed_file
            for bed_file in sorted(tag_dir.glob("*.bed"))
            if chrom_selected(chroms, bed_file.stem)
        ]
        if not bed_files:
            raise SystemExit(f"ERROR: no BED files found for {tag}: {tag_dir}")
        for bed_file in bed_files:
            shards.append(
                Shard(tag=tag, chrom=bed_file.stem, bed_path=bed_file.resolve(), kind="pair")
            )
    return shards


def ensure_dependencies(root: Path) -> None:
    if shutil.which("zstd") is None:
        raise SystemExit(
            "ERROR: zstd is required but not found in PATH. Activate rust_build_env first."
        )

    vardict_bin = root / VARDICT_BIN_REL
    if not vardict_bin.is_file() or not os.access(vardict_bin, os.X_OK):
        raise SystemExit(
            f"ERROR: VarDictJava binary not found or not executable: {vardict_bin}\n"
            "Build it first with: cd VarDictJava && ./gradlew installDist -q"
        )


def cleanup_dirs(paths: list[Path]) -> None:
    for path in paths:
        if path.exists():
            shutil.rmtree(path, ignore_errors=True)


def compress_paths(paths: list[Path], batch_size: int = 500) -> None:
    if not paths:
        return
    for i in range(0, len(paths), batch_size):
        batch = paths[i : i + batch_size]
        subprocess.run(["zstd", "--rm", "-q", *[str(p) for p in batch]], check=True)


def promote_files(staging_dir: Path, final_dir: Path, pattern: str) -> None:
    final_dir.mkdir(parents=True, exist_ok=True)
    for source in sorted(staging_dir.glob(pattern)):
        os.replace(source, final_dir / source.name)
    staging_dir.rmdir()


def output_dir(output_root: Path, config_name: str | None, chrom: str) -> Path:
    if config_name:
        return output_root / "output" / config_name / chrom
    return output_root / "output" / chrom


def module_dir(output_root: Path, module_name: str, config_name: str | None, chrom: str) -> Path:
    if config_name:
        return output_root / module_name / config_name / chrom
    return output_root / module_name / chrom


def output_staging_dir(output_root: Path, config_name: str | None, shard: Shard) -> Path:
    if config_name:
        return output_root / "output" / config_name / f"{shard.tag}_{shard.chrom}.staging"
    return output_root / "output" / f"{shard.tag}_{shard.chrom}.staging"


def module_staging_dir(output_root: Path, module_name: str, config_name: str | None, shard: Shard) -> Path:
    if config_name:
        return output_root / module_name / config_name / f"{shard.tag}_{shard.chrom}.staging"
    return output_root / module_name / f"{shard.tag}_{shard.chrom}.staging"


def log_file_path(output_root: Path, config_name: str | None, shard: Shard) -> Path:
    if config_name:
        return output_root / "logs" / config_name / shard.tag / f"{shard.chrom}.log"
    return output_root / "logs" / shard.tag / f"{shard.chrom}.log"


# Maximum tiles per VarDict invocation when chunking is explicitly enabled.
CHUNK_SIZE = 20_000


def count_bed_lines(bed_path: Path) -> int:
    count = 0
    with bed_path.open("r", encoding="utf-8") as handle:
        for _ in handle:
            count += 1
    return count


def split_bed_chunks(bed_path: Path, chunk_size: int, tmp_dir: Path) -> list[Path]:
    chunks: list[Path] = []
    chunk_idx = 0
    line_count = 0
    handle = None
    try:
        with bed_path.open("r", encoding="utf-8") as src:
            for line in src:
                if line_count % chunk_size == 0:
                    if handle is not None:
                        handle.close()
                    chunk_path = tmp_dir / f"chunk_{chunk_idx}.bed"
                    chunks.append(chunk_path)
                    handle = chunk_path.open("w", encoding="utf-8")
                    chunk_idx += 1
                handle.write(line)
                line_count += 1
    finally:
        if handle is not None:
            handle.close()
    return chunks


def percentile(sorted_values: list[float], fraction: float) -> float:
    if not sorted_values:
        return 0.0
    if len(sorted_values) == 1:
        return sorted_values[0]

    rank = (len(sorted_values) - 1) * fraction
    lower_index = math.floor(rank)
    upper_index = math.ceil(rank)
    if lower_index == upper_index:
        return sorted_values[lower_index]

    lower_value = sorted_values[lower_index]
    upper_value = sorted_values[upper_index]
    weight = rank - lower_index
    return lower_value + (upper_value - lower_value) * weight


def build_timing_summary(
    *,
    shard_start: float,
    chunk_wall_s: list[float],
    num_chunks: int,
) -> dict[str, float | int]:
    wall_s = time.monotonic() - shard_start
    if chunk_wall_s:
        sorted_chunk_walls = sorted(chunk_wall_s)
        chunk_wall_p50 = percentile(sorted_chunk_walls, 0.50)
        chunk_wall_p99 = percentile(sorted_chunk_walls, 0.99)
    else:
        chunk_wall_p50 = 0.0
        chunk_wall_p99 = 0.0

    return {
        "num_chunks": num_chunks,
        "wall_s": wall_s,
        "chunk_wall_p50": chunk_wall_p50,
        "chunk_wall_p99": chunk_wall_p99,
        "jvm_invocations": num_chunks,
    }


def resume_or_cleanup_shard(
    *,
    shard: Shard,
    force: bool,
    output_root: Path,
    config_name: str | None,
    shard_start: float,
) -> ShardResult | None:
    if force:
        return None

    final_dir = output_dir(output_root, config_name, shard.chrom)
    output_path, chunks_path = shard_output_paths(final_dir, shard.tag, shard.chrom)
    if shard_is_complete(final_dir, shard.tag, shard.chrom):
        print(f"INFO: resume-skipped {shard.tag}/{shard.chrom}", file=sys.stderr, flush=True)
        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="skipped",
            config_name=config_name,
            fixture_count=0,
            log_path="(resume-skipped)",
            **build_timing_summary(
                shard_start=shard_start,
                chunk_wall_s=[],
                num_chunks=0,
            ),
        )

    if output_path.exists() or chunks_path.exists():
        print(
            f"WARN: regenerating shard {shard.tag}/{shard.chrom} (incomplete prior run)",
            file=sys.stderr,
            flush=True,
        )
        cleanup_partial_shard(final_dir, shard.tag, shard.chrom)

    return None


def initialize_timings_file(timings_path: Path) -> None:
    timings_path.parent.mkdir(parents=True, exist_ok=True)
    with timings_path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t")
        writer.writerow(TIMINGS_HEADER)


def append_timing_rows(timings_path: Path, results: list[ShardResult]) -> None:
    with timings_path.open("a", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t")
        for result in results:
            writer.writerow(
                [
                    result.config_name or "default",
                    result.tag,
                    result.chrom,
                    result.num_chunks,
                    f"{result.wall_s:.6f}",
                    f"{result.chunk_wall_p50:.6f}",
                    f"{result.chunk_wall_p99:.6f}",
                    result.jvm_invocations,
                    result.status,
                ]
            )


def execute_shard_run(
    shard: Shard,
    force: bool,
    output_only: bool,
    config_name: str | None,
    config_flags: tuple[str, ...],
    root: Path,
    output_root: Path,
    chunk_size: int,
    bam_arg: str,
    reference: Path,
    zstd_executor: concurrent.futures.ThreadPoolExecutor | None = None,
    pending_compressions: list[
        tuple[tuple[str | None, str, str], concurrent.futures.Future[None]]
    ] | None = None,
    sample_name_override: str | None = None,
) -> ShardResult:
    t_shard_start = time.monotonic()
    vardict_bin = (root / VARDICT_BIN_REL).resolve()
    effective_output_only = output_only or shard.kind == "pair"
    log_path = log_file_path(output_root, config_name, shard)

    resume_result = resume_or_cleanup_shard(
        shard=shard,
        force=force,
        output_root=output_root,
        config_name=config_name,
        shard_start=t_shard_start,
    )
    if resume_result is not None:
        return resume_result

    module_staging: dict[str, Path] = {}
    staging_dirs: list[Path] = []
    if not effective_output_only:
        for module_name, _ in MODULES:
            staging_dir = module_staging_dir(output_root, module_name, config_name, shard)
            if staging_dir.exists():
                shutil.rmtree(staging_dir)
            staging_dir.mkdir(parents=True, exist_ok=True)
            module_staging[module_name] = staging_dir
            staging_dirs.append(staging_dir)

    output_staging = output_staging_dir(output_root, config_name, shard)
    if output_staging.exists():
        shutil.rmtree(output_staging)
    output_staging.mkdir(parents=True, exist_ok=True)
    staging_dirs.append(output_staging)

    log_path.parent.mkdir(parents=True, exist_ok=True)
    stdout_path = output_staging / f"{shard.tag}_{shard.chrom}.tsv"
    chunks_path = stdout_path.with_suffix(".chunks.json")
    env = os.environ.copy()
    env["JAVA_OPTS"] = "-Xms512m -Xmx4g"
    if not effective_output_only:
        for module_name, env_name in MODULES:
            env[env_name] = str(module_staging[module_name].resolve())

    total_bed_lines = count_bed_lines(shard.bed_path)
    need_chunking = chunk_size > 0 and total_bed_lines > chunk_size

    if need_chunking:
        chunk_beds = split_bed_chunks(shard.bed_path, chunk_size, output_staging)
    else:
        chunk_beds = [shard.bed_path]
    num_chunks = len(chunk_beds)
    chunk_wall_s: list[float] = []
    chunk_records: list[dict[str, object]] = []
    raw_offset = 0

    try:
        fixture_count = 0

        for chunk_idx, chunk_bed in enumerate(chunk_beds):
            stdout_path_chunk = output_staging / f"{stdout_path.stem}.chunk_{chunk_idx:04d}.tsv"
            num_tiles = count_bed_lines(chunk_bed) if need_chunking else total_bed_lines
            command = [
                str(vardict_bin),
                "-G",
                str(reference),
                "-b",
                bam_arg,
                "-th",
                "1",
                "-c",
                "1",
                "-S",
                "2",
                "-E",
                "3",
                "-s",
                "2",
                "-e",
                "3",
            ]
            if sample_name_override is not None:
                command.extend(["-N", sample_name_override])
            command.extend([*config_flags, str(chunk_bed)])

            stderr_mode = "ab" if chunk_idx > 0 else "wb"

            with stdout_path_chunk.open("wb") as stdout_handle, log_path.open(
                stderr_mode
            ) as stderr_handle:
                t_chunk_start = time.monotonic()
                completed = subprocess.run(
                    command,
                    cwd=root,
                    env=env,
                    stdout=stdout_handle,
                    stderr=stderr_handle,
                    check=False,
                )
                chunk_wall = time.monotonic() - t_chunk_start
                chunk_wall_s.append(chunk_wall)

            if completed.returncode != 0:
                cleanup_dirs(staging_dirs)
                return ShardResult(
                    tag=shard.tag,
                    chrom=shard.chrom,
                    status="failed",
                    error=f"VarDict exited with code {completed.returncode} (chunk {chunk_idx})",
                    log_path=str(log_path),
                    **build_timing_summary(
                        shard_start=t_shard_start,
                        chunk_wall_s=chunk_wall_s,
                        num_chunks=num_chunks,
                    ),
                )

            chunk_md5 = compute_file_md5(stdout_path_chunk)
            chunk_bytes = stdout_path_chunk.stat().st_size
            offset_before_append = raw_offset
            with stdout_path_chunk.open("rb") as chunk_handle, stdout_path.open(
                "ab"
            ) as stdout_handle:
                shutil.copyfileobj(chunk_handle, stdout_handle)
            raw_offset += chunk_bytes
            stdout_path_chunk.unlink(missing_ok=True)
            chunk_records.append(
                {
                    "idx": chunk_idx,
                    "md5_raw": chunk_md5,
                    "wall_s": chunk_wall,
                    "num_tiles": num_tiles,
                    "byte_range": [offset_before_append, chunk_bytes],
                }
            )

            # Compress JSONL files after each chunk to bound disk usage.
            if not effective_output_only:
                for module_name, _ in MODULES:
                    jsonl_files = sorted(module_staging[module_name].glob("*.jsonl"))
                    fixture_count += len(jsonl_files)
                    compress_paths(jsonl_files)

            # Remove chunk BED file after use.
            if need_chunking:
                chunk_bed.unlink(missing_ok=True)

        if not effective_output_only:
            for module_name, _ in MODULES:
                promote_files(
                    module_staging[module_name],
                    module_dir(output_root, module_name, config_name, shard.chrom),
                    "*.jsonl.zst",
                )

        monolithic_md5 = compute_file_md5(stdout_path)
        monolithic_bytes = stdout_path.stat().st_size
        write_chunks_json(
            chunks_path,
            monolithic_md5=monolithic_md5,
            monolithic_bytes=monolithic_bytes,
            num_chunks=num_chunks,
            chunk_size=chunk_size,
            chunks=chunk_records,
            vardict_commit=get_vardict_commit(root),
            generator_flags=list(config_flags),
            preset=config_name if config_name else "default",
            bed_sha256=compute_file_sha256(shard.bed_path),
        )

        if zstd_executor is None:
            compress_path(stdout_path)
            final_output_dir = output_dir(output_root, config_name, shard.chrom)
            final_output_dir.mkdir(parents=True, exist_ok=True)
            os.replace(
                stdout_path.with_name(f"{stdout_path.name}.zst"),
                final_output_dir / f"{stdout_path.name}.zst",
            )
            os.replace(chunks_path, final_output_dir / chunks_path.name)
            output_staging.rmdir()
        else:
            final_output_dir = output_dir(output_root, config_name, shard.chrom)
            final_output_dir.mkdir(parents=True, exist_ok=True)
            final_stdout_path = final_output_dir / stdout_path.name
            final_zst_path = final_stdout_path.with_name(f"{final_stdout_path.name}.zst")
            final_chunks_path = final_output_dir / chunks_path.name
            final_stdout_path.unlink(missing_ok=True)
            final_zst_path.unlink(missing_ok=True)
            final_chunks_path.unlink(missing_ok=True)
            os.replace(stdout_path, final_stdout_path)
            future = zstd_executor.submit(
                compress_and_promote_chunks,
                final_stdout_path,
                chunks_path,
                final_chunks_path,
                output_staging,
            )
            if pending_compressions is not None:
                pending_compressions.append(
                    ((config_name, shard.tag, shard.chrom), future)
                )
            else:
                future.result()

        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="success",
            config_name=config_name,
            fixture_count=fixture_count,
            log_path=str(log_path),
            **build_timing_summary(
                shard_start=t_shard_start,
                chunk_wall_s=chunk_wall_s,
                num_chunks=num_chunks,
            ),
        )
    except Exception as exc:
        cleanup_dirs(staging_dirs)
        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="failed",
            config_name=config_name,
            error=str(exc),
            log_path=str(log_path),
            **build_timing_summary(
                shard_start=t_shard_start,
                chunk_wall_s=chunk_wall_s,
                num_chunks=num_chunks,
            ),
        )


def run_shard(
    shard: Shard,
    force: bool,
    output_only: bool,
    config_name: str | None,
    config_flags: tuple[str, ...],
    root_str: str,
    output_root_str: str,
    chunk_size: int = CHUNK_SIZE,
    use_shm: bool = True,
    shm_root_str: str = DEFAULT_SHM_ROOT.as_posix(),
    samtools_bin: str | None = None,
) -> ShardResult:
    root = Path(root_str)
    output_root = Path(output_root_str)
    resume_result = resume_or_cleanup_shard(
        shard=shard,
        force=force,
        output_root=output_root,
        config_name=config_name,
        shard_start=time.monotonic(),
    )
    if resume_result is not None:
        return resume_result

    staged: StagedChrom | None = None
    try:
        bam_arg, reference, staged = resolve_vardict_inputs(
            root=root,
            shard=shard,
            use_shm=use_shm,
            shm_root=Path(shm_root_str),
            samtools_bin=samtools_bin,
        )
        sample_name_override = None
        if staged is not None and shard.kind == "single":
            sample_name_override = Path(BAM_MAP[shard.tag]).stem
        return execute_shard_run(
            shard=shard,
            force=force,
            output_only=output_only,
            config_name=config_name,
            config_flags=config_flags,
            root=root,
            output_root=output_root,
            chunk_size=chunk_size,
            bam_arg=bam_arg,
            reference=reference,
            sample_name_override=sample_name_override,
        )
    except Exception as exc:
        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="failed",
            config_name=config_name,
            error=str(exc),
        )
    finally:
        if staged is not None:
            evict_chrom(staged)


def run_inverted_shards(
    shards: list[Shard],
    presets: list[ConfigPreset],
    force: bool,
    output_only: bool,
    root: Path,
    output_root: Path,
    chunk_size: int,
    use_shm: bool,
    shm_root: Path,
    samtools_bin: str | None,
    workers: int,
) -> list[ShardResult]:
    results: list[ShardResult] = []

    for shard in shards:
        runnable_presets: list[ConfigPreset] = []
        for preset in presets:
            resume_result = resume_or_cleanup_shard(
                shard=shard,
                force=force,
                output_root=output_root,
                config_name=preset.name,
                shard_start=time.monotonic(),
            )
            if resume_result is None:
                runnable_presets.append(preset)
            else:
                results.append(resume_result)
        if not runnable_presets:
            continue

        staged: StagedChrom | None = None
        try:
            bam_arg, reference, staged = resolve_vardict_inputs(
                root=root,
                shard=shard,
                use_shm=use_shm,
                shm_root=shm_root,
                samtools_bin=samtools_bin,
            )
            sample_name_override = None
            if staged is not None and shard.kind == "single":
                sample_name_override = Path(BAM_MAP[shard.tag]).stem
        except Exception as exc:
            for preset in runnable_presets:
                results.append(
                    ShardResult(
                        tag=shard.tag,
                        chrom=shard.chrom,
                        status="failed",
                        config_name=preset.name,
                        error=str(exc),
                    )
                )
            continue

        try:
            pending_compressions: list[
                tuple[tuple[str | None, str, str], concurrent.futures.Future[None]]
            ] = []
            result_index_by_key: dict[tuple[str | None, str, str], int] = {}
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as zstd_executor:
                with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
                    future_map = {
                        executor.submit(
                            execute_shard_run,
                            shard,
                            force,
                            output_only,
                            preset.name,
                            preset.flags,
                            root,
                            output_root,
                            chunk_size,
                            bam_arg,
                            reference,
                            zstd_executor,
                            pending_compressions,
                            sample_name_override,
                        ): preset.name
                        for preset in runnable_presets
                    }
                    for future in concurrent.futures.as_completed(future_map):
                        preset_name = future_map[future]
                        try:
                            result = future.result()
                        except Exception as exc:
                            result = ShardResult(
                                tag=shard.tag,
                                chrom=shard.chrom,
                                status="failed",
                                config_name=preset_name,
                                error=str(exc),
                            )
                        result_key = (result.config_name, result.tag, result.chrom)
                        result_index_by_key[result_key] = len(results)
                        results.append(result)
                for result_key, future in pending_compressions:
                    try:
                        future.result()
                    except Exception as exc:
                        result_index = result_index_by_key.get(result_key)
                        failed_result = ShardResult(
                            tag=result_key[1],
                            chrom=result_key[2],
                            status="failed",
                            config_name=result_key[0],
                            error=str(exc),
                            log_path=(
                                results[result_index].log_path if result_index is not None else ""
                            ),
                        )
                        if result_index is None:
                            results.append(failed_result)
                        else:
                            results[result_index] = failed_result
        finally:
            if staged is not None:
                evict_chrom(staged)

    return results


def collect_region_bams(shards: list[Shard]) -> dict[str, set[str]]:
    region_bams: dict[str, set[str]] = {}
    for shard in shards:
        if shard.kind != "single":
            continue
        bam_rel = BAM_MAP[shard.tag]
        with shard.bed_path.open("r", encoding="utf-8") as handle:
            for line in handle:
                parts = line.rstrip("\n").split("\t")
                if len(parts) < 3 or not parts[0]:
                    continue
                start = int(parts[1]) + 1
                end = parts[2]
                region = f"{parts[0]}:{start}-{end}"
                region_bams.setdefault(region, set()).add(bam_rel)
    return region_bams


def scan_fixture_regions(output_root: Path) -> set[str]:
    regions: set[str] = set()
    for module_name, _ in MODULES:
        module_root = output_root / module_name
        if not module_root.is_dir():
            continue
        for file_path in module_root.rglob("*.jsonl.zst"):
            chrom = file_path.parent.name
            prefix = f"{module_name}_{chrom}_"
            suffix = ".jsonl.zst"
            name = file_path.name
            if not name.startswith(prefix) or not name.endswith(suffix):
                continue
            remainder = name[len(prefix) : -len(suffix)]
            start, _, end = remainder.partition("_")
            if start.isdigit() and end.isdigit():
                regions.add(f"{chrom}:{start}-{end}")
    return regions


def write_regions_tsv(output_root: Path, region_bams: dict[str, set[str]]) -> int:
    regions_path = output_root / "regions.tsv"
    fixture_regions = scan_fixture_regions(output_root)
    region_count = 0
    with regions_path.open("w", encoding="utf-8") as handle:
        for region in sorted(fixture_regions):
            for bam_rel in sorted(region_bams.get(region, set())):
                handle.write(f"{region}\t{bam_rel}\t{REF_REL.as_posix()}\n")
                region_count += 1
    return region_count


def count_fixture_files(output_root: Path) -> int:
    total = 0
    for module_name, _ in MODULES:
        module_root = output_root / module_name
        if module_root.is_dir():
            total += sum(1 for _ in module_root.rglob("*.jsonl.zst"))
    return total


def get_vardict_commit(root: Path) -> str:
    vardict_root = (root / "VarDictJava").resolve()
    cached_commit = VARDICT_COMMIT_CACHE.get(vardict_root)
    if cached_commit is not None:
        return cached_commit

    completed = subprocess.run(
        ["git", "-C", str(vardict_root), "rev-parse", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=True,
    )
    commit = completed.stdout.strip()
    VARDICT_COMMIT_CACHE[vardict_root] = commit
    return commit


def write_manifest(
    output_root: Path,
    selected_tags: list[str],
    selected_pair_tags: list[str],
    shard_count: int,
    failed_shards: list[ShardResult],
    vardict_commit: str,
    region_count: int,
    fixture_count: int,
    mode: str,
    config_name: str | None,
    manifest_path: Path | None = None,
) -> None:
    manifest = {
        "vardictjava_commit": vardict_commit,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "bam_tags": selected_tags,
        "pair_tags": selected_pair_tags,
        "bam_paths": {tag: BAM_MAP[tag] for tag in selected_tags},
        "pair_bam_paths": {
            tag: {"tumor": tumor, "normal": normal}
            for tag, (tumor, normal) in SOMATIC_PAIR_MAP.items()
            if tag in selected_pair_tags
        },
        "tag_modes": {
            **{tag: "single" for tag in selected_tags},
            **{tag: "somatic" for tag in selected_pair_tags},
        },
        "mode": mode,
        "config": config_name,
        "region_count": region_count,
        "fixture_count": fixture_count,
        "shard_count": shard_count,
        "failed_shards": [
            (
                f"{result.config_name}/{result.tag}/{result.chrom}"
                if result.config_name
                else f"{result.tag}/{result.chrom}"
            )
            for result in failed_shards
        ],
    }
    target_path = manifest_path if manifest_path is not None else output_root / "manifest.json"
    target_path.parent.mkdir(parents=True, exist_ok=True)
    with target_path.open("w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")


def write_run_summary(
    output_root: Path,
    *,
    shard_count: int,
    completed_shards: int,
    skipped_shards: int,
    failed_shards: int,
    results: list[ShardResult],
) -> Path:
    summary_path = output_root / "run_summary.json"
    summary = {
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "shard_count": shard_count,
        "success": completed_shards,
        "skipped": skipped_shards,
        "failed": failed_shards,
        "results": [
            {
                "config": result.config_name or "default",
                "tag": result.tag,
                "chrom": result.chrom,
                "status": result.status,
            }
            for result in results
        ],
    }
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    with summary_path.open("w", encoding="utf-8") as handle:
        json.dump(summary, handle, indent=2, sort_keys=False)
        handle.write("\n")
    return summary_path


def emit(message: str, log_handle: TextIO, stream: TextIO = sys.stdout) -> None:
    print(message, file=stream, flush=True)
    log_handle.write(message + "\n")
    log_handle.flush()


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if args.chunk_size == 0:
        print(
            "WARN: --chunk-size 0 disables chunked-mode diagnostics. Sidecar will report "
            "num_chunks=1; consider --chunk-size 20000 for per-chunk md5 localization.",
            file=sys.stderr,
        )
    selected_pair_tags = parse_pair_tags(args.pair_tags, parser)
    selected_tags = parse_tags(args.tags, parser, allow_empty=bool(selected_pair_tags))
    selected_chroms = parse_chroms(args.chroms, parser)
    if not selected_tags and not selected_pair_tags:
        parser.error("at least one of --tags or --pair-tags must select a shard family")
    root = project_root()
    output_root = resolve_path(root, args.output_dir)
    timings_path = Path(args.timings_path)
    if not timings_path.is_absolute():
        timings_path = (root / timings_path).resolve()
    sweep_bed_root = resolve_path(root, args.sweep_bed_root)
    shm_root = resolve_path(root, args.shm_root)
    presets = load_all_presets(root)
    selected_presets = parse_presets(args.presets, presets, parser)
    config_name = None
    config_flags: tuple[str, ...] = ()
    if args.config:
        preset = presets.get(args.config)
        if preset is None:
            valid = ", ".join(sorted(presets))
            parser.error(f"unknown config: {args.config}. Valid configs: {valid}")
        config_name = preset.name
        config_flags = preset.flags

    output_root.mkdir(parents=True, exist_ok=True)
    run_log_path = root / RUN_LOG_REL
    run_log_path.parent.mkdir(parents=True, exist_ok=True)
    run_output_only = args.output_only or (bool(selected_pair_tags) and not bool(selected_tags))

    ensure_dependencies(root)
    shards = discover_shards(root, selected_tags, selected_pair_tags, sweep_bed_root, selected_chroms)
    use_shm = not args.no_shm
    samtools_bin = None
    if use_shm and any(shard.kind == "single" for shard in shards):
        samtools_bin = str(locate_samtools())
    region_bams = collect_region_bams(shards) if not run_output_only else {}

    with run_log_path.open("w", encoding="utf-8") as run_log:
        if timings_path.exists() and not args.force:
            emit(
                f"WARNING: overwriting existing timings TSV without --force: {timings_path}",
                run_log,
                stream=sys.stderr,
            )
        initialize_timings_file(timings_path)

        emit("=== sweep_fixtures_parallel ===", run_log)
        emit(f"Project root:  {root}", run_log)
        emit(f"Sweep BEDs:    {sweep_bed_root}", run_log)
        emit(f"Output root:   {output_root}", run_log)
        emit(f"Timings TSV:   {timings_path}", run_log)
        emit(f"BAM tags:      {','.join(selected_tags) if selected_tags else '(none)'}", run_log)
        emit(
            f"Pair tags:     {','.join(selected_pair_tags) if selected_pair_tags else '(none)'}",
            run_log,
        )
        emit(f"Workers:       {args.workers}", run_log)
        emit(f"Force:         {1 if args.force else 0}", run_log)
        emit(f"Mode:          {'output_only' if run_output_only else 'full'}", run_log)
        emit(f"Config:        {config_name or 'default'}", run_log)
        emit(
            f"Presets:       {','.join(preset.name for preset in selected_presets) if selected_presets else '(none)'}",
            run_log,
        )
        emit(f"SHM enabled:   {1 if use_shm else 0}", run_log)
        emit(f"SHM root:      {shm_root}", run_log)
        chunk_default_note = " (default since S4)" if args.chunk_size == CHUNK_SIZE else ""
        emit(f"Chunk size:    {args.chunk_size}{chunk_default_note}", run_log)
        emit("", run_log)

        results: list[ShardResult] = []
        if selected_presets is not None:
            results = run_inverted_shards(
                shards=shards,
                presets=selected_presets,
                force=args.force,
                output_only=run_output_only,
                root=root,
                output_root=output_root,
                chunk_size=args.chunk_size,
                use_shm=use_shm,
                shm_root=shm_root,
                samtools_bin=samtools_bin,
                workers=args.workers,
            )
            for result in results:
                label = result.config_name or "default"
                if result.status == "success":
                    emit(
                        f"[{label}] [{result.tag}] {result.chrom}: {result.fixture_count} fixtures, done",
                        run_log,
                    )
                elif result.status == "skipped":
                    emit(f"[{label}] [{result.tag}] {result.chrom}: skipped", run_log)
                else:
                    detail = f"[{label}] [{result.tag}] {result.chrom}: FAILED: {result.error}"
                    if result.log_path:
                        detail += f" (stderr: {result.log_path})"
                    emit(detail, run_log, stream=sys.stderr)
        else:
            with concurrent.futures.ProcessPoolExecutor(max_workers=args.workers) as executor:
                future_map = {
                    executor.submit(
                        run_shard,
                        shard,
                        args.force,
                        run_output_only,
                        config_name,
                        config_flags,
                        str(root),
                        str(output_root),
                        args.chunk_size,
                        use_shm,
                        str(shm_root),
                        samtools_bin,
                    ): shard
                    for shard in shards
                }
                for future in concurrent.futures.as_completed(future_map):
                    shard = future_map[future]
                    try:
                        result = future.result()
                    except Exception as exc:
                        result = ShardResult(
                            tag=shard.tag,
                            chrom=shard.chrom,
                            status="failed",
                            config_name=config_name,
                            error=str(exc),
                        )
                    results.append(result)
                    if result.status == "success":
                        emit(
                            f"[{result.tag}] {result.chrom}: {result.fixture_count} fixtures, done",
                            run_log,
                        )
                    elif result.status == "skipped":
                        emit(f"[{result.tag}] {result.chrom}: skipped", run_log)
                    else:
                        detail = f"[{result.tag}] {result.chrom}: FAILED: {result.error}"
                        if result.log_path:
                            detail += f" (stderr: {result.log_path})"
                        emit(detail, run_log, stream=sys.stderr)

        append_timing_rows(timings_path, results)

        failed_shards = [result for result in results if result.status == "failed"]
        skipped_shards = sum(1 for result in results if result.status == "skipped")
        completed_shards = sum(1 for result in results if result.status == "success")
        shard_count = len(results) if selected_presets is not None else len(shards)

        region_count = 0
        if not run_output_only:
            region_count = write_regions_tsv(output_root, region_bams)

        fixture_count = 0 if run_output_only else count_fixture_files(output_root)
        vardict_commit = get_vardict_commit(root)
        manifest_override = Path(args.manifest_path) if args.manifest_path else None
        if manifest_override is not None and not manifest_override.is_absolute():
            manifest_override = (root / manifest_override).resolve()
        summary_path = write_run_summary(
            output_root=output_root,
            shard_count=shard_count,
            completed_shards=completed_shards,
            skipped_shards=skipped_shards,
            failed_shards=len(failed_shards),
            results=results,
        )
        write_manifest(
            output_root=output_root,
            selected_tags=selected_tags,
            selected_pair_tags=selected_pair_tags,
            shard_count=shard_count,
            failed_shards=failed_shards,
            vardict_commit=vardict_commit,
            region_count=region_count,
            fixture_count=fixture_count,
            mode="output_only" if run_output_only else "full",
            config_name=config_name if selected_presets is None else None,
            manifest_path=manifest_override,
        )

        emit("", run_log)
        emit("=== Summary ===", run_log)
        emit(f"Shards discovered: {len(shards)}", run_log)
        emit(f"Shards completed:  {completed_shards}", run_log)
        emit(f"Shards skipped:    {skipped_shards}", run_log)
        emit(f"Shards failed:     {len(failed_shards)}", run_log)
        emit(f"Fixture count:     {fixture_count}", run_log)
        emit(f"Region count:      {region_count}", run_log)
        emit(f"Run summary:       {summary_path}", run_log)
        emit(f"Manifest:          {output_root / 'manifest.json'}", run_log)

        if failed_shards:
            emit("", run_log)
            emit("Failed shards:", run_log, stream=sys.stderr)
            for result in failed_shards:
                detail = f"- {result.tag}/{result.chrom}: {result.error}"
                if result.log_path:
                    detail += f" ({result.log_path})"
                emit(detail, run_log, stream=sys.stderr)

    return 1 if any(result.status == "failed" for result in results) else 0


if __name__ == "__main__":
    sys.exit(main())