#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


MODULES = (
    ("cigar_parser", "VARDICT_PARITY_CIGAR_PARSER"),
    ("realigner", "VARDICT_PARITY_REALIGNER"),
    ("sv_processor", "VARDICT_PARITY_SV_PROCESSOR"),
    ("tovars", "VARDICT_PARITY_TOVARS"),
)
DEFAULT_OUTPUT_DIR = Path("tmp/pilot")
DEFAULT_VARDICT_BIN = Path("VarDictJava/build/install/VarDict/bin/VarDict")


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


class ArchiveWriter:
    def __init__(self, final_path: Path, temp_root: Path) -> None:
        self.final_path = final_path
        self.temp_path = temp_root / final_path.parent.parent.name / final_path.parent.name / f"{final_path.name}.tmp"
        self.temp_path.parent.mkdir(parents=True, exist_ok=True)
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
        self.final_path.parent.mkdir(parents=True, exist_ok=True)
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
        description="Generate pilot v2 archives from VarDictJava per-tile parity output.",
    )
    parser.add_argument("--bed", required=True, help="Input BED file.")
    parser.add_argument("--bam", required=True, help="Input BAM file.")
    parser.add_argument("--ref", required=True, help="Reference FASTA file.")
    parser.add_argument("--tag", required=True, help="BAM tag used in output layout.")
    parser.add_argument("--chrom", required=True, help="Chromosome label used in output layout.")
    parser.add_argument(
        "--chunk-size",
        type=positive_int,
        default=20_000,
        help="Maximum number of BED tiles per VarDict invocation (default: 20000).",
    )
    parser.add_argument(
        "--output-dir",
        default=str(DEFAULT_OUTPUT_DIR),
        help="Base output directory for pilot artifacts (default: tmp/pilot).",
    )
    parser.add_argument(
        "--vardict-bin",
        default=str(DEFAULT_VARDICT_BIN),
        help="Path to the VarDictJava binary (default: VarDictJava/build/install/VarDict/bin/VarDict).",
    )
    return parser


def project_root() -> Path:
    return Path(__file__).resolve().parent.parent


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
                    raise SystemExit(f"ERROR: malformed BED line: {line}")
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


def ensure_binary(path: Path, label: str) -> None:
    if not path.is_file() or not os.access(path, os.X_OK):
        raise SystemExit(f"ERROR: {label} not found or not executable: {path}")


def reset_run_outputs(output_dir: Path, tag: str, chrom: str) -> None:
    for module_name, _ in MODULES:
        archive_path = output_dir / "v2" / module_name / tag / f"{chrom}.jsonl.zst"
        archive_path.unlink(missing_ok=True)

        sample_dir = output_dir / "samples" / module_name
        if sample_dir.is_dir():
            shutil.rmtree(sample_dir)

    tmp_dir = output_dir / "tmp"
    if tmp_dir.is_dir():
        shutil.rmtree(tmp_dir)


def save_summary(summary_path: Path, payload: dict[str, object]) -> None:
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    with summary_path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2)
        handle.write("\n")


