#!/usr/bin/env python3

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
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
MODULES = (
    ("cigar_parser", "VARDICT_PARITY_CIGAR_PARSER"),
    ("realigner", "VARDICT_PARITY_REALIGNER"),
    ("sv_processor", "VARDICT_PARITY_SV_PROCESSOR"),
    ("tovars", "VARDICT_PARITY_TOVARS"),
)
OUTPUT_ROOT_REL = Path("tmp/sweep_fixtures")
RUN_LOG_REL = Path("tmp/sweep_fixtures_run.log")
SWEEP_BED_ROOT_REL = Path("tmp/sweep_beds")
REF_REL = Path("testdata/hs37d5.fa")
VARDICT_BIN_REL = Path("VarDictJava/build/install/VarDict/bin/VarDict")


@dataclass(frozen=True)
class Shard:
    tag: str
    chrom: str
    bed_path: Path


@dataclass(frozen=True)
class ShardResult:
    tag: str
    chrom: str
    status: str
    fixture_count: int = 0
    error: str = ""
    log_path: str = ""


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
        default=",".join(ALL_BAM_TAGS),
        help="Comma-separated BAM tags to process.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate shards even if the final output TSV already exists.",
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


def parse_tags(raw_tags: str, parser: argparse.ArgumentParser) -> list[str]:
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
        parser.error("no BAM tags selected")
    return selected


def project_root() -> Path:
    return Path(__file__).resolve().parent.parent


def discover_shards(root: Path, tags: list[str]) -> list[Shard]:
    shards: list[Shard] = []
    sweep_bed_root = root / SWEEP_BED_ROOT_REL
    for tag in tags:
        tag_dir = sweep_bed_root / tag
        if not tag_dir.is_dir():
            raise SystemExit(f"ERROR: sweep BED directory not found for {tag}: {tag_dir}")
        bed_files = sorted(tag_dir.glob("*.bed"))
        if not bed_files:
            raise SystemExit(f"ERROR: no BED files found for {tag}: {tag_dir}")
        for bed_file in bed_files:
            shards.append(Shard(tag=tag, chrom=bed_file.stem, bed_path=bed_file.resolve()))
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


def run_shard(shard: Shard, force: bool, root_str: str, chunk_size: int = CHUNK_SIZE) -> ShardResult:
    root = Path(root_str)
    output_root = (root / OUTPUT_ROOT_REL).resolve()
    vardict_bin = (root / VARDICT_BIN_REL).resolve()
    reference = (root / REF_REL).resolve()
    bam_path = (root / BAM_MAP[shard.tag]).resolve()
    output_file = output_root / "output" / shard.chrom / f"{shard.tag}_{shard.chrom}.tsv.zst"
    log_path = output_root / "logs" / shard.tag / f"{shard.chrom}.log"

    if output_file.exists() and not force:
        return ShardResult(tag=shard.tag, chrom=shard.chrom, status="skipped")

    module_staging: dict[str, Path] = {}
    staging_dirs: list[Path] = []
    for module_name, _ in MODULES:
        staging_dir = output_root / module_name / f"{shard.tag}_{shard.chrom}.staging"
        if staging_dir.exists():
            shutil.rmtree(staging_dir)
        staging_dir.mkdir(parents=True, exist_ok=True)
        module_staging[module_name] = staging_dir
        staging_dirs.append(staging_dir)

    output_staging = output_root / "output" / f"{shard.tag}_{shard.chrom}.staging"
    if output_staging.exists():
        shutil.rmtree(output_staging)
    output_staging.mkdir(parents=True, exist_ok=True)
    staging_dirs.append(output_staging)

    log_path.parent.mkdir(parents=True, exist_ok=True)
    stdout_path = output_staging / f"{shard.tag}_{shard.chrom}.tsv"
    env = os.environ.copy()
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
                str(bam_path),
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
            for module_name, _ in MODULES:
                jsonl_files = sorted(module_staging[module_name].glob("*.jsonl"))
                fixture_count += len(jsonl_files)
                compress_paths(jsonl_files)

            # Remove chunk BED file after use.
            if need_chunking:
                chunk_bed.unlink(missing_ok=True)

        compress_paths([stdout_path])

        for module_name, _ in MODULES:
            promote_files(
                module_staging[module_name],
                output_root / module_name / shard.chrom,
                "*.jsonl.zst",
            )
        promote_files(output_staging, output_root / "output" / shard.chrom, "*.tsv.zst")

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
    shard_count: int,
    failed_shards: list[ShardResult],
    vardict_commit: str,
    region_count: int,
    fixture_count: int,
) -> None:
    manifest = {
        "vardictjava_commit": vardict_commit,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "bam_tags": selected_tags,
        "bam_paths": {tag: BAM_MAP[tag] for tag in selected_tags},
        "region_count": region_count,
        "fixture_count": fixture_count,
        "shard_count": shard_count,
        "failed_shards": [f"{result.tag}/{result.chrom}" for result in failed_shards],
    }
    manifest_path = output_root / "manifest.json"
    with manifest_path.open("w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")


def emit(message: str, log_handle: TextIO, stream: TextIO = sys.stdout) -> None:
    print(message, file=stream, flush=True)
    log_handle.write(message + "\n")
    log_handle.flush()


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    selected_tags = parse_tags(args.tags, parser)
    root = project_root()
    output_root = root / OUTPUT_ROOT_REL
    output_root.mkdir(parents=True, exist_ok=True)
    run_log_path = root / RUN_LOG_REL
    run_log_path.parent.mkdir(parents=True, exist_ok=True)

    ensure_dependencies(root)
    shards = discover_shards(root, selected_tags)
    region_bams = collect_region_bams(shards)

    with run_log_path.open("w", encoding="utf-8") as run_log:
        emit("=== sweep_fixtures_parallel ===", run_log)
        emit(f"Project root:  {root}", run_log)
        emit(f"Sweep BEDs:    {root / SWEEP_BED_ROOT_REL}", run_log)
        emit(f"Output root:   {output_root}", run_log)
        emit(f"BAM tags:      {','.join(selected_tags)}", run_log)
        emit(f"Workers:       {args.workers}", run_log)
        emit(f"Force:         {1 if args.force else 0}", run_log)
        emit(f"Chunk size:    {args.chunk_size}", run_log)
        emit("", run_log)

        results: list[ShardResult] = []
        with concurrent.futures.ProcessPoolExecutor(max_workers=args.workers) as executor:
            future_map = {
                executor.submit(run_shard, shard, args.force, str(root), args.chunk_size): shard for shard in shards
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

        region_count = write_regions_tsv(output_root, region_bams)
        fixture_count = count_fixture_files(output_root)
        vardict_commit = get_vardict_commit(root)
        write_manifest(
            output_root=output_root,
            selected_tags=selected_tags,
            shard_count=len(shards),
            failed_shards=failed_shards,
            vardict_commit=vardict_commit,
            region_count=region_count,
            fixture_count=fixture_count,
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