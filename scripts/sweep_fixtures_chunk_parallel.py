#!/usr/bin/env python3

from __future__ import annotations

import concurrent.futures
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path


if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from scripts import sweep_fixtures_parallel as base


_ORIGINAL_BUILD_PARSER = base.build_parser
_ORIGINAL_EXECUTE_SHARD_RUN = base.execute_shard_run
CLI_CHUNK_WORKERS = 1


@dataclass(frozen=True)
class ChunkWorkItem:
    chunk_idx: int
    chunk_bed: Path
    stdout_path: Path
    stderr_path: Path
    command: tuple[str, ...]
    num_tiles: int


@dataclass(frozen=True)
class ChunkRunResult:
    chunk_idx: int
    stdout_path: Path
    stderr_path: Path
    num_tiles: int
    returncode: int
    wall_s: float
    md5_raw: str
    byte_count: int


def build_parser():
    parser = _ORIGINAL_BUILD_PARSER()
    parser.add_argument(
        "--chunk-workers",
        type=base.positive_int,
        default=1,
        help=(
            "Maximum parallel workers for chunked output-only shard execution. "
            "Chunk parallelism only activates when output-only mode is effective, "
            "chunking is active, and this value is greater than 1 (default: 1)."
        ),
    )
    return parser


def chunk_parallelism_enabled(
    *,
    effective_output_only: bool,
    need_chunking: bool,
    chunk_workers: int,
) -> bool:
    return effective_output_only and need_chunking and chunk_workers > 1


def allocate_chunk_stdout_path(output_staging: Path, stdout_stem: str, chunk_idx: int) -> Path:
    fd, raw_path = tempfile.mkstemp(
        dir=output_staging,
        prefix=f"{stdout_stem}.chunk_{chunk_idx:04d}.",
        suffix=".tsv",
    )
    os.close(fd)
    return Path(raw_path)


def chunk_stderr_path(output_staging: Path, stdout_stem: str, chunk_idx: int) -> Path:
    return output_staging / f"{stdout_stem}.chunk_{chunk_idx:04d}.stderr.log"


