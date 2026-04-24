from __future__ import annotations

import atexit
import os
import shutil
import signal
import subprocess
import threading
from dataclasses import dataclass
from pathlib import Path


KNOWN_SAMTOOLS_PATHS = (
    Path("/home/eck/software/miniconda3/envs/rust_build_env/bin/samtools"),
    Path.home() / "software/miniconda3/envs/rust_build_env/bin/samtools",
)
MIN_FREE_BYTES = 2 * 1024 * 1024 * 1024
SIZE_THRESHOLD_BYTES = 2 * 1024 * 1024 * 1024

_REGISTRY_LOCK = threading.Lock()
_STAGED_REGISTRY: set["StagedChrom"] = set()
_HANDLERS_INSTALLED = False


@dataclass(frozen=True)
class StagedChrom:
    bam_path: Path
    fasta_path: Path
    root_dir: Path


class ShmCapacityError(RuntimeError):
    def __init__(self, shm_root: Path, required_bytes: int, free_bytes: int) -> None:
        super().__init__(
            f"insufficient capacity under {shm_root}: required at least {required_bytes} bytes, "
            f"found {free_bytes} bytes"
        )
        self.shm_root = shm_root
        self.required_bytes = required_bytes
        self.free_bytes = free_bytes


def locate_samtools() -> Path:
    path_hit = shutil.which("samtools")
    if path_hit:
        return Path(path_hit).resolve()

    candidates: list[Path] = []
    conda_prefix = os.environ.get("CONDA_PREFIX")
    if conda_prefix:
        candidates.append(Path(conda_prefix) / "bin/samtools")
    candidates.extend(KNOWN_SAMTOOLS_PATHS)

    home = Path.home()
    for pattern in (
        "software/miniconda3/envs/*/bin/samtools",
        "miniconda3/envs/*/bin/samtools",
    ):
        candidates.extend(home.glob(pattern))

    seen: set[Path] = set()
    for candidate in candidates:
        resolved = candidate.expanduser().resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        if resolved.is_file() and os.access(resolved, os.X_OK):
            return resolved

    checked = ", ".join(str(path) for path in seen)
    raise FileNotFoundError(
        "samtools not found; checked PATH and known conda locations. "
        f"Looked at: {checked}"
    )


def stage_chrom(
    tag: str,
    chrom: str,
    bam_source: Path,
    fasta_source: Path,
    shm_root: Path,
    samtools_bin: Path,
) -> StagedChrom:
    _install_cleanup_handlers()
    shm_root = shm_root.resolve()
    shm_root.mkdir(parents=True, exist_ok=True)
    os.chmod(shm_root, 0o700)
    _ensure_capacity(shm_root, MIN_FREE_BYTES)

    root_dir = (shm_root / f"pid_{os.getpid()}").resolve()
    root_dir.mkdir(parents=True, exist_ok=True)
    os.chmod(root_dir, 0o700)

    bam_path = root_dir / f"{tag}_{chrom}.bam"
    fasta_path = root_dir / f"{chrom}.fa"
    staged = StagedChrom(bam_path=bam_path, fasta_path=fasta_path, root_dir=root_dir)

    try:
        src_size = bam_source.stat().st_size
        if src_size <= SIZE_THRESHOLD_BYTES:
            # Path A — byte-preserve source BAM + BAI (keeps dense L5 bins).
            src_bai = _source_bai_path(bam_source)
            if not src_bai.is_file():
                raise FileNotFoundError(
                    f"source BAI not found for cp-preserve staging: {src_bai}"
                )
            # Per-branch capacity guard: require ~1.1× combined BAM+BAI bytes free,
            # at least MIN_FREE_BYTES. Raises ShmCapacityError if insufficient.
            required = int((src_size + src_bai.stat().st_size) * 1.1)
            _ensure_capacity(shm_root, max(MIN_FREE_BYTES, required))
            shutil.copy2(bam_source, bam_path)
            shutil.copy2(src_bai, _bam_index_path(bam_path))
        else:
            # Path B — threaded sliced stage with embedded BAI.
            _run(
                [
                    str(samtools_bin), "view", "-b", "-@", "10", "--write-index",
                    "-o", f"{bam_path}##idx##{_bam_index_path(bam_path)}",
                    str(bam_source), chrom,
                ]
            )
        _run([str(samtools_bin), "quickcheck", str(bam_path)])

        _run_to_file(
            [str(samtools_bin), "faidx", str(fasta_source), chrom],
            fasta_path,
        )
        _validate_fasta_header(fasta_path, chrom)
        _run([str(samtools_bin), "faidx", str(fasta_path)])
    except Exception:
        evict_chrom(staged)
        raise

    with _REGISTRY_LOCK:
        _STAGED_REGISTRY.add(staged)
    return staged


