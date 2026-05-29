#!/usr/bin/env python3
"""Sort existing E2E sweep TSV fixtures and refresh their sidecars.

The generator now writes canonical TSV fixtures in ``Region<TAB>row`` order.
This migration tool applies the same contract to already-generated
``*.tsv.zst`` caches without rerunning VarDictJava. It streams
decompression/compression through the ``zstd`` CLI and uses external GNU sort
with a bounded buffer, so large fixture sets can be migrated without loading
whole files into memory.

No fixture is rewritten unless ``--in-place`` or ``--output-fixture-root`` is
provided. Use ``--dry-run`` to inspect the selected scope.
"""

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

if __package__ is None or __package__ == "":
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from scripts import sweep_fixtures_parallel as base


@dataclass(frozen=True)
class FixturePath:
    source_tsv: Path
    source_sidecar: Path
    relative_tsv: Path
    config: str | None
    chrom: str
    tag: str


@dataclass
class MigrationResult:
    fixture: FixturePath
    status: str
    rows: int | None = None
    bytes: int | None = None
    seconds: float | None = None


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Sort existing sweep fixture TSV caches and update .chunks.json provenance."
    )
    parser.add_argument(
        "--fixture-root",
        default="tmp/sweep_fixtures",
        help="Fixture root containing manifest.json and output/ (default: tmp/sweep_fixtures).",
    )
    parser.add_argument(
        "--output-fixture-root",
        help="Stage sorted fixtures under this fixture root instead of modifying the source root.",
    )
    parser.add_argument(
        "--in-place",
        action="store_true",
        help="Rewrite fixtures in --fixture-root/output atomically.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Only list selected fixtures; do not sort or write files.",
    )
    parser.add_argument(
        "--configs",
        default="ALL",
        help="Comma-separated preset/config names to migrate, DEFAULT for legacy default layout, or ALL.",
    )
    parser.add_argument("--tags", default="ALL", help="Comma-separated tags or ALL.")
    parser.add_argument("--chroms", default="ALL", help="Comma-separated chromosome directory names or ALL.")
    parser.add_argument(
        "--sort-buffer-size",
        default=None,
        help="GNU sort --buffer-size value (default: VARDICT_SWEEP_FIXTURE_SORT_BUFFER_SIZE, "
        "VARDICT_E2E_SWEEP_SORT_BUFFER_SIZE, or 128M).",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Re-sort fixtures even when their sidecar already has compatible sorted provenance.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Migrate at most this many selected fixtures; useful for smoke tests.",
    )
    return parser


def resolve_project_path(raw_path: str) -> Path:
    path = Path(raw_path)
    if not path.is_absolute():
        path = Path.cwd() / path
    return path.resolve()


def output_root_for_fixture_root(fixture_root: Path) -> Path:
    if fixture_root.name == "output" and fixture_root.is_dir():
        return fixture_root
    return fixture_root / "output"


def sidecar_path(tsv: Path) -> Path:
    if not tsv.name.endswith(".tsv.zst"):
        raise ValueError(f"Not a .tsv.zst fixture: {tsv}")
    return tsv.with_name(f"{tsv.name[:-len('.tsv.zst')]}.chunks.json")


def parse_filter(raw: str) -> set[str] | None:
    if raw.strip().upper() == "ALL":
        return None
    values = {part.strip() for part in raw.split(",") if part.strip()}
    return values or None


def parse_fixture_path(source_output_root: Path, tsv: Path) -> FixturePath | None:
    rel = tsv.relative_to(source_output_root)
    parts = rel.parts
    if len(parts) == 2:
        config: str | None = None
        chrom, filename = parts
    elif len(parts) == 3:
        config = parts[0]
        chrom, filename = parts[1], parts[2]
    else:
        return None
    suffix = f"_{chrom}.tsv.zst"
    if not filename.endswith(suffix):
        return None
    tag = filename[: -len(suffix)]
    return FixturePath(
        source_tsv=tsv,
        source_sidecar=sidecar_path(tsv),
        relative_tsv=rel,
        config=config,
        chrom=chrom,
        tag=tag,
    )


def config_label(config: str | None) -> str:
    return config if config is not None else "DEFAULT"