def build_chunk_work_items(
    *,
    chunk_beds: list[Path],
    shard: base.Shard,
    output_staging: Path,
    stdout_path: Path,
    vardict_bin: Path,
    reference: Path,
    bam_arg: str,
    config_flags: tuple[str, ...],
    sample_name_override: str | None,
    total_bed_lines: int,
    need_chunking: bool,
) -> list[ChunkWorkItem]:
    work_items: list[ChunkWorkItem] = []
    for chunk_idx, chunk_bed in enumerate(chunk_beds):
        num_tiles = base.count_bed_lines(chunk_bed) if need_chunking else total_bed_lines
        command = [
            str(vardict_bin),
            "-G",
            str(reference),
            "-b",
            bam_arg,
            *base.default_thread_flags(config_flags),
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
        work_items.append(
            ChunkWorkItem(
                chunk_idx=chunk_idx,
                chunk_bed=chunk_bed,
                stdout_path=allocate_chunk_stdout_path(output_staging, stdout_path.stem, chunk_idx),
                stderr_path=chunk_stderr_path(output_staging, stdout_path.stem, chunk_idx),
                command=tuple(command),
                num_tiles=num_tiles,
            )
        )
    return work_items


def terminate_active_chunk_processes(
    active_processes: dict[int, subprocess.Popen[bytes]],
    process_lock: threading.Lock,
) -> None:
    with process_lock:
        processes = list(active_processes.values())

    for process in processes:
        if process.poll() is None:
            process.terminate()
    for process in processes:
        if process.poll() is not None:
            continue
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
    for process in processes:
        if process.poll() is None:
            process.wait()


def execute_chunk_work_item(
    *,
    item: ChunkWorkItem,
    root: Path,
    env: dict[str, str],
    active_processes: dict[int, subprocess.Popen[bytes]],
    process_lock: threading.Lock,
) -> ChunkRunResult:
    t_chunk_start = time.monotonic()
    with item.stdout_path.open("wb") as stdout_handle, item.stderr_path.open("wb") as stderr_handle:
        process = subprocess.Popen(
            list(item.command),
            cwd=root,
            env=env,
            stdout=stdout_handle,
            stderr=stderr_handle,
        )
        with process_lock:
            active_processes[item.chunk_idx] = process
        try:
            returncode = process.wait()
        finally:
            with process_lock:
                active_processes.pop(item.chunk_idx, None)

    wall_s = time.monotonic() - t_chunk_start
    if returncode != 0:
        return ChunkRunResult(
            chunk_idx=item.chunk_idx,
            stdout_path=item.stdout_path,
            stderr_path=item.stderr_path,
            num_tiles=item.num_tiles,
            returncode=returncode,
            wall_s=wall_s,
            md5_raw="",
            byte_count=item.stdout_path.stat().st_size if item.stdout_path.exists() else 0,
        )

    return ChunkRunResult(
        chunk_idx=item.chunk_idx,
        stdout_path=item.stdout_path,
        stderr_path=item.stderr_path,
        num_tiles=item.num_tiles,
        returncode=returncode,
        wall_s=wall_s,
        md5_raw=base.compute_file_md5(item.stdout_path),
        byte_count=item.stdout_path.stat().st_size,
    )


def merge_chunk_outputs(
    *,
    ordered_results: list[ChunkRunResult],
    stdout_path: Path,
) -> list[dict[str, object]]:
    chunk_records: list[dict[str, object]] = []
    raw_offset = 0
    with stdout_path.open("wb") as stdout_handle:
        for result in ordered_results:
            with result.stdout_path.open("rb") as chunk_handle:
                shutil.copyfileobj(chunk_handle, stdout_handle)
            chunk_records.append(
                {
                    "idx": result.chunk_idx,
                    "md5_raw": result.md5_raw,
                    "wall_s": result.wall_s,
                    "num_tiles": result.num_tiles,
                    "byte_range": [raw_offset, result.byte_count],
                }
            )
            raw_offset += result.byte_count
    return chunk_records


def combine_chunk_logs(log_path: Path, ordered_results: list[ChunkRunResult]) -> None:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("wb") as merged_log:
        for result in ordered_results:
            if not result.stderr_path.exists():
                continue
            with result.stderr_path.open("rb") as chunk_log:
                shutil.copyfileobj(chunk_log, merged_log)


def write_chunk_parallel_failure_log(
    *,
    log_path: Path,
    error: str,
    failed_chunk_idx: int | None,
    output_staging: Path,
) -> None:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w", encoding="utf-8") as handle:
        handle.write(f"chunk-parallel execution failed: {error}\n")
        if failed_chunk_idx is not None:
            handle.write(f"failed_chunk_idx={failed_chunk_idx}\n")
        handle.write(f"preserved_artifacts={output_staging}\n")


def cleanup_chunk_success_artifacts(
    *,
    chunk_beds: list[Path],
    ordered_results: list[ChunkRunResult],
) -> None:
    for chunk_bed in chunk_beds:
        chunk_bed.unlink(missing_ok=True)
    for result in ordered_results:
        result.stdout_path.unlink(missing_ok=True)
        result.stderr_path.unlink(missing_ok=True)


def execute_shard_run(
    shard: base.Shard,
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
    chunk_workers: int | None = None,
) -> base.ShardResult:
    resolved_chunk_workers = CLI_CHUNK_WORKERS if chunk_workers is None else chunk_workers
    effective_output_only = output_only or shard.kind == "pair"
    total_bed_lines = base.count_bed_lines(shard.bed_path)
    need_chunking = chunk_size > 0 and total_bed_lines > chunk_size

    if not chunk_parallelism_enabled(
        effective_output_only=effective_output_only,
        need_chunking=need_chunking,
        chunk_workers=resolved_chunk_workers,
    ):
        return _ORIGINAL_EXECUTE_SHARD_RUN(
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
            zstd_executor=zstd_executor,
            pending_compressions=pending_compressions,
            sample_name_override=sample_name_override,
        )

    t_shard_start = time.monotonic()
    vardict_bin = (root / base.VARDICT_BIN_REL).resolve()
    log_path = base.log_file_path(output_root, config_name, shard)

    resume_result = base.resume_or_cleanup_shard(
        shard=shard,
        force=force,
        output_root=output_root,
        config_name=config_name,
        shard_start=t_shard_start,
    )
    if resume_result is not None:
        return resume_result

    output_staging = base.output_staging_dir(output_root, config_name, shard)
    if output_staging.exists():
        shutil.rmtree(output_staging)
    output_staging.mkdir(parents=True, exist_ok=True)

    stdout_path = output_staging / f"{shard.tag}_{shard.chrom}.tsv"
    chunks_path = stdout_path.with_suffix(".chunks.json")

    env = os.environ.copy()
    env["JAVA_OPTS"] = "-Xms512m -Xmx4g"

    chunk_beds = base.split_bed_chunks(shard.bed_path, chunk_size, output_staging)
    num_chunks = len(chunk_beds)
    work_items = build_chunk_work_items(
        chunk_beds=chunk_beds,
        shard=shard,
        output_staging=output_staging,
        stdout_path=stdout_path,
        vardict_bin=vardict_bin,
        reference=reference,
        bam_arg=bam_arg,
        config_flags=config_flags,
        sample_name_override=sample_name_override,
        total_bed_lines=total_bed_lines,
        need_chunking=need_chunking,
    )

    active_processes: dict[int, subprocess.Popen[bytes]] = {}
    process_lock = threading.Lock()
    chunk_results: dict[int, ChunkRunResult] = {}
    failed_chunk_idx: int | None = None
    failure_error: str | None = None

    try:
        try:
            with concurrent.futures.ThreadPoolExecutor(max_workers=resolved_chunk_workers) as executor:
                future_map = {
                    executor.submit(
                        execute_chunk_work_item,
                        item=item,
                        root=root,
                        env=env,
                        active_processes=active_processes,
                        process_lock=process_lock,
                    ): item.chunk_idx
                    for item in work_items
                }
                for future in concurrent.futures.as_completed(future_map):
                    chunk_idx = future_map[future]
                    try:
                        result = future.result()
                    except Exception as exc:
                        failed_chunk_idx = chunk_idx
                        failure_error = str(exc)
                        terminate_active_chunk_processes(active_processes, process_lock)
                        for pending_future in future_map:
                            pending_future.cancel()
                        break

                    chunk_results[result.chunk_idx] = result
                    if result.returncode != 0:
                        failed_chunk_idx = result.chunk_idx
                        failure_error = (
                            f"VarDict exited with code {result.returncode} (chunk {result.chunk_idx})"
                        )
                        terminate_active_chunk_processes(active_processes, process_lock)
                        for pending_future in future_map:
                            pending_future.cancel()
                        break
        except BaseException:
            terminate_active_chunk_processes(active_processes, process_lock)
            raise

        if failure_error is not None:
            write_chunk_parallel_failure_log(
                log_path=log_path,
                error=failure_error,
                failed_chunk_idx=failed_chunk_idx,
                output_staging=output_staging,
            )
            chunk_wall_s = [result.wall_s for _, result in sorted(chunk_results.items())]
            return base.ShardResult(
                tag=shard.tag,
                chrom=shard.chrom,
                status="failed",
                config_name=config_name,
                error=failure_error,
                log_path=str(log_path),
                **base.build_timing_summary(
                    shard_start=t_shard_start,
                    chunk_wall_s=chunk_wall_s,
                    num_chunks=num_chunks,
                ),
            )

        ordered_results = [chunk_results[idx] for idx in range(num_chunks)]
        chunk_wall_s = [result.wall_s for result in ordered_results]
        chunk_records = merge_chunk_outputs(
            ordered_results=ordered_results,
            stdout_path=stdout_path,
        )
        combine_chunk_logs(log_path, ordered_results)
        cleanup_chunk_success_artifacts(
            chunk_beds=chunk_beds,
            ordered_results=ordered_results,
        )

        output_order = base.sort_final_output_if_required(stdout_path, config_name)
        source_num_chunks = num_chunks if output_order is not None else None
        source_chunks = chunk_records if output_order is not None else None
        final_chunk_records = (
            base.final_output_chunk_records(
                stdout_path,
                wall_s=sum(chunk_wall_s),
                num_tiles=total_bed_lines,
            )
            if output_order is not None
            else chunk_records
        )
        final_num_chunks = 1 if output_order is not None else num_chunks

        monolithic_md5 = base.compute_file_md5(stdout_path)
        monolithic_bytes = stdout_path.stat().st_size
        base.write_chunks_json(
            chunks_path,
            monolithic_md5=monolithic_md5,
            monolithic_bytes=monolithic_bytes,
            num_chunks=final_num_chunks,
            chunk_size=chunk_size,
            chunks=final_chunk_records,
            vardict_commit=base.get_vardict_commit(root),
            generator_flags=list(config_flags),
            preset=config_name if config_name else "default",
            bed_sha256=base.compute_file_sha256(shard.bed_path),
            output_order=output_order,
            source_num_chunks=source_num_chunks,
            source_chunks=source_chunks,
        )

        if zstd_executor is None:
            base.compress_path(stdout_path)
            final_output_dir = base.output_dir(output_root, config_name, shard.chrom)
            final_output_dir.mkdir(parents=True, exist_ok=True)
            os.replace(
                stdout_path.with_name(f"{stdout_path.name}.zst"),
                final_output_dir / f"{stdout_path.name}.zst",
            )
            os.replace(chunks_path, final_output_dir / chunks_path.name)
            output_staging.rmdir()
        else:
            final_output_dir = base.output_dir(output_root, config_name, shard.chrom)
            final_output_dir.mkdir(parents=True, exist_ok=True)
            final_stdout_path = final_output_dir / stdout_path.name
            final_zst_path = final_stdout_path.with_name(f"{final_stdout_path.name}.zst")
            final_chunks_path = final_output_dir / chunks_path.name
            final_stdout_path.unlink(missing_ok=True)
            final_zst_path.unlink(missing_ok=True)
            final_chunks_path.unlink(missing_ok=True)
            os.replace(stdout_path, final_stdout_path)
            future = zstd_executor.submit(
                base.compress_and_promote_chunks,
                final_stdout_path,
                chunks_path,
                final_chunks_path,
                output_staging,
            )
            if pending_compressions is not None:
                pending_compressions.append(((config_name, shard.tag, shard.chrom), future))
            else:
                future.result()

        return base.ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="success",
            config_name=config_name,
            fixture_count=0,
            log_path=str(log_path),
            **base.build_timing_summary(
                shard_start=t_shard_start,
                chunk_wall_s=chunk_wall_s,
                num_chunks=num_chunks,
            ),
        )
    except Exception as exc:
        write_chunk_parallel_failure_log(
            log_path=log_path,
            error=str(exc),
            failed_chunk_idx=failed_chunk_idx,
            output_staging=output_staging,
        )
        return base.ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="failed",
            config_name=config_name,
            error=str(exc),
            log_path=str(log_path),
            **base.build_timing_summary(
                shard_start=t_shard_start,
                chunk_wall_s=[result.wall_s for _, result in sorted(chunk_results.items())],
                num_chunks=num_chunks,
            ),
        )
    finally:
        terminate_active_chunk_processes(active_processes, process_lock)


