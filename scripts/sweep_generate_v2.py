#!/usr/bin/env python3

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path


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
    ("cigar_modifier", "VARDICT_PARITY_CIGAR_MODIFIER"),
    ("sam_file_parser", "VARDICT_PARITY_SAM_FILE_PARSER"),
)
DEFAULT_OUTPUT_DIR = Path("tmp/sweep_fixtures")
DEFAULT_SWEEP_BED_ROOT = Path("tmp/sweep_beds")
DEFAULT_REF = Path("testdata/hs37d5.fa")
DEFAULT_VARDICT_BIN = Path("VarDictJava/build/install/VarDict/bin/VarDict")
TEMP_ROOT = Path("tmp/sweep_generate_v2")


@dataclass(frozen=True)
class Tile:
    index: int
    chrom: str
    start: int
    end: int

    @property
    def region_key(self) -> tuple[str, int, int]:
        return (self.chrom, self.start, self.end)


@dataclass(frozen=True)
class Chunk:
    index: int
    bed_path: Path
    tiles: list[Tile]


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
    total_tiles: int = 0
    chunk_count: int = 0
    module_stats: dict[str, dict[str, int | str]] = field(default_factory=dict)
    archive_paths: dict[str, str] = field(default_factory=dict)
    warnings: list[str] = field(default_factory=list)
    error: str = ""
    log_path: str = ""


class ArchiveWriter:
    def __init__(self, final_path: Path, temp_root: Path | None = None) -> None:
        self.final_path = final_path
        # Stage .tmp next to final_path (same device) so os.replace() works atomically.
        # Archive writes are bulk sequential I/O, acceptable even on slower devices.
        self.final_path.parent.mkdir(parents=True, exist_ok=True)
        self.temp_path = final_path.with_name(final_path.name + ".tmp")
        self.process = subprocess.Popen(
            ["zstd", "-qf", "-o", str(self.temp_path)],
            stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

    def write_line(self, line: str) -> None:
        if self.process.stdin is None:
            raise RuntimeError(f"zstd stdin closed for {self.final_path}")
        self.process.stdin.write(line.encode("utf-8"))

    def close(self) -> int:
        if self.process.stdin is not None and not self.process.stdin.closed:
            self.process.stdin.close()
        stderr = self.process.stderr.read() if self.process.stderr is not None else b""
        returncode = self.process.wait()
        if returncode != 0:
            raise RuntimeError(
                f"zstd failed for {self.final_path}: {stderr.decode('utf-8', errors='replace').strip()}"
            )
        os.replace(self.temp_path, self.final_path)
        return self.final_path.stat().st_size

    def abort(self) -> None:
        if self.process.stdin is not None and not self.process.stdin.closed:
            self.process.stdin.close()
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.temp_path.unlink(missing_ok=True)


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Generate production v2 sweep fixture archives from VarDictJava parity output.",
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
        help="Comma-separated BAM tags to process (default: all tags).",
    )
    parser.add_argument(
        "--output-dir",
        default=str(DEFAULT_OUTPUT_DIR),
        help="Base output directory for v2 sweep fixtures (default: tmp/sweep_fixtures).",
    )
    parser.add_argument(
        "--chunk-size",
        type=positive_int,
        default=20_000,
        help="Maximum number of BED tiles per VarDict invocation (default: 20000).",
    )
    parser.add_argument(
        "--temp-dir",
        default=None,
        help="Directory for intermediate staging files (default: <output-dir>/.tmp). Must be on the same filesystem as --output-dir.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate existing shard archives.",
    )
    return parser


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


def resolve_path(root: Path, raw_path: str) -> Path:
    path = Path(raw_path)
    if path.is_absolute():
        return path
    return (root / path).resolve()


def ensure_binary(path: Path, label: str) -> None:
    if not path.is_file() or not os.access(path, os.X_OK):
        raise SystemExit(f"ERROR: {label} not found or not executable: {path}")


def ensure_dependencies(root: Path) -> None:
    if shutil.which("zstd") is None:
        raise SystemExit("ERROR: zstd not found in PATH")
    ensure_binary((root / DEFAULT_VARDICT_BIN).resolve(), "VarDictJava binary")
    ref_path = (root / DEFAULT_REF).resolve()
    if not ref_path.is_file():
        raise SystemExit(f"ERROR: reference FASTA not found: {ref_path}")
    for tag, bam_rel in BAM_MAP.items():
        bam_path = (root / bam_rel).resolve()
        if not bam_path.is_file():
            raise SystemExit(f"ERROR: BAM file not found for {tag}: {bam_path}")