def selected(value: str, allowed: set[str] | None) -> bool:
    return allowed is None or value in allowed


def discover_fixtures(
    source_output_root: Path,
    *,
    configs: set[str] | None,
    tags: set[str] | None,
    chroms: set[str] | None,
) -> list[FixturePath]:
    fixtures: list[FixturePath] = []
    for tsv in sorted(source_output_root.rglob("*.tsv.zst")):
        fixture = parse_fixture_path(source_output_root, tsv)
        if fixture is None:
            continue
        if not selected(config_label(fixture.config), configs):
            continue
        if not selected(fixture.tag, tags):
            continue
        if not selected(fixture.chrom, chroms):
            continue
        fixtures.append(fixture)
    return fixtures


def load_sidecar(path: Path) -> dict[str, object]:
    with path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise ValueError(f"{path} top-level JSON value is not an object")
    return payload


def has_sorted_provenance(payload: dict[str, object]) -> bool:
    output_order = payload.get("output_order")
    if not isinstance(output_order, dict):
        return False
    return (
        output_order.get("mode") == "sorted"
        and output_order.get("key") == base.SORT_KEY
        and output_order.get("lc_all") == base.SORT_ENV
    )


def write_json_atomic(path: Path, payload: dict[str, object]) -> None:
    tmp_path = path.with_name(f"{path.name}.tmp")
    with tmp_path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2, sort_keys=False)
        handle.write("\n")
    os.replace(tmp_path, path)


def decompress_to_plain(source_tsv: Path, plain_path: Path) -> None:
    with plain_path.open("wb") as output:
        subprocess.run(["zstd", "-dc", str(source_tsv)], stdout=output, check=True)


def compress_plain(plain_path: Path, dest_tsv: Path) -> None:
    subprocess.run(["zstd", "-q", "-f", str(plain_path), "-o", str(dest_tsv)], check=True)


def sidecar_for_sorted_fixture(
    existing: dict[str, object],
    plain_path: Path,
    output_order: dict[str, object],
) -> dict[str, object]:
    monolithic_bytes = plain_path.stat().st_size
    chunks = existing.get("chunks")
    num_chunks = existing.get("num_chunks")
    total_wall = 0.0
    total_tiles = 0
    if isinstance(chunks, list):
        for chunk in chunks:
            if not isinstance(chunk, dict):
                continue
            wall_s = chunk.get("wall_s")
            if isinstance(wall_s, (int, float)) and not isinstance(wall_s, bool):
                total_wall += float(wall_s)
            num_tiles = chunk.get("num_tiles")
            if isinstance(num_tiles, int) and not isinstance(num_tiles, bool):
                total_tiles += num_tiles

    payload = dict(existing)
    payload["monolithic_md5"] = base.compute_file_md5(plain_path)
    payload["monolithic_bytes"] = monolithic_bytes
    payload["num_chunks"] = 1
    payload["chunk_size"] = existing.get("chunk_size", monolithic_bytes)
    payload["chunks"] = [
        {
            "idx": 0,
            "md5_raw": base.compute_file_md5(plain_path),
            "wall_s": total_wall,
            "num_tiles": total_tiles,
            "byte_range": [0, monolithic_bytes],
            "derived_from": "fixture_sort_migration",
        }
    ]
    payload["generated_at"] = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    payload["output_order"] = output_order
    if num_chunks is not None and "source_num_chunks" not in payload:
        payload["source_num_chunks"] = num_chunks
    if chunks is not None and "source_chunks" not in payload:
        payload["source_chunks"] = chunks
    return payload


def migrate_fixture(
    fixture: FixturePath,
    *,
    source_output_root: Path,
    dest_output_root: Path,
    force: bool,
) -> MigrationResult:
    started = time.monotonic()
    if not fixture.source_sidecar.is_file():
        raise FileNotFoundError(f"Missing sidecar for {fixture.source_tsv}: {fixture.source_sidecar}")
    existing_sidecar = load_sidecar(fixture.source_sidecar)
    if has_sorted_provenance(existing_sidecar) and not force:
        return MigrationResult(fixture=fixture, status="skipped-sorted")

    dest_tsv = dest_output_root / fixture.relative_tsv
    dest_sidecar = sidecar_path(dest_tsv)
    dest_tsv.parent.mkdir(parents=True, exist_ok=True)
    work_dir = dest_tsv.parent / f".{dest_tsv.name}.sort-work"
    if work_dir.exists():
        shutil.rmtree(work_dir)
    work_dir.mkdir(parents=True)
    plain_path = work_dir / "fixture.tsv"
    sorted_tsv = work_dir / dest_tsv.name
    sorted_sidecar = work_dir / dest_sidecar.name
    try:
        decompress_to_plain(fixture.source_tsv, plain_path)
        output_order = base.sort_final_output_if_required(plain_path, fixture.config)
        if output_order is None:
            raise RuntimeError(f"Sorting was unexpectedly skipped for {fixture.source_tsv}")
        compress_plain(plain_path, sorted_tsv)
        payload = sidecar_for_sorted_fixture(existing_sidecar, plain_path, output_order)
        write_json_atomic(sorted_sidecar, payload)
        os.replace(sorted_tsv, dest_tsv)
        os.replace(sorted_sidecar, dest_sidecar)
        return MigrationResult(
            fixture=fixture,
            status="sorted",
            rows=int(output_order["rows"]),
            bytes=plain_path.stat().st_size,
            seconds=time.monotonic() - started,
        )
    finally:
        shutil.rmtree(work_dir, ignore_errors=True)


def copy_manifest_if_staging(source_fixture_root: Path, dest_fixture_root: Path) -> None:
    if source_fixture_root == dest_fixture_root:
        return
    source_manifest = source_fixture_root / "manifest.json"
    if not source_manifest.exists():
        return
    dest_fixture_root.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_manifest, dest_fixture_root / "manifest.json")


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.in_place and args.output_fixture_root:
        parser.error("--in-place and --output-fixture-root are mutually exclusive")
    if not args.dry_run and not args.in_place and not args.output_fixture_root:
        parser.error("Choose --in-place, --output-fixture-root, or --dry-run")
    if args.limit is not None and args.limit < 0:
        parser.error("--limit must be non-negative")

    source_fixture_root = resolve_project_path(args.fixture_root)
    source_output_root = output_root_for_fixture_root(source_fixture_root)
    if not source_output_root.is_dir():
        parser.error(f"fixture output root not found: {source_output_root}")

    if args.output_fixture_root:
        dest_fixture_root = resolve_project_path(args.output_fixture_root)
    else:
        dest_fixture_root = source_fixture_root
    dest_output_root = output_root_for_fixture_root(dest_fixture_root)

    if args.sort_buffer_size:
        os.environ["VARDICT_SWEEP_FIXTURE_SORT_BUFFER_SIZE"] = args.sort_buffer_size

    fixtures = discover_fixtures(
        source_output_root,
        configs=parse_filter(args.configs),
        tags=parse_filter(args.tags),
        chroms=parse_filter(args.chroms),
    )
    if args.limit is not None:
        fixtures = fixtures[: args.limit]

    print(f"Selected fixtures: {len(fixtures)}")
    print(f"Source output root: {source_output_root}")
    print(f"Destination output root: {dest_output_root}")
    print(f"Mode: {'dry-run' if args.dry_run else 'in-place' if args.in_place else 'staged'}")
    if args.dry_run:
        for fixture in fixtures:
            print(f"DRY-RUN {fixture.relative_tsv}")
        return 0

    copy_manifest_if_staging(source_fixture_root, dest_fixture_root)

    counts: dict[str, int] = {}
    for fixture in fixtures:
        result = migrate_fixture(
            fixture,
            source_output_root=source_output_root,
            dest_output_root=dest_output_root,
            force=args.force,
        )
        counts[result.status] = counts.get(result.status, 0) + 1
        detail = f" rows={result.rows} bytes={result.bytes} seconds={result.seconds:.3f}" if result.rows is not None else ""
        print(f"{result.status.upper()} {result.fixture.relative_tsv}{detail}", flush=True)

    print("Summary:")
    for status, count in sorted(counts.items()):
        print(f"  {status}: {count}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
