#!/usr/bin/env python3

from __future__ import annotations

import argparse
import concurrent.futures
import csv
import json
import os
import shlex
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import TextIO


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
    fixture_count: int = 0
    error: str = ""
    log_path: str = ""


@dataclass(frozen=True)
class ConfigPreset:
    name: str
    flags: tuple[str, ...]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Generate sweep parity fixtures in parallel.",
    )
    parser.add_argument(
        "--workers",
        type=positive_int,
        default=10,
        help="Maximum number of parallel shard workers (default: 10).",
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
        "--force",
        action="store_true",
        help="Regenerate shards even if the final output TSV already exists.",
    )
    parser.add_argument(
        "--output-only",
        action="store_true",
        help="Write only output TSVs and skip module JSONL fixture generation.",
    )
    parser.add_argument(
        "--config",
        help="Preset slug from scripts/config_presets.tsv to append to VarDict.",
    )
    parser.add_argument(
        "--sweep-bed-root",
        default=SWEEP_BED_ROOT_REL.as_posix(),
        help="Override sweep BED root for testing (default: tmp/sweep_beds).",
    )
    parser.add_argument(
        "--manifest-path",
        default=None,
        help="Override manifest.json output path (default: <output_root>/manifest.json). "
        "Use a per-invocation staging path when running multiple presets in parallel.",
    )
    parser.add_argument(
        "--chunk-size",
        type=positive_int,
        default=CHUNK_SIZE,
        help=f"Max tiles per VarDict invocation. Larger BEDs are split and compressed "
        f"between chunks to bound disk usage (default: {CHUNK_SIZE}).",
    )
    return parser


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
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


def resolve_path(root: Path, raw_path: str) -> Path:
    path = Path(raw_path)
    if not path.is_absolute():
        path = root / path
    return path.resolve()


def discover_shards(
    root: Path,
    tags: list[str],
    pair_tags: list[str],
    sweep_bed_root: Path,
) -> list[Shard]:
    shards: list[Shard] = []
    for tag in tags:
        tag_dir = sweep_bed_root / tag
        if not tag_dir.is_dir():
            raise SystemExit(f"ERROR: sweep BED directory not found for {tag}: {tag_dir}")
        bed_files = sorted(tag_dir.glob("*.bed"))
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
        bed_files = sorted(tag_dir.glob("*.bed"))
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


# Maximum tiles per VarDict invocation. Larger BEDs are split into chunks
# and compressed between chunks to bound uncompressed disk footprint.
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


def run_shard(
    shard: Shard,
    force: bool,
    output_only: bool,
    config_name: str | None,
    config_flags: tuple[str, ...],
    root_str: str,
    chunk_size: int = CHUNK_SIZE,
) -> ShardResult:
    root = Path(root_str)
    output_root = (root / OUTPUT_ROOT_REL).resolve()
    vardict_bin = (root / VARDICT_BIN_REL).resolve()
    effective_output_only = output_only or shard.kind == "pair"
    if shard.kind == "pair":
        tumor_rel, normal_rel = SOMATIC_PAIR_MAP[shard.tag]
        bam_arg = f"{(root / tumor_rel).resolve()}|{(root / normal_rel).resolve()}"
        reference = (root / SOMATIC_REF_MAP[shard.tag]).resolve()
    else:
        bam_arg = str((root / BAM_MAP[shard.tag]).resolve())
        reference = (root / REF_REL).resolve()
    output_file = output_dir(output_root, config_name, shard.chrom) / f"{shard.tag}_{shard.chrom}.tsv.zst"
    log_path = log_file_path(output_root, config_name, shard)

    if output_file.exists() and not force:
        return ShardResult(tag=shard.tag, chrom=shard.chrom, status="skipped")

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
    env = os.environ.copy()
    if not effective_output_only:
        for module_name, env_name in MODULES:
            env[env_name] = str(module_staging[module_name].resolve())

    tile_count = count_bed_lines(shard.bed_path)
    need_chunking = tile_count > chunk_size

    if need_chunking:
        chunk_beds = split_bed_chunks(shard.bed_path, chunk_size, output_staging)
    else:
        chunk_beds = [shard.bed_path]

    try:
        fixture_count = 0

        for chunk_idx, chunk_bed in enumerate(chunk_beds):
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
                *config_flags,
                str(chunk_bed),
            ]

            stdout_mode = "ab" if chunk_idx > 0 else "wb"
            stderr_mode = "ab" if chunk_idx > 0 else "wb"

            with stdout_path.open(stdout_mode) as stdout_handle, log_path.open(
                stderr_mode
            ) as stderr_handle:
                completed = subprocess.run(
                    command,
                    cwd=root,
                    env=env,
                    stdout=stdout_handle,
                    stderr=stderr_handle,
                    check=False,
                )

            if completed.returncode != 0:
                cleanup_dirs(staging_dirs)
                return ShardResult(
                    tag=shard.tag,
                    chrom=shard.chrom,
                    status="failed",
                    error=f"VarDict exited with code {completed.returncode} (chunk {chunk_idx})",
                    log_path=str(log_path),
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

        compress_paths([stdout_path])

        if not effective_output_only:
            for module_name, _ in MODULES:
                promote_files(
                    module_staging[module_name],
                    module_dir(output_root, module_name, config_name, shard.chrom),
                    "*.jsonl.zst",
                )
        promote_files(output_staging, output_dir(output_root, config_name, shard.chrom), "*.tsv.zst")

        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="success",
            fixture_count=fixture_count,
            log_path=str(log_path),
        )
    except Exception as exc:
        cleanup_dirs(staging_dirs)
        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="failed",
            error=str(exc),
            log_path=str(log_path),
        )


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
    completed = subprocess.run(
        ["git", "-C", str(root / "VarDictJava"), "rev-parse", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=True,
    )
    return completed.stdout.strip()


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
        "failed_shards": [f"{result.tag}/{result.chrom}" for result in failed_shards],
    }
    target_path = manifest_path if manifest_path is not None else output_root / "manifest.json"
    target_path.parent.mkdir(parents=True, exist_ok=True)
    with target_path.open("w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")