def discover_shards(root: Path, tags: list[str]) -> list[Shard]:
    shards: list[Shard] = []
    sweep_bed_root = root / DEFAULT_SWEEP_BED_ROOT
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


def count_bed_lines(bed_path: Path) -> int:
    count = 0
    with bed_path.open("r", encoding="utf-8") as handle:
        for _ in handle:
            count += 1
    return count


def prepare_chunks(bed_path: Path, chunk_size: int, chunks_dir: Path) -> list[Chunk]:
    chunks_dir.mkdir(parents=True, exist_ok=True)
    chunks: list[Chunk] = []
    chunk_index = 0
    tile_index = 0
    chunk_tiles: list[Tile] = []
    chunk_handle = None
    chunk_path: Path | None = None

    try:
        with bed_path.open("r", encoding="utf-8") as source:
            for raw_line in source:
                line = raw_line.rstrip("\n")
                if not line:
                    continue
                fields = line.split("\t")
                if len(fields) < 3:
                    raise RuntimeError(f"malformed BED line: {line}")
                chrom = fields[0]
                start = int(fields[1])
                end = int(fields[2])

                if len(chunk_tiles) == 0:
                    chunk_path = chunks_dir / f"chunk_{chunk_index:04d}.bed"
                    chunk_handle = chunk_path.open("w", encoding="utf-8")

                assert chunk_handle is not None
                assert chunk_path is not None

                chunk_handle.write(raw_line)
                chunk_tiles.append(Tile(index=tile_index, chrom=chrom, start=start, end=end))
                tile_index += 1

                if len(chunk_tiles) == chunk_size:
                    chunk_handle.close()
                    chunks.append(Chunk(index=chunk_index, bed_path=chunk_path, tiles=chunk_tiles))
                    chunk_index += 1
                    chunk_tiles = []
                    chunk_handle = None
                    chunk_path = None

        if chunk_handle is not None and chunk_path is not None:
            chunk_handle.close()
            chunks.append(Chunk(index=chunk_index, bed_path=chunk_path, tiles=chunk_tiles))
    finally:
        if chunk_handle is not None:
            chunk_handle.close()

    return chunks


def parse_fixture_name(module_name: str, file_name: str) -> tuple[str, int, int]:
    prefix = f"{module_name}_"
    suffix = ".jsonl"
    if not file_name.startswith(prefix) or not file_name.endswith(suffix):
        raise ValueError(f"unexpected fixture filename: {file_name}")
    remainder = file_name[len(prefix) : -len(suffix)]
    chrom, start, end = remainder.rsplit("_", 2)
    return chrom, int(start), int(end)


def read_second_line(file_path: Path) -> str:
    with file_path.open("r", encoding="utf-8") as handle:
        handle.readline()
        second_line = handle.readline()
    if not second_line:
        raise ValueError(f"fixture missing line 2: {file_path}")
    return second_line.rstrip("\n")


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
    output_dir: Path,
    selected_tags: list[str],
    results: list[ShardResult],
    workers: int,
    chunk_size: int,
    vardict_commit: str,
) -> None:
    manifest = {
        "vardictjava_commit": vardict_commit,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "bam_tags": selected_tags,
        "bam_paths": {tag: BAM_MAP[tag] for tag in selected_tags},
        "workers": workers,
        "chunk_size": chunk_size,
        "module_names": [module_name for module_name, _ in MODULES],
        "shard_count": len(results),
        "completed_shards": sum(1 for result in results if result.status == "success"),
        "skipped_shards": sum(1 for result in results if result.status == "skipped"),
        "failed_shards": [
            {
                "tag": result.tag,
                "chrom": result.chrom,
                "error": result.error,
                "log_path": result.log_path,
            }
            for result in results
            if result.status == "failed"
        ],
        "shards": [
            {
                "tag": result.tag,
                "chrom": result.chrom,
                "status": result.status,
                "total_tiles": result.total_tiles,
                "chunk_count": result.chunk_count,
                "archive_paths": result.archive_paths,
                "modules": result.module_stats,
                "warnings": result.warnings,
                "log_path": result.log_path,
            }
            for result in sorted(results, key=lambda item: (item.tag, item.chrom))
        ],
    }
    manifest_path = output_dir / "manifest.json"
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    with manifest_path.open("w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")