def run_shard(
    shard: base.Shard,
    force: bool,
    output_only: bool,
    config_name: str | None,
    config_flags: tuple[str, ...],
    root_str: str,
    output_root_str: str,
    chunk_size: int = base.CHUNK_SIZE,
    use_shm: bool = True,
    shm_root_str: str = base.DEFAULT_SHM_ROOT.as_posix(),
    samtools_bin: str | None = None,
) -> base.ShardResult:
    root = Path(root_str)
    output_root = Path(output_root_str)
    resume_result = base.resume_or_cleanup_shard(
        shard=shard,
        force=force,
        output_root=output_root,
        config_name=config_name,
        shard_start=time.monotonic(),
    )
    if resume_result is not None:
        return resume_result

    staged: base.StagedChrom | None = None
    try:
        bam_arg, reference, staged = base.resolve_vardict_inputs(
            root=root,
            shard=shard,
            use_shm=use_shm,
            shm_root=Path(shm_root_str),
            samtools_bin=samtools_bin,
        )
        sample_name_override = None
        if staged is not None and shard.kind == "single":
            sample_name_override = Path(base.BAM_MAP[shard.tag]).stem
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
            chunk_workers=CLI_CHUNK_WORKERS,
        )
    except Exception as exc:
        return base.ShardResult(
            tag=shard.tag,
            chrom=shard.chrom,
            status="failed",
            config_name=config_name,
            error=str(exc),
        )
    finally:
        if staged is not None:
            base.evict_chrom(staged)


def main() -> int:
    global CLI_CHUNK_WORKERS

    parser = build_parser()
    args = parser.parse_args()
    CLI_CHUNK_WORKERS = args.chunk_workers
    base.build_parser = build_parser
    base.execute_shard_run = execute_shard_run
    base.run_shard = run_shard
    return base.main()


if __name__ == "__main__":
    sys.exit(main())