def main() -> int:
    args = build_parser().parse_args()
    root = project_root()
    bed_path = (root / args.bed).resolve() if not Path(args.bed).is_absolute() else Path(args.bed)
    bam_path = (root / args.bam).resolve() if not Path(args.bam).is_absolute() else Path(args.bam)
    ref_path = (root / args.ref).resolve() if not Path(args.ref).is_absolute() else Path(args.ref)
    vardict_bin = (
        (root / args.vardict_bin).resolve()
        if not Path(args.vardict_bin).is_absolute()
        else Path(args.vardict_bin)
    )
    output_dir = (
        (root / args.output_dir).resolve()
        if not Path(args.output_dir).is_absolute()
        else Path(args.output_dir)
    )

    for required_path, label in ((bed_path, "BED file"), (bam_path, "BAM file"), (ref_path, "Reference FASTA")):
        if not required_path.is_file():
            raise SystemExit(f"ERROR: {label} not found: {required_path}")

    ensure_binary(vardict_bin, "VarDictJava binary")
    if shutil.which("zstd") is None:
        raise SystemExit("ERROR: zstd not found in PATH")

    total_tiles = count_bed_lines(bed_path)
    if total_tiles == 0:
        raise SystemExit(f"ERROR: BED file is empty: {bed_path}")

    sample_targets = {0, total_tiles // 2, total_tiles - 1}
    started_at = datetime.now(timezone.utc)
    start_time = time.monotonic()

    reset_run_outputs(output_dir, args.tag, args.chrom)

    tmp_dir = output_dir / "tmp"
    chunks_dir = tmp_dir / "chunks"
    staging_root = tmp_dir / "staging"
    logs_dir = output_dir / "logs"
    chunks = prepare_chunks(bed_path, args.chunk_size, chunks_dir)

    sample_dirs = {module_name: output_dir / "samples" / module_name for module_name, _ in MODULES}
    for sample_dir in sample_dirs.values():
        sample_dir.mkdir(parents=True, exist_ok=True)

    summary_modules = {
        module_name: {
            "emitted_tiles": 0,
            "missing_tiles": 0,
            "unexpected_files": 0,
            "samples_saved": 0,
            "sample_files": [],
            "archive_path": str(output_dir / "v2" / module_name / args.tag / f"{args.chrom}.jsonl.zst"),
        }
        for module_name, _ in MODULES
    }
    warnings: list[str] = []

    archive_writers = {
        module_name: ArchiveWriter(
            final_path=output_dir / "v2" / module_name / args.tag / f"{args.chrom}.jsonl.zst",
            temp_root=tmp_dir / "archives",
        )
        for module_name, _ in MODULES
    }
    run_succeeded = False

    try:
        tiles_processed = 0
        for chunk_position, chunk in enumerate(chunks, start=1):
            chunk_staging = staging_root / f"chunk_{chunk.index:04d}"
            if chunk_staging.exists():
                shutil.rmtree(chunk_staging)
            chunk_staging.mkdir(parents=True, exist_ok=True)

            env = os.environ.copy()
            module_dirs: dict[str, Path] = {}
            for module_name, env_var in MODULES:
                module_dir = chunk_staging / module_name
                module_dir.mkdir(parents=True, exist_ok=True)
                module_dirs[module_name] = module_dir
                env[env_var] = str(module_dir.resolve())

            log_path = logs_dir / f"chunk_{chunk.index:04d}.stderr.log"
            log_path.parent.mkdir(parents=True, exist_ok=True)
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

            with log_path.open("wb") as stderr_handle:
                completed = subprocess.run(
                    command,
                    cwd=root,
                    env=env,
                    stdout=subprocess.DEVNULL,
                    stderr=stderr_handle,
                    check=False,
                )

            if completed.returncode != 0:
                raise SystemExit(
                    f"ERROR: VarDict failed for chunk {chunk.index} with exit code {completed.returncode}. "
                    f"See {log_path}"
                )

            expected_regions = {tile.region_key for tile in chunk.tiles}
            for module_name, _ in MODULES:
                produced: dict[tuple[str, int, int], Path] = {}
                for jsonl_path in sorted(module_dirs[module_name].glob("*.jsonl")):
                    region_key = parse_fixture_name(module_name, jsonl_path.name)
                    produced[region_key] = jsonl_path

                for tile in chunk.tiles:
                    jsonl_path = produced.pop(tile.region_key, None)
                    if jsonl_path is None:
                        summary_modules[module_name]["missing_tiles"] += 1
                        continue

                    line2 = read_second_line(jsonl_path)
                    archive_writers[module_name].write_line(
                        f"{tile.chrom}\t{tile.start}\t{tile.end}\t{line2}\n"
                    )
                    summary_modules[module_name]["emitted_tiles"] += 1

                    if tile.index in sample_targets:
                        sample_path = sample_dirs[module_name] / jsonl_path.name
                        shutil.copy2(jsonl_path, sample_path)
                        summary_modules[module_name]["samples_saved"] += 1
                        summary_modules[module_name]["sample_files"].append(str(sample_path))

                    jsonl_path.unlink()

                if produced:
                    summary_modules[module_name]["unexpected_files"] += len(produced)
                    for region_key, leftover_path in produced.items():
                        if region_key not in expected_regions:
                            warnings.append(
                                f"{module_name}: unexpected tile {region_key[0]}:{region_key[1]}-{region_key[2]} in chunk {chunk.index}"
                            )
                        leftover_path.unlink(missing_ok=True)

                if summary_modules[module_name]["missing_tiles"]:
                    warnings_count = summary_modules[module_name]["missing_tiles"]
                    if chunk_position == len(chunks):
                        warnings.append(f"{module_name}: missing fixtures for {warnings_count} tiles total")

                shutil.rmtree(module_dirs[module_name], ignore_errors=True)

            shutil.rmtree(chunk_staging, ignore_errors=True)
            chunk.bed_path.unlink(missing_ok=True)

            tiles_processed += len(chunk.tiles)
            print(
                f"Chunk {chunk_position}/{len(chunks)}: {tiles_processed}/{total_tiles} tiles",
                flush=True,
            )

        for module_name, writer in archive_writers.items():
            summary_modules[module_name]["archive_bytes"] = writer.close()

        run_succeeded = True
    finally:
        if not run_succeeded:
            for writer in archive_writers.values():
                writer.abort()

    completed_at = datetime.now(timezone.utc)
    elapsed_seconds = round(time.monotonic() - start_time, 3)
    summary = {
        "started_at": started_at.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "completed_at": completed_at.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "elapsed_seconds": elapsed_seconds,
        "bed": str(bed_path),
        "bam": str(bam_path),
        "ref": str(ref_path),
        "vardict_bin": str(vardict_bin),
        "tag": args.tag,
        "chrom": args.chrom,
        "chunk_size": args.chunk_size,
        "chunk_count": len(chunks),
        "total_tiles": total_tiles,
        "sample_target_indices": sorted(sample_targets),
        "modules": summary_modules,
        "warnings": warnings,
    }
    save_summary(output_dir / "generation_summary.json", summary)

    print(
        f"Completed {len(chunks)} chunks in {elapsed_seconds:.3f}s; summary written to {output_dir / 'generation_summary.json'}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