def build_archive_paths(output_dir: Path, shard: Shard) -> dict[str, Path]:
    return {
        module_name: output_dir / "v2" / module_name / shard.tag / f"{shard.chrom}.jsonl.zst"
        for module_name, _ in MODULES
    }


def cleanup_dir(path: Path) -> None:
    if path.exists():
        shutil.rmtree(path, ignore_errors=True)


def run_shard(shard: Shard, root_str: str, output_dir_str: str, temp_dir_str: str, chunk_size: int, force: bool) -> ShardResult:
    root = Path(root_str)
    output_dir = Path(output_dir_str)
    temp_base = Path(temp_dir_str)
    vardict_bin = (root / DEFAULT_VARDICT_BIN).resolve()
    ref_path = (root / DEFAULT_REF).resolve()
    bam_path = (root / BAM_MAP[shard.tag]).resolve()
    log_path = output_dir / "logs" / shard.tag / f"{shard.chrom}.log"
    archive_paths = build_archive_paths(output_dir, shard)

    if all(path.exists() for path in archive_paths.values()) and not force:
        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="skipped",
            archive_paths={module_name: str(path) for module_name, path in archive_paths.items()},
            log_path=str(log_path),
        )

    if force:
        for archive_path in archive_paths.values():
            archive_path.unlink(missing_ok=True)
        log_path.unlink(missing_ok=True)

    shard_temp = (temp_base / shard.tag / shard.chrom).resolve()
    cleanup_dir(shard_temp)
    chunks_dir = shard_temp / "chunks"
    staging_root = shard_temp / "staging"
    archive_temp_root = shard_temp / "archives"

    total_tiles = count_bed_lines(shard.bed_path)
    if total_tiles == 0:
        return ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="failed",
            error=f"BED file is empty: {shard.bed_path}",
            log_path=str(log_path),
        )

    chunks = prepare_chunks(shard.bed_path, chunk_size, chunks_dir)
    module_stats = {
        module_name: {
            "emitted_tiles": 0,
            "missing_tiles": 0,
            "unexpected_files": 0,
            "archive_path": str(archive_paths[module_name]),
        }
        for module_name, _ in MODULES
    }
    warnings: list[str] = []
    archive_writers = {
        module_name: ArchiveWriter(final_path=archive_paths[module_name], temp_root=archive_temp_root)
        for module_name, _ in MODULES
    }
    run_succeeded = False
    log_path.parent.mkdir(parents=True, exist_ok=True)
    start_time = time.monotonic()

    try:
        for chunk_index, chunk in enumerate(chunks):
            # Debug: verify chunk BED exists before processing
            if not chunk.bed_path.exists():
                raise RuntimeError(
                    f"chunk BED does not exist BEFORE processing: {chunk.bed_path} "
                    f"(chunk_index={chunk_index}, chunks_dir contents: "
                    f"{sorted(p.name for p in chunks_dir.iterdir()) if chunks_dir.exists() else 'DIR_MISSING'})"
                )
            chunk_staging = staging_root / f"chunk_{chunk.index:04d}"
            cleanup_dir(chunk_staging)
            chunk_staging.mkdir(parents=True, exist_ok=True)

            env = os.environ.copy()
            module_dirs: dict[str, Path] = {}
            for module_name, env_var in MODULES:
                module_dir = chunk_staging / module_name
                module_dir.mkdir(parents=True, exist_ok=True)
                module_dirs[module_name] = module_dir
                env[env_var] = str(module_dir.resolve())

            command = [
                str(vardict_bin),
                "-G",
                str(ref_path),
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
                str(chunk.bed_path),
            ]

            stderr_mode = "ab" if chunk_index > 0 else "wb"
            with log_path.open(stderr_mode) as stderr_handle:
                completed = subprocess.run(
                    command,
                    cwd=root,
                    env=env,
                    stdout=subprocess.DEVNULL,
                    stderr=stderr_handle,
                    check=False,
                )

            if completed.returncode != 0:
                raise RuntimeError(
                    f"VarDict exited with code {completed.returncode} for chunk {chunk.index}; see {log_path}"
                )

            expected_regions = {tile.region_key for tile in chunk.tiles}
            for module_name, _ in MODULES:
                produced: dict[tuple[str, int, int], Path] = {}
                for jsonl_path in sorted(module_dirs[module_name].glob("*.jsonl")):
                    produced[parse_fixture_name(module_name, jsonl_path.name)] = jsonl_path

                for tile in chunk.tiles:
                    jsonl_path = produced.pop(tile.region_key, None)
                    if jsonl_path is None:
                        module_stats[module_name]["missing_tiles"] += 1
                        continue

                    line2 = read_second_line(jsonl_path)
                    archive_writers[module_name].write_line(
                        f"{tile.chrom}\t{tile.start}\t{tile.end}\t{line2}\n"
                    )
                    module_stats[module_name]["emitted_tiles"] += 1
                    jsonl_path.unlink()

                if produced:
                    module_stats[module_name]["unexpected_files"] += len(produced)
                    for region_key, leftover_path in produced.items():
                        if region_key not in expected_regions:
                            warnings.append(
                                f"{module_name}: unexpected tile {region_key[0]}:{region_key[1]}-{region_key[2]} in chunk {chunk.index}"
                            )
                        leftover_path.unlink(missing_ok=True)

                cleanup_dir(module_dirs[module_name])

            cleanup_dir(chunk_staging)
            chunk.bed_path.unlink(missing_ok=True)

        for module_name, writer in archive_writers.items():
            module_stats[module_name]["archive_bytes"] = writer.close()

        run_succeeded = True
    finally:
        if not run_succeeded:
            for writer in archive_writers.values():
                writer.abort()
        cleanup_dir(staging_root)
        cleanup_dir(chunks_dir)
        cleanup_dir(archive_temp_root)
        cleanup_dir(shard_temp)

    elapsed_seconds = round(time.monotonic() - start_time, 3)
    if warnings:
        warnings.append(f"elapsed_seconds={elapsed_seconds}")

    return ShardResult(
        tag=shard.tag,
        chrom=shard.chrom,
        status="success",
        total_tiles=total_tiles,
        chunk_count=len(chunks),
        module_stats=module_stats,
        archive_paths={module_name: str(path) for module_name, path in archive_paths.items()},
        warnings=warnings,
        log_path=str(log_path),
    )


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    selected_tags = parse_tags(args.tags, parser)
    root = project_root()
    output_dir = resolve_path(root, args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    temp_dir = resolve_path(root, Path(args.temp_dir)) if args.temp_dir else resolve_path(root, TEMP_ROOT)
    temp_dir.mkdir(parents=True, exist_ok=True)

    ensure_dependencies(root)
    shards = discover_shards(root, selected_tags)

    print(f"Discovered {len(shards)} shards across {len(selected_tags)} BAM tags", flush=True)
    results: list[ShardResult] = []

    with concurrent.futures.ProcessPoolExecutor(max_workers=args.workers) as executor:
        future_map = {
            executor.submit(run_shard, shard, str(root), str(output_dir), str(temp_dir), args.chunk_size, args.force): shard
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
                    log_path=str(output_dir / "logs" / shard.tag / f"{shard.chrom}.log"),
                )
            results.append(result)
            if result.status == "success":
                emitted = sum(
                    int(module_stats.get("emitted_tiles", 0))
                    for module_stats in result.module_stats.values()
                )
                print(
                    f"[{result.tag}] {result.chrom}: {result.chunk_count} chunks, {emitted} archive lines",
                    flush=True,
                )
            elif result.status == "skipped":
                print(f"[{result.tag}] {result.chrom}: skipped", flush=True)
            else:
                detail = f"[{result.tag}] {result.chrom}: FAILED: {result.error}"
                if result.log_path:
                    detail += f" ({result.log_path})"
                print(detail, file=sys.stderr, flush=True)

    results.sort(key=lambda item: (item.tag, item.chrom))
    vardict_commit = get_vardict_commit(root)
    write_manifest(
        output_dir=output_dir,
        selected_tags=selected_tags,
        results=results,
        workers=args.workers,
        chunk_size=args.chunk_size,
        vardict_commit=vardict_commit,
    )

    failed_count = sum(1 for result in results if result.status == "failed")
    skipped_count = sum(1 for result in results if result.status == "skipped")
    success_count = sum(1 for result in results if result.status == "success")
    print(f"Completed shards: {success_count}", flush=True)
    print(f"Skipped shards:   {skipped_count}", flush=True)
    print(f"Failed shards:    {failed_count}", flush=True)
    print(f"Manifest:         {output_dir / 'manifest.json'}", flush=True)
    return 1 if failed_count else 0


if __name__ == "__main__":
    sys.exit(main())