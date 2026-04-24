#!/usr/bin/env python3
"""I/O backend micro-benchmark: disk vs /dev/shm vs memfd.

Measures, for a BAM + fasta pair:
  1. Stage time  — cost of materializing the file in each backend from the source on disk.
  2. Cold-read time  — full sequential scan immediately after stage, before any warm-up.
  3. Warm-read time — second sequential scan (page cache / RAM already primed).

Usage:
    python3 scripts/io_backend_bench.py --bam testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam \
        --fasta testdata/hs37d5.fa

Notes:
- The script does NOT attempt to drop the page cache (requires root). Before running, the
  caller should ensure the source files are not already hot (check `free -h` buff/cache).
- memfd is created via os.memfd_create; staged by sendfile() from the source.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import os
import shutil
import sys
import time
from dataclasses import dataclass
from pathlib import Path

CHUNK = 1 << 20  # 1 MiB


@dataclass
class Result:
    backend: str
    stage_seconds: float
    cold_seconds: float
    warm_seconds: float
    total_bytes: int

    def mbps(self, seconds: float) -> float:
        if seconds <= 0:
            return float("inf")
        return (self.total_bytes / (1024 * 1024)) / seconds


def sequential_read_path(path: str) -> int:
    total = 0
    with open(path, "rb", buffering=0) as fh:
        while True:
            chunk = fh.read(CHUNK)
            if not chunk:
                break
            total += len(chunk)
    return total


def sequential_read_fd(fd: int) -> int:
    total = 0
    os.lseek(fd, 0, os.SEEK_SET)
    while True:
        chunk = os.read(fd, CHUNK)
        if not chunk:
            break
        total += len(chunk)
    return total


def stage_to_disk(src: Path, dst_dir: Path) -> Path:
    dst_dir.mkdir(parents=True, exist_ok=True)
    dst = dst_dir / src.name
    shutil.copy2(src, dst)
    # fsync to ensure the copy is flushed before measuring read
    with open(dst, "rb") as fh:
        os.fsync(fh.fileno())
    return dst


def stage_to_shm(src: Path, dst_dir: Path) -> Path:
    dst_dir.mkdir(parents=True, exist_ok=True)
    dst = dst_dir / src.name
    shutil.copy2(src, dst)
    return dst


_libc = ctypes.CDLL(ctypes.util.find_library("c") or "libc.so.6", use_errno=True)
_libc.syscall.restype = ctypes.c_long


def _memfd_create(name: str, flags: int = 0) -> int:
    if hasattr(os, "memfd_create"):
        return os.memfd_create(name, flags)
    # x86_64 Linux syscall 319
    SYS_MEMFD_CREATE = 319
    fd = _libc.syscall(SYS_MEMFD_CREATE, name.encode(), flags)
    if fd < 0:
        err = ctypes.get_errno()
        raise OSError(err, os.strerror(err), name)
    return int(fd)


def stage_to_memfd(src: Path) -> int:
    fd = _memfd_create(src.name, 0)
    with open(src, "rb") as sfh:
        src_fd = sfh.fileno()
        # sendfile copies in kernel space, fastest possible
        src_size = os.fstat(src_fd).st_size
        offset = 0
        while offset < src_size:
            sent = os.sendfile(fd, src_fd, offset, src_size - offset)
            if sent == 0:
                break
            offset += sent
    return fd


def bench_backend(name: str, src_files: list[Path], stage_fn, read_fn, cleanup_fn) -> Result:
    stage_start = time.perf_counter()
    handles = [stage_fn(src) for src in src_files]
    stage_seconds = time.perf_counter() - stage_start

    cold_start = time.perf_counter()
    total_bytes = sum(read_fn(h) for h in handles)
    cold_seconds = time.perf_counter() - cold_start

    warm_start = time.perf_counter()
    total_bytes_warm = sum(read_fn(h) for h in handles)
    warm_seconds = time.perf_counter() - warm_start

    assert total_bytes == total_bytes_warm

    cleanup_fn(handles)

    return Result(
        backend=name,
        stage_seconds=stage_seconds,
        cold_seconds=cold_seconds,
        warm_seconds=warm_seconds,
        total_bytes=total_bytes,
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--bam", required=True, type=Path)
    ap.add_argument("--fasta", required=True, type=Path)
    ap.add_argument("--disk-root", default="/tmp/io_bench_disk", type=Path)
    ap.add_argument("--shm-root", default="/dev/shm/io_bench_shm", type=Path)
    args = ap.parse_args()

    src_files = [args.bam, args.fasta]
    for f in src_files:
        if not f.exists():
            print(f"missing: {f}", file=sys.stderr)
            return 1

    total_src = sum(f.stat().st_size for f in src_files)
    print(f"Source: {args.bam.name} ({args.bam.stat().st_size / 1e6:.1f} MB) + "
          f"{args.fasta.name} ({args.fasta.stat().st_size / 1e6:.1f} MB) = "
          f"{total_src / 1e6:.1f} MB total", flush=True)
    print(flush=True)

    # Warn if buff/cache is already high
    try:
        with open("/proc/meminfo") as mi:
            for line in mi:
                if line.startswith("Cached:"):
                    cached_kb = int(line.split()[1])
                    print(f"[info] Page cache at start: {cached_kb / 1024:.0f} MB", flush=True)
                    break
    except OSError:
        pass
    print(flush=True)

    results: list[Result] = []

    # --- disk ---
    if args.disk_root.exists():
        shutil.rmtree(args.disk_root)

    def disk_read(p: Path) -> int:
        return sequential_read_path(str(p))

    def disk_cleanup(paths: list[Path]) -> None:
        shutil.rmtree(args.disk_root, ignore_errors=True)

    print("[1/3] disk (tmpfs-free path)", flush=True)
    results.append(bench_backend("disk", src_files,
                                  lambda s: stage_to_disk(s, args.disk_root),
                                  disk_read, disk_cleanup))

    # --- shm ---
    if args.shm_root.exists():
        shutil.rmtree(args.shm_root)

    def shm_cleanup(paths: list[Path]) -> None:
        shutil.rmtree(args.shm_root, ignore_errors=True)

    print("[2/3] /dev/shm (tmpfs)", flush=True)
    results.append(bench_backend("shm", src_files,
                                  lambda s: stage_to_shm(s, args.shm_root),
                                  disk_read, shm_cleanup))

    # --- memfd ---
    def memfd_cleanup(fds: list[int]) -> None:
        for fd in fds:
            os.close(fd)

    print("[3/3] memfd", flush=True)
    results.append(bench_backend("memfd", src_files,
                                  stage_to_memfd,
                                  sequential_read_fd,
                                  memfd_cleanup))

    # --- report ---
    print(flush=True)
    hdr = f"{'backend':<8} {'stage(s)':>10} {'cold(s)':>10} {'cold MB/s':>12} {'warm(s)':>10} {'warm MB/s':>12}"
    print(hdr)
    print("-" * len(hdr))
    for r in results:
        print(f"{r.backend:<8} {r.stage_seconds:>10.3f} {r.cold_seconds:>10.3f} "
              f"{r.mbps(r.cold_seconds):>12.1f} {r.warm_seconds:>10.3f} "
              f"{r.mbps(r.warm_seconds):>12.1f}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
