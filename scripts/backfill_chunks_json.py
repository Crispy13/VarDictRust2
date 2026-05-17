#!/usr/bin/env python3
"""Backfill missing ``*.chunks.json`` sidecars for orphan ``*.tsv.zst`` shards.

For each ``<output_dir>/*/*/<stem>.tsv.zst`` without a paired
``<stem>.chunks.json``, stream-decompress the TSV, compute MD5 + byte length,
and write a single-chunk sidecar that satisfies ``validate_chunks_json``
(``scripts/sweep_fixtures_parallel.py``).

Default mode is DRY-RUN; pass ``--apply`` to actually write files.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

try:
    import zstandard  # type: ignore
except ImportError:  # pragma: no cover
    zstandard = None  # fall back to `zstd` CLI


READ_CHUNK_BYTES = 8 * 1024 * 1024


@contextmanager
def decompressed_reader(tsv: Path):
    if zstandard is not None:
        with tsv.open("rb") as compressed:
            reader = zstandard.ZstdDecompressor().stream_reader(compressed)
            try:
                yield reader
            finally:
                reader.close()
        return

    proc = subprocess.Popen(
        ["zstd", "-dc", str(tsv)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert proc.stdout is not None
    try:
        yield proc.stdout
        return_code = proc.wait()
        if return_code != 0:
            stderr = proc.stderr.read().decode("utf-8", errors="replace") if proc.stderr else ""
            raise subprocess.CalledProcessError(return_code, proc.args, stderr=stderr)
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        if proc.stdout is not None:
            proc.stdout.close()
        if proc.stderr is not None:
            proc.stderr.close()


def find_orphans(root: Path, name_glob: str) -> list[Path]:
    orphans: list[Path] = []
    # Layout: <root>/<preset>/<chrom>/<stem>.tsv.zst
    for tsv in sorted(root.glob(f"*/*/{name_glob}")):
        if not tsv.name.endswith(".tsv.zst"):
            continue
        stem = tsv.name[: -len(".tsv.zst")]
        sidecar = tsv.parent / f"{stem}.chunks.json"
        if not sidecar.exists():
            orphans.append(tsv)
    return orphans


def vardict_head(repo: Path) -> str:
    out = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
    return out.stdout.strip()


def build_payload(tsv: Path, vardict_commit: str) -> dict:
    md5 = hashlib.md5()
    n = 0
    with decompressed_reader(tsv) as reader:
        while True:
            chunk = reader.read(READ_CHUNK_BYTES)
            if not chunk:
                break
            md5.update(chunk)
            n += len(chunk)
    return {
        "monolithic_md5": md5.hexdigest(),
        "monolithic_bytes": n,
        "num_chunks": 1,
        "chunk_size": n,
        "chunks": [{"byte_range": [0, n]}],
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "vardict_commit": vardict_commit,
        "backfilled": True,
    }


def write_atomic(path: Path, payload: dict) -> None:
    tmp = path.with_name(f"{path.name}.tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        json.dump(payload, fh, indent=2, sort_keys=False)
        fh.write("\n")
    os.replace(tmp, path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path, help="Sweep fixtures output root")
    parser.add_argument("--apply", action="store_true", help="Actually write files (default: dry-run)")
    parser.add_argument(
        "--vardict-repo",
        type=Path,
        default=Path("VarDictJava"),
        help="Path to VarDictJava git repo (default: VarDictJava)",
    )
    parser.add_argument(
        "--name-glob",
        default="*.tsv.zst",
        help="Only consider shard filenames matching this glob (default: *.tsv.zst)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Maximum number of orphan shards to process after filtering",
    )
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()

    if not args.output_dir.is_dir():
        print(f"ERROR: not a directory: {args.output_dir}", file=sys.stderr)
        return 2

    vardict_commit = vardict_head(args.vardict_repo)
    if not args.quiet:
        print(f"vardict_commit: {vardict_commit}")
        print(f"mode: {'APPLY' if args.apply else 'DRY-RUN'}")

    all_tsv = sorted(args.output_dir.glob(f"*/*/{args.name_glob}"))
    orphans = find_orphans(args.output_dir, args.name_glob)
    if args.limit is not None:
        orphans = orphans[: args.limit]
    paired = len(all_tsv) - len(orphans)

    if not args.quiet:
        print(f"scanned tsv.zst: {len(all_tsv)}")
        print(f"already paired:  {paired}")
        print(f"orphans:         {len(orphans)}")

    written = 0
    errors: list[tuple[Path, str]] = []
    for tsv in orphans:
        stem = tsv.name[: -len(".tsv.zst")]
        sidecar = tsv.parent / f"{stem}.chunks.json"
        try:
            payload = build_payload(tsv, vardict_commit)
            if args.apply:
                write_atomic(sidecar, payload)
                written += 1
                if not args.quiet:
                    print(
                        f"wrote {sidecar} (md5={payload['monolithic_md5']}, "
                        f"bytes={payload['monolithic_bytes']})"
                    )
            elif not args.quiet:
                print(
                    f"would write {sidecar} (md5={payload['monolithic_md5']}, "
                    f"bytes={payload['monolithic_bytes']})"
                )
        except Exception as exc:  # noqa: BLE001
            errors.append((tsv, str(exc)))
            print(f"ERROR processing {tsv}: {exc}", file=sys.stderr)

    print(
        f"summary: scanned={len(all_tsv)} paired={paired} orphans={len(orphans)} "
        f"written={written} errors={len(errors)}"
    )
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