def evict_chrom(staged: StagedChrom) -> None:
    with _REGISTRY_LOCK:
        _STAGED_REGISTRY.discard(staged)

    _unlink_if_exists(staged.bam_path)
    _unlink_if_exists(_bam_index_path(staged.bam_path))
    _unlink_if_exists(staged.fasta_path)
    _unlink_if_exists(_fasta_index_path(staged.fasta_path))

    _prune_empty_dirs(staged.root_dir, stop_at=staged.root_dir.parent)


def cleanup_all_staged() -> None:
    with _REGISTRY_LOCK:
        staged_items = list(_STAGED_REGISTRY)
        _STAGED_REGISTRY.clear()

    for staged in staged_items:
        _unlink_if_exists(staged.bam_path)
        _unlink_if_exists(_bam_index_path(staged.bam_path))
        _unlink_if_exists(staged.fasta_path)
        _unlink_if_exists(_fasta_index_path(staged.fasta_path))
        _prune_empty_dirs(staged.root_dir, stop_at=staged.root_dir.parent)


def _source_bai_path(bam_source: Path) -> Path:
    """Return the canonical sibling .bai for a BAM source (<bam>.bai)."""
    return bam_source.with_name(f"{bam_source.name}.bai")


def _ensure_capacity(shm_root: Path, required_bytes: int) -> None:
    usage_path = shm_root
    while not usage_path.exists() and usage_path != usage_path.parent:
        usage_path = usage_path.parent
    free_bytes = shutil.disk_usage(usage_path).free
    if free_bytes < required_bytes:
        raise ShmCapacityError(shm_root, required_bytes, free_bytes)


def _run(command: list[str]) -> None:
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)


def _run_to_file(command: list[str], destination: Path) -> None:
    temp_path = destination.with_suffix(destination.suffix + ".tmp")
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        with temp_path.open("wb") as handle:
            subprocess.run(command, check=True, stdout=handle, stderr=subprocess.PIPE)
        os.replace(temp_path, destination)
    except Exception:
        _unlink_if_exists(temp_path)
        raise


def _validate_fasta_header(fasta_path: Path, chrom: str) -> None:
    with fasta_path.open("r", encoding="utf-8") as handle:
        header = handle.readline().strip()
    expected = f">{chrom}"
    if header != expected:
        raise RuntimeError(
            f"staged fasta header mismatch for {chrom}: expected {expected}, found {header}"
        )


def _prune_empty_dirs(path: Path, stop_at: Path) -> None:
    current = path
    while True:
        if current == stop_at:
            return
        try:
            current.rmdir()
        except FileNotFoundError:
            return
        except OSError:
            return
        if current.parent == current:
            return
        current = current.parent


def _bam_index_path(bam_path: Path) -> Path:
    return bam_path.with_name(f"{bam_path.name}.bai")


def _fasta_index_path(fasta_path: Path) -> Path:
    return fasta_path.with_name(f"{fasta_path.name}.fai")


def _unlink_if_exists(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass


def _install_cleanup_handlers() -> None:
    global _HANDLERS_INSTALLED
    if _HANDLERS_INSTALLED:
        return
    _HANDLERS_INSTALLED = True
    atexit.register(cleanup_all_staged)
    for signum in (signal.SIGINT, signal.SIGTERM):
        try:
            signal.signal(signum, _handle_signal)
        except ValueError:
            # Only the main thread may install signal handlers.
            continue


def _handle_signal(signum: int, _frame: object) -> None:
    cleanup_all_staged()
    signal.signal(signum, signal.SIG_DFL)
    os.kill(os.getpid(), signum)