def emit(message: str, log_handle: TextIO, stream: TextIO = sys.stdout) -> None:
    print(message, file=stream, flush=True)
    log_handle.write(message + "\n")
    log_handle.flush()


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    selected_pair_tags = parse_pair_tags(args.pair_tags, parser)
    selected_tags = parse_tags(args.tags, parser, allow_empty=bool(selected_pair_tags))
    if not selected_tags and not selected_pair_tags:
        parser.error("at least one of --tags or --pair-tags must select a shard family")
    root = project_root()
    output_root = root / OUTPUT_ROOT_REL
    sweep_bed_root = resolve_path(root, args.sweep_bed_root)
    presets = load_all_presets(root)
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
    shards = discover_shards(root, selected_tags, selected_pair_tags, sweep_bed_root)
    region_bams = collect_region_bams(shards) if not run_output_only else {}

    with run_log_path.open("w", encoding="utf-8") as run_log:
        emit("=== sweep_fixtures_parallel ===", run_log)
        emit(f"Project root:  {root}", run_log)
        emit(f"Sweep BEDs:    {sweep_bed_root}", run_log)
        emit(f"Output root:   {output_root}", run_log)
        emit(f"BAM tags:      {','.join(selected_tags) if selected_tags else '(none)'}", run_log)
        emit(
            f"Pair tags:     {','.join(selected_pair_tags) if selected_pair_tags else '(none)'}",
            run_log,
        )
        emit(f"Workers:       {args.workers}", run_log)
        emit(f"Force:         {1 if args.force else 0}", run_log)
        emit(f"Mode:          {'output_only' if run_output_only else 'full'}", run_log)
        emit(f"Config:        {config_name or 'default'}", run_log)
        emit(f"Chunk size:    {args.chunk_size}", run_log)
        emit("", run_log)

        results: list[ShardResult] = []
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
                    args.chunk_size,
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

        failed_shards = [result for result in results if result.status == "failed"]
        skipped_shards = sum(1 for result in results if result.status == "skipped")
        completed_shards = sum(1 for result in results if result.status == "success")

        region_count = 0
        if not run_output_only:
            region_count = write_regions_tsv(output_root, region_bams)

        fixture_count = 0 if run_output_only else count_fixture_files(output_root)
        vardict_commit = get_vardict_commit(root)
        manifest_override = Path(args.manifest_path) if args.manifest_path else None
        if manifest_override is not None and not manifest_override.is_absolute():
            manifest_override = (root / manifest_override).resolve()
        write_manifest(
            output_root=output_root,
            selected_tags=selected_tags,
            selected_pair_tags=selected_pair_tags,
            shard_count=len(shards),
            failed_shards=failed_shards,
            vardict_commit=vardict_commit,
            region_count=region_count,
            fixture_count=fixture_count,
            mode="output_only" if run_output_only else "full",
            config_name=config_name,
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