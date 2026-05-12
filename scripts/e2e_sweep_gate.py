"""E2E sweep parity gate (manual-only).

Stages existing fixtures from --fixture-source into tmp/sweep_fixtures/output/,
initializes manifest.json for fresh staging roots, populates cache_entries via
scripts.lib.merge_manifest, and runs scoped parity_e2e_sweep cargo tests.
Produces parity-failure-report.json on red.

Stdlib only.
"""
from __future__ import annotations

import argparse
import csv
import fcntl
import json
import os
import re
import selectors
import shutil
import subprocess
import sys
import time
from contextlib import contextmanager
from pathlib import Path
from typing import TextIO


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _resolve_fixture_root() -> Path:
    """Resolve the sweep fixture root.

    Honors ``VARDICT_E2E_SWEEP_FIXTURE_ROOT``; falls back to ``tmp/sweep_fixtures`` so
    default CI behavior is byte-identical when the env var is unset. Relative values
    resolve under :data:`PROJECT_ROOT`.
    """

    raw = os.environ.get("VARDICT_E2E_SWEEP_FIXTURE_ROOT")
    if raw is None or not raw.strip():
        return PROJECT_ROOT / "tmp" / "sweep_fixtures"
    candidate = Path(raw).expanduser()
    if not candidate.is_absolute():
        candidate = PROJECT_ROOT / candidate
    return candidate


CANONICAL_FIXTURE_ROOT = _resolve_fixture_root()
CANONICAL_OUTPUT_ROOT = CANONICAL_FIXTURE_ROOT / "output"
CANONICAL_MANIFEST = CANONICAL_FIXTURE_ROOT / "manifest.json"
LOCK_FILE = CANONICAL_FIXTURE_ROOT / ".manifest.lock"
MANIFEST_SNAPSHOT = CANONICAL_FIXTURE_ROOT / ".manifest.cache_entries.before.json"
PARITY_ITERATION_DIR = PROJECT_ROOT / "tmp" / "parity-iteration"
FAILURE_REPORT = PARITY_ITERATION_DIR / "parity-failure-report.json"
LAST_PASS = PARITY_ITERATION_DIR / "last-pass.json"
PRESETS_TSV = PROJECT_ROOT / "scripts" / "config_presets.tsv"
DEFAULT_TAGS = ("hg002", "na12878_exome", "na12878_lowcov")
MERGE_PRESERVE_WORK = CANONICAL_FIXTURE_ROOT / ".manifest.cache_entries.gate_working.json"
RUNNING_TESTS_RE = re.compile(r"running (\d+) tests?")
PROGRESS_LOG_HANDLE: TextIO | None = None


sys.path.insert(0, str(PROJECT_ROOT))
try:
    from scripts.lib.merge_manifest import merge_cache_entries
except ImportError as exc:  # pragma: no cover - import guard
    raise SystemExit(
        f"ERROR: cannot import scripts.lib.merge_manifest: {exc}\n"
        f"Run via `bash scripts/e2e_sweep_gate.sh ...` or `cd {PROJECT_ROOT} && "
        "python3 -m scripts.e2e_sweep_gate ...`."
    ) from exc


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Stage existing sweep fixtures into tmp/sweep_fixtures/output/, merge "
            "manifest cache_entries, and run scoped parity_e2e_sweep cargo tests."
        ),
        epilog=(
            "Chrom scoping is enforced through --sweep-bed-root/<tag>/*.bed. The gate "
            "refuses extra BED chroms unless --allow-extra-beds is set. When --chrom "
            "is exactly '1', the gate refuses to run if --sweep-bed-root/<tag>/ "
            "contains BED files for non-chr1 chromosomes. Set "
            "VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1 to bypass with a stderr warning. "
            "Fixture root precedence: --fixture-source (CLI, used as the staging "
            "source) > VARDICT_E2E_SWEEP_FIXTURE_ROOT (env, used as the canonical "
            "staging destination + manifest location) > default tmp/sweep_fixtures."
        ),
    )
    preset_group = parser.add_mutually_exclusive_group()
    preset_group.add_argument(
        "--preset",
        action="append",
        default=[],
        help="Preset to include. Repeat to select multiple presets.",
    )
    preset_group.add_argument(
        "--all-presets",
        action="store_true",
        help="Read all presets from scripts/config_presets.tsv.",
    )
    parser.add_argument(
        "--tag",
        action="append",
        default=None,
        help="Sweep tag to include. Repeat to select multiple tags. Default: hg002.",
    )
    parser.add_argument(
        "--chrom",
        action="append",
        default=None,
        help="Chromosome stem to include. Repeat to select multiple chroms. Default: 1.",
    )
    parser.add_argument(
        "--sweep-bed-root",
        type=Path,
        default=PROJECT_ROOT / "tmp" / "sweep_beds",
        help="Root containing per-tag BED directories. Default: tmp/sweep_beds.",
    )
    parser.add_argument(
        "--fixture-source",
        help="Fixture source root to stage from. Required unless --unstage is the only mode and a snapshot exists.",
    )
    parser.add_argument("--dry-run", action="store_true", help="Print the resolved matrix and planned actions, then exit.")
    parser.add_argument("--unstage", action="store_true", help="Remove previously staged symlinks and restore manifest cache_entries.")
    parser.add_argument(
        "--force",
        action="store_true",
        help="Replace mismatched symlinks without prompting. Required in non-interactive mode.",
    )
    parser.add_argument(
        "--allow-extra-beds",
        action="store_true",
        help="Warn instead of failing when --sweep-bed-root/<tag>/ contains chroms outside the matrix.",
    )
    parser.add_argument(
        "--report-dir",
        type=Path,
        default=PARITY_ITERATION_DIR,
        help="Directory for parity gate reports. Default: tmp/parity-iteration.",
    )
    parser.add_argument(
        "--cargo-extra-arg",
        action="extend",
        nargs="+",
        default=[],
        metavar="ARG",
        help="Additional argument(s) appended after the cargo test selector arguments.",
    )
    parser.add_argument(
        "--test-threads",
        type=int,
        default=4,
        help=(
            "Thread count passed to cargo test for wrapper-driven sweep runs. Default: 4. "
            "Each parity sweep chunk peaks around 4.6 GB RAM and 1.5 cores of internal "
            "work, so 4 keeps the host within ~18 GB and avoids paging. This is the wrapper "
            "default, not a repo-wide rule: some CI or manual repro paths intentionally pin "
            "other values. Set higher only on machines with plenty of free RAM."
        ),
    )
    return parser


def load_all_presets(tsv_path: Path) -> list[str]:
    presets: list[str] = []
    with tsv_path.open("r", encoding="utf-8") as handle:
        for row in csv.reader(handle, delimiter="\t"):
            if not row or row[0].startswith("#"):
                continue
            presets.append(row[0])
    return presets


def normalize_args(args: argparse.Namespace, parser: argparse.ArgumentParser) -> argparse.Namespace:
    args.tag = list(args.tag or ["hg002"])
    args.chrom = [str(chrom) for chrom in (args.chrom or ["1"])]
    if args.fixture_source:
        args.fixture_source = Path(args.fixture_source).expanduser().resolve()
    elif not args.unstage:
        parser.error("--fixture-source is required unless --unstage is the only mode")
    elif not MANIFEST_SNAPSHOT.exists():
        parser.error("--fixture-source is required when no manifest snapshot exists for --unstage")
    args.sweep_bed_root = Path(args.sweep_bed_root).expanduser().resolve()
    args.report_dir = Path(args.report_dir).expanduser().resolve()
    if args.test_threads < 1:
        parser.error("--test-threads must be >= 1")
    return args


def resolve_matrix(args: argparse.Namespace) -> list[tuple[str, str, str]]:
    presets = load_all_presets(PRESETS_TSV) if args.all_presets else list(args.preset)
    if not presets:
        raise SystemExit("ERROR: no presets selected; pass --preset or --all-presets")
    return [(preset, tag, chrom) for preset in presets for tag in args.tag for chrom in args.chrom]


def print_matrix(matrix: list[tuple[str, str, str]]) -> None:
    print(f"Resolved matrix: {len(matrix)} cells")
    for preset, tag, chrom in matrix:
        print(f"  {preset} / {tag} / chr{chrom}")


def sweep_test_selector(tag: str) -> str:
    return f"{tag}_sweep::parity_e2e_sweep_{tag}"


def sweep_test_command(args: argparse.Namespace, selector: str) -> list[str]:
    return [
        "cargo",
        "test",
        "--profile",
        "debug-release",
        "--test",
        "parity_e2e_sweep",
        "--",
        "--include-ignored",
        "--exact",
        selector,
        f"--test-threads={args.test_threads}",
        *args.cargo_extra_arg,
    ]


def sweep_test_reproducer(
    args: argparse.Namespace,
    preset: str,
    sweep_bed_root: Path,
    selector: str,
) -> str:
    return (
        f"VARDICT_E2E_SWEEP_CONFIG={preset} "
        f"VARDICT_E2E_SWEEP_BED_ROOT={sweep_bed_root} "
        f"CI=true {' '.join(sweep_test_command(args, selector))}"
    )


def print_planned_actions(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> None:
    if args.unstage:
        print("Planned actions:")
        for preset, tag, chrom in matrix:
            print(f"  unstage symlink {target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, '.tsv.zst')}")
            print(f"  unstage symlink {target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, '.chunks.json')}")
        if MANIFEST_SNAPSHOT.exists():
            print(f"  restore manifest cache_entries from {MANIFEST_SNAPSHOT}")
        else:
            print("  no manifest snapshot present to restore")
        return

    print("Planned actions:")
    for preset, tag, chrom in matrix:
        print(
            f"  stage {source_path(args.fixture_source, preset, chrom, tag, '.tsv.zst')} -> "
            f"{target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, '.tsv.zst')}"
        )
        print(
            f"  stage {source_path(args.fixture_source, preset, chrom, tag, '.chunks.json')} -> "
            f"{target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, '.chunks.json')}"
        )
    for preset, tags in grouped_tags_by_preset(matrix).items():
        print(f"  merge manifest cache_entries for {preset} tags={','.join(tags)}")
    print("  run provenance checks against staged chunks metadata")
    for preset, tag in grouped_pairs(matrix):
        selector = sweep_test_selector(tag)
        print(f"  {sweep_test_reproducer(args, preset, args.sweep_bed_root, selector)}")


def format_elapsed(started_at: float | None) -> str:
    if started_at is None:
        return "00:00:00"

    elapsed_seconds = max(0, int(time.monotonic() - started_at))
    hours, remainder = divmod(elapsed_seconds, 3600)
    minutes, seconds = divmod(remainder, 60)
    return f"{hours:02}:{minutes:02}:{seconds:02}"


def format_status_line(
    phase: str,
    *,
    started_at: float | None = None,
    completed: int | None = None,
    total: int | None = None,
    active: str | None = None,
    event: str | None = None,
    detail: str | None = None,
) -> str:
    parts = [f"STATUS phase={phase}"]
    if event is not None:
        parts.append(f"event={event}")
    if completed is not None and total is not None:
        parts.append(f"progress={completed}/{total}")
    if active is not None:
        parts.append(f"active={active}")
    parts.append(f"elapsed={format_elapsed(started_at)}")
    if detail is not None:
        parts.append(detail)
    return " ".join(parts)


def emit_status(
    phase: str,
    *,
    started_at: float | None = None,
    completed: int | None = None,
    total: int | None = None,
    active: str | None = None,
    event: str | None = None,
    detail: str | None = None,
) -> None:
    line = format_status_line(
        phase,
        started_at=started_at,
        completed=completed,
        total=total,
        active=active,
        event=event,
        detail=detail,
    )
    print(line, flush=True)
    if PROGRESS_LOG_HANDLE is not None:
        print(line, file=PROGRESS_LOG_HANDLE, flush=True)


def emit_warning_summary(
    phase: str,
    warning_counts: dict[str, int],
    *,
    started_at: float | None = None,
    samples: list[str] | None = None,
) -> None:
    if not warning_counts:
        return

    total_warnings = sum(warning_counts.values())
    breakdown = ",".join(f"{key}={value}" for key, value in sorted(warning_counts.items()))
    detail_parts = [f"warnings={total_warnings}", f"breakdown={breakdown}"]
    if samples:
        detail_parts.append(f"samples={truncate_guard_items(samples)}")
    line = f"WARNING phase={phase} elapsed={format_elapsed(started_at)} {' '.join(detail_parts)}"
    print(line, file=sys.stderr, flush=True)
    if PROGRESS_LOG_HANDLE is not None:
        print(line, file=PROGRESS_LOG_HANDLE, flush=True)


def progress_log_path(args: argparse.Namespace) -> Path:
    return args.report_dir / "progress.log"


@contextmanager
def progress_log(args: argparse.Namespace):
    global PROGRESS_LOG_HANDLE

    args.report_dir.mkdir(parents=True, exist_ok=True)
    previous_handle = PROGRESS_LOG_HANDLE
    handle = progress_log_path(args).open("a", encoding="utf-8", buffering=1)
    PROGRESS_LOG_HANDLE = handle
    try:
        yield
    finally:
        handle.close()
        PROGRESS_LOG_HANDLE = previous_handle


def grouped_cells_by_pair(matrix: list[tuple[str, str, str]]) -> list[tuple[str, str, list[str]]]:
    grouped: dict[tuple[str, str], list[str]] = {}
    ordered_pairs: list[tuple[str, str]] = []
    for preset, tag, chrom in matrix:
        pair = (preset, tag)
        if pair not in grouped:
            grouped[pair] = []
            ordered_pairs.append(pair)
        grouped[pair].append(chrom)
    return [(preset, tag, grouped[(preset, tag)]) for preset, tag in ordered_pairs]


def track_warning(
    warning_counts: dict[str, int],
    warning_key: str,
    warning_message: str,
    warning_samples: list[str],
) -> None:
    warning_counts[warning_key] = warning_counts.get(warning_key, 0) + 1
    if len(warning_samples) < 5:
        warning_samples.append(warning_message)


def run_streaming_subprocess(
    cmd: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    process = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdout is not None
    assert process.stderr is not None

    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, data="stdout")
    selector.register(process.stderr, selectors.EVENT_READ, data="stderr")
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []

    try:
        while selector.get_map():
            for key, _mask in selector.select():
                stream = key.fileobj
                raw_line = stream.readline()
                if raw_line == "":
                    selector.unregister(stream)
                    continue

                line = raw_line.rstrip("\n")
                if key.data == "stderr":
                    stderr_lines.append(line)
                    print(line, file=sys.stderr, flush=True)
                else:
                    stdout_lines.append(line)
                    print(line, flush=True)
    finally:
        selector.close()
        process.stdout.close()
        process.stderr.close()

    return subprocess.CompletedProcess(
        cmd,
        process.wait(),
        "\n".join(stdout_lines),
        "\n".join(stderr_lines),
    )


@contextmanager
def manifest_lock():
    LOCK_FILE.parent.mkdir(parents=True, exist_ok=True)
    handle = LOCK_FILE.open("a+")
    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        yield
    finally:
        fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        handle.close()


def target_path(root: Path, preset: str, chrom: str, tag: str, suffix: str) -> Path:
    if preset == "default":
        return root / chrom / f"{tag}_{chrom}{suffix}"
    return root / preset / chrom / f"{tag}_{chrom}{suffix}"


def source_path(root: Path, preset: str, chrom: str, tag: str, suffix: str) -> Path:
    return target_path(root, preset, chrom, tag, suffix)


def grouped_tags_by_preset(matrix: list[tuple[str, str, str]]) -> dict[str, list[str]]:
    grouped: dict[str, list[str]] = {}
    for preset, tag, _chrom in matrix:
        grouped.setdefault(preset, [])
        if tag not in grouped[preset]:
            grouped[preset].append(tag)
    return grouped


def grouped_chroms_by_tag(matrix: list[tuple[str, str, str]]) -> dict[str, list[str]]:
    grouped: dict[str, list[str]] = {}
    for _preset, tag, chrom in matrix:
        grouped.setdefault(tag, [])
        if chrom not in grouped[tag]:
            grouped[tag].append(chrom)
    return grouped


def grouped_pairs(matrix: list[tuple[str, str, str]]) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()
    for preset, tag, _chrom in matrix:
        pair = (preset, tag)
        if pair in seen:
            continue
        seen.add(pair)
        pairs.append(pair)
    return pairs


def read_json(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, payload: dict, *, sort_keys: bool = True) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=sort_keys) + "\n", encoding="utf-8")


def write_json_atomic(path: Path, payload: dict, *, sort_keys: bool = True) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = path.with_suffix(path.suffix + ".tmp")
    temp_path.write_text(json.dumps(payload, indent=2, sort_keys=sort_keys) + "\n", encoding="utf-8")
    os.replace(temp_path, path)


def ensure_stage_manifest() -> bool:
    if CANONICAL_MANIFEST.is_file():
        return False

    payload = {
        "vardictjava_commit": live_vardictjava_commit(),
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "mode": "staged_existing_fixtures",
        "cache_entries": {},
    }
    write_json(CANONICAL_MANIFEST, payload, sort_keys=False)
    print(f"Initialized manifest -> {CANONICAL_MANIFEST}")
    return True


def snapshot_cache_entries() -> bool:
    if MANIFEST_SNAPSHOT.is_file():
        print(f"Preserving existing manifest snapshot: {MANIFEST_SNAPSHOT}")
        return False

    if not CANONICAL_MANIFEST.is_file():
        payload = {"cache_entries": {}}
    else:
        data = read_json(CANONICAL_MANIFEST)
        payload = {"cache_entries": data.get("cache_entries", {})}
    write_json(MANIFEST_SNAPSHOT, payload)
    print(f"Snapshot cache_entries -> {MANIFEST_SNAPSHOT}")
    return True


def live_vardictjava_commit() -> str:
    return subprocess.check_output(
        ["git", "-C", str(PROJECT_ROOT / "VarDictJava"), "rev-parse", "HEAD"],
        text=True,
    ).strip()


def sha256_concat(paths: list[Path]) -> str:
    import hashlib

    digest = hashlib.sha256()
    for path in paths:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def bed_sha256(sweep_bed_root: Path, tag: str) -> str:
    bed_paths = sorted((sweep_bed_root / tag).glob("*.bed"))
    return sha256_concat(bed_paths)


def manifest_cache_entries_payload() -> dict:
    if not CANONICAL_MANIFEST.exists():
        return {}
    return read_json(CANONICAL_MANIFEST).get("cache_entries", {})


def prepare_merge_preserve_file() -> Path:
    write_json(MERGE_PRESERVE_WORK, manifest_cache_entries_payload())
    return MERGE_PRESERVE_WORK


def ensure_fixture_source(args: argparse.Namespace) -> Path:
    if args.fixture_source is None:
        raise SystemExit("ERROR: --fixture-source is required for stage mode")
    if not args.fixture_source.is_dir():
        raise SystemExit(f"ERROR: fixture source is not a directory: {args.fixture_source}")
    return args.fixture_source


def truncate_guard_items(items: list[str]) -> str:
    limit = 5
    if len(items) <= limit:
        return ", ".join(items)
    return f"{', '.join(items[:limit])} (and {len(items) - limit} more)"


def enforce_chr1_scope_guard(args: argparse.Namespace) -> None:
    if args.chrom != ["1"]:
        return

    extra_stems: set[str] = set()
    extra_paths: list[str] = []
    for tag in args.tag:
        for path in (args.sweep_bed_root / tag).glob("*.bed"):
            if path.stem in {"1", "chr1"}:
                continue
            extra_stems.add(path.stem)
            extra_paths.append(str(path.resolve()))

    if not extra_paths:
        return

    stems_summary = truncate_guard_items(sorted(extra_stems))
    paths_summary = truncate_guard_items(sorted(extra_paths))
    if os.environ.get("VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM") == "1":
        print(
            "WARNING: VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1 set \u2014 running --chrom 1 "
            f"against multi-chrom BED root {args.sweep_bed_root} "
            f"(extra chroms: {stems_summary}); compute will exceed chr1 budget",
            file=sys.stderr,
        )
        return

    raise SystemExit(
        "ERROR: --chrom 1 was requested but --sweep-bed-root "
        f"{args.sweep_bed_root} contains BED files for non-chr1 chromosomes: {stems_summary} "
        f"(full paths: {paths_summary}); set VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1 to bypass, "
        "or point --sweep-bed-root at a chr1-only tree such as tmp/sweep_beds_chr1only."
    )


def validate_bed_scope(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> None:
    grouped = grouped_chroms_by_tag(matrix)
    errors: list[str] = []
    warnings: list[str] = []
    for tag, chroms in grouped.items():
        bed_dir = args.sweep_bed_root / tag
        if not bed_dir.is_dir():
            errors.append(f"ERROR: missing BED directory for tag {tag}: {bed_dir}")
            continue
        actual = sorted(path.stem for path in bed_dir.glob("*.bed"))
        expected = sorted(chroms)
        for chrom in expected:
            bed_path = bed_dir / f"{chrom}.bed"
            if not bed_path.is_file():
                errors.append(f"ERROR: missing BED file required by matrix: {bed_path}")
        extras = [chrom for chrom in actual if chrom not in expected]
        if extras:
            message = (
                f"BED set for {tag} contains extra chroms outside the matrix: {', '.join(extras)} "
                f"(expected only: {', '.join(expected)})"
            )
            if args.allow_extra_beds:
                warnings.append(f"WARNING: {message}")
            else:
                errors.append(f"ERROR: {message}; rerun with --allow-extra-beds to bypass this guard")
    if warnings:
        for warning in warnings:
            print(warning, file=sys.stderr)
    if errors:
        raise SystemExit("\n".join(errors))


def stage_symlink(source: Path, target: Path, args: argparse.Namespace, staged_links: list[Path]) -> str:
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink():
        existing = target.resolve()
        if existing == source.resolve():
            return "reused"
        if not should_replace_link(target, existing, source, args):
            raise SystemExit(
                f"ERROR: {target} already points to {existing}; rerun with --force to replace it"
            )
        target.unlink()
        link_state = "replaced"
    else:
        link_state = "linked"

    if target.exists() and not target.is_symlink():
        raise SystemExit(f"ERROR: refusing to overwrite regular file at {target}")

    os.symlink(source.resolve(), target)
    staged_links.append(target)
    return link_state


def should_replace_link(target: Path, existing: Path, source: Path, args: argparse.Namespace) -> bool:
    if args.force:
        return True
    if not sys.stdin.isatty():
        return False
    reply = input(f"Replace {target} -> {existing} with {source}? [y/N] ").strip().lower()
    return reply in {"y", "yes"}


def run_stage(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> None:
    fixture_source = ensure_fixture_source(args)
    validate_bed_scope(args, matrix)

    missing_sources: list[str] = []
    for preset, tag, chrom in matrix:
        tsv_source = source_path(fixture_source, preset, chrom, tag, ".tsv.zst")
        if not tsv_source.is_file():
            missing_sources.append(str(tsv_source))
    if missing_sources:
        raise SystemExit("ERROR: missing source TSV fixtures:\n" + "\n".join(missing_sources))

    ensure_stage_manifest()
    snapshot_cache_entries()

    staged_links: list[Path] = []
    stage_started_at = time.monotonic()
    pair_batches = grouped_cells_by_pair(matrix)
    total_pairs = len(pair_batches)
    total_cells = len(matrix)
    completed_cells = 0
    link_counts = {"linked": 0, "replaced": 0, "reused": 0}
    stage_warning_counts: dict[str, int] = {}
    stage_warning_samples: list[str] = []
    emit_status("stage", started_at=stage_started_at, completed=0, total=total_pairs, event="start")

    for pair_index, (preset, tag, chroms) in enumerate(pair_batches, start=1):
        for chrom in chroms:
            tsv_source = source_path(fixture_source, preset, chrom, tag, ".tsv.zst")
            link_counts[
                stage_symlink(
                    tsv_source,
                    target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, ".tsv.zst"),
                    args,
                    staged_links,
                )
            ] += 1

            chunks_source = source_path(fixture_source, preset, chrom, tag, ".chunks.json")
            if chunks_source.is_file():
                link_counts[
                    stage_symlink(
                        chunks_source,
                        target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, ".chunks.json"),
                        args,
                        staged_links,
                    )
                ] += 1
            else:
                track_warning(
                    stage_warning_counts,
                    "missing_chunks",
                    f"{preset}/{tag}/chr{chrom}",
                    stage_warning_samples,
                )
            completed_cells += 1

        emit_status(
            "stage",
            started_at=stage_started_at,
            completed=pair_index,
            total=total_pairs,
            active=f"{preset}/{tag}",
            event="pair-complete",
            detail=(
                f"cells={completed_cells}/{total_cells} linked={link_counts['linked']} "
                f"replaced={link_counts['replaced']} reused={link_counts['reused']} "
                f"warnings={sum(stage_warning_counts.values())}"
            ),
        )

    for preset, tags in grouped_tags_by_preset(matrix).items():
        preserve_path = prepare_merge_preserve_file()
        logical_flags = (
            f"--output-only --config {preset} --tags {','.join(tags)} "
            f"--sweep-bed-root {args.sweep_bed_root}"
        )
        merge_cache_entries(
            config_name=preset,
            tags_csv=",".join(tags),
            logical_flags=logical_flags,
            project_root=PROJECT_ROOT,
            sweep_bed_root=args.sweep_bed_root,
            preserve_path=preserve_path,
            manifest_only=True,
            fixture_output_root=CANONICAL_OUTPUT_ROOT,
        )
        emit_status(
            "stage",
            started_at=stage_started_at,
            active=f"{preset}/{','.join(tags)}",
            event="manifest-merged",
        )

    emit_warning_summary("stage", stage_warning_counts, started_at=stage_started_at, samples=stage_warning_samples)
    emit_status(
        "stage",
        started_at=stage_started_at,
        completed=total_pairs,
        total=total_pairs,
        event="complete",
        detail=(
            f"cells={total_cells}/{total_cells} linked={link_counts['linked']} "
            f"replaced={link_counts['replaced']} reused={link_counts['reused']} "
            f"warnings={sum(stage_warning_counts.values())}"
        ),
    )


def run_provenance_check(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> None:
    live_commit = live_vardictjava_commit()
    grouped_tags = grouped_tags_by_preset(matrix)
    provenance_started_at = time.monotonic()
    pair_batches = grouped_cells_by_pair(matrix)
    total_pairs = len(pair_batches)
    total_cells = len(matrix)
    completed_cells = 0
    warning_counts: dict[str, int] = {}
    warning_samples: list[str] = []
    emit_status("provenance", started_at=provenance_started_at, completed=0, total=total_pairs, event="start")

    for pair_index, (preset, tag, chroms) in enumerate(pair_batches, start=1):
        for chrom in chroms:
            chunks_path = target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, ".chunks.json")
            if not chunks_path.exists():
                track_warning(
                    warning_counts,
                    "missing_chunks",
                    f"{preset}/{tag}/chr{chrom}",
                    warning_samples,
                )
                completed_cells += 1
                continue

            payload = read_json(chunks_path)
            vardict_commit = payload.get("vardict_commit")
            if vardict_commit is None:
                track_warning(
                    warning_counts,
                    "missing_vardict_commit",
                    str(chunks_path),
                    warning_samples,
                )
            elif vardict_commit != live_commit:
                raise SystemExit(
                    f"ERROR: provenance mismatch for {preset}/{tag}/chr{chrom}: "
                    f"vardict_commit={vardict_commit} live={live_commit}"
                )

            expected_flags = (
                f"--output-only --config {preset} --tags {','.join(grouped_tags[preset])} "
                f"--sweep-bed-root {args.sweep_bed_root}"
            )
            optional_checks = {
                "generator_flags": expected_flags,
                "preset": preset,
                "bed_sha256": bed_sha256(args.sweep_bed_root, tag),
            }
            for key, expected_value in optional_checks.items():
                actual_value = payload.get(key)
                if actual_value is None:
                    track_warning(
                        warning_counts,
                        f"missing_{key}",
                        str(chunks_path),
                        warning_samples,
                    )
                elif str(actual_value) != str(expected_value):
                    track_warning(
                        warning_counts,
                        f"mismatch_{key}",
                        f"{chunks_path} expected={expected_value} actual={actual_value}",
                        warning_samples,
                    )
            completed_cells += 1

        emit_status(
            "provenance",
            started_at=provenance_started_at,
            completed=pair_index,
            total=total_pairs,
            active=f"{preset}/{tag}",
            event="pair-complete",
            detail=f"cells={completed_cells}/{total_cells} warnings={sum(warning_counts.values())}",
        )

    emit_warning_summary("provenance", warning_counts, started_at=provenance_started_at, samples=warning_samples)
    emit_status(
        "provenance",
        started_at=provenance_started_at,
        completed=total_pairs,
        total=total_pairs,
        event="complete",
        detail=f"cells={total_cells}/{total_cells} warnings={sum(warning_counts.values())}",
    )


def parse_failure_chroms(output: str, expected_chroms: list[str]) -> list[str]:
    chroms = []
    for match in re.finditer(r"Mismatch in [^:]+: ([^:\s]+):\d+-\d+", output):
        chrom = match.group(1)
        if chrom in expected_chroms and chrom not in chroms:
            chroms.append(chrom)
    return chroms or list(expected_chroms)


def failure_report_path(args: argparse.Namespace) -> Path:
    return args.report_dir / FAILURE_REPORT.name


def last_pass_path(args: argparse.Namespace) -> Path:
    return args.report_dir / LAST_PASS.name


def write_failure_report(
    args: argparse.Namespace,
    matrix: list[tuple[str, str, str]],
    commit: str,
    failures: list[dict],
    *,
    started_at: float,
    stopped_after: str | None = None,
) -> int:
    report_path = failure_report_path(args)
    payload = failure_report_base(matrix, commit)
    payload["failures"] = failures
    write_json_atomic(report_path, payload)

    detail_parts = [f"failures={len(failures)}", f"report={report_path}"]
    if stopped_after is not None:
        detail_parts.append(f"stopped_after={stopped_after}")
    emit_status(
        "done",
        started_at=started_at,
        event="failed",
        detail=" ".join(detail_parts),
    )
    return 1


def failure_report_base(matrix: list[tuple[str, str, str]], commit: str) -> dict:
    return {
        "schema_version": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "vardictjava_commit": commit,
        "matrix": [
            {"preset": preset, "tag": tag, "chrom": chrom}
            for preset, tag, chrom in matrix
        ],
    }


def run_tests_and_report(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> int:
    report_commit = live_vardictjava_commit()
    failures: list[dict] = []
    sweep_bed_root = args.sweep_bed_root.resolve()
    tests_started_at = time.monotonic()
    chroms_by_pair: dict[tuple[str, str], list[str]] = {}
    for preset, tag, chrom in matrix:
        chroms_by_pair.setdefault((preset, tag), [])
        if chrom not in chroms_by_pair[(preset, tag)]:
            chroms_by_pair[(preset, tag)].append(chrom)

    pairs = grouped_pairs(matrix)
    total_pairs = len(pairs)
    emit_status("tests", started_at=tests_started_at, completed=0, total=total_pairs, event="start")

    for pair_index, (preset, tag) in enumerate(pairs, start=1):
        test_name = sweep_test_selector(tag)
        reproducer = sweep_test_reproducer(args, preset, sweep_bed_root, test_name)
        env = dict(os.environ)
        env["VARDICT_E2E_SWEEP_CONFIG"] = preset
        env["VARDICT_E2E_SWEEP_BED_ROOT"] = str(sweep_bed_root)
        env["CI"] = "true"
        cmd = sweep_test_command(args, test_name)
        emit_status(
            "tests",
            started_at=tests_started_at,
            completed=pair_index - 1,
            total=total_pairs,
            active=f"{preset}/{tag}",
            event="pair-start",
            detail=f"cmd={reproducer}",
        )
        result = run_streaming_subprocess(cmd, cwd=PROJECT_ROOT, env=env)
        combined_output = "\n".join(part for part in (result.stdout, result.stderr) if part)
        stderr_source = result.stderr if result.stderr else combined_output
        stderr_tail = stderr_source.splitlines()[-50:]
        running_match = RUNNING_TESTS_RE.search(result.stdout)
        if running_match and int(running_match.group(1)) == 0:
            for chrom in parse_failure_chroms(combined_output, chroms_by_pair[(preset, tag)]):
                failures.append(
                    {
                        "preset": preset,
                        "tag": tag,
                        "chrom": chrom,
                        "cargo_test_name": test_name,
                        "reproducer_cmd": reproducer,
                        "stderr_tail": stderr_tail,
                        "exit_code": result.returncode,
                        "message": "selector matched 0 tests (likely module-path drift)",
                    }
                )
            emit_status(
                "tests",
                started_at=tests_started_at,
                completed=pair_index,
                total=total_pairs,
                active=f"{preset}/{tag}",
                event="pair-fail",
                detail=f"exit={result.returncode} fail_fast=1 reason=selector-matched-0-tests",
            )
            return write_failure_report(
                args,
                matrix,
                report_commit,
                failures,
                started_at=tests_started_at,
                stopped_after=f"{preset}/{tag}",
            )

        if result.returncode == 0:
            emit_status(
                "tests",
                started_at=tests_started_at,
                completed=pair_index,
                total=total_pairs,
                active=f"{preset}/{tag}",
                event="pair-pass",
            )
            continue

        cap_reached = "MAX_FAILURES cap" in combined_output
        for chrom in parse_failure_chroms(combined_output, chroms_by_pair[(preset, tag)]):
            failures.append(
                {
                    "preset": preset,
                    "tag": tag,
                    "chrom": chrom,
                    "cargo_test_name": test_name,
                    "reproducer_cmd": reproducer,
                    "stderr_tail": stderr_tail,
                    "exit_code": result.returncode,
                }
            )
        emit_status(
            "tests",
            started_at=tests_started_at,
            completed=pair_index,
            total=total_pairs,
            active=f"{preset}/{tag}",
            event="pair-fail",
            detail=(
                f"exit={result.returncode} fail_fast=1 "
                f"rust_max_failures=10 cap_reached={1 if cap_reached else 0}"
            ),
        )
        return write_failure_report(
            args,
            matrix,
            report_commit,
            failures,
            started_at=tests_started_at,
            stopped_after=f"{preset}/{tag}",
        )

    report_path = failure_report_path(args)

    pass_payload = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "vardictjava_commit": report_commit,
        "matrix": [
            {"preset": preset, "tag": tag, "chrom": chrom}
            for preset, tag, chrom in matrix
        ],
    }
    args.report_dir.mkdir(parents=True, exist_ok=True)
    write_json_atomic(last_pass_path(args), pass_payload)
    green_report = failure_report_base(matrix, report_commit)
    green_report["failures"] = []
    write_json_atomic(report_path, green_report)
    emit_status(
        "done",
        started_at=tests_started_at,
        event="passed",
        detail=(
            f"shards={len(matrix)} presets={len({preset for preset, _, _ in matrix})} "
            f"tags={len({tag for _, tag, _ in matrix})} chroms={len({chrom for _, _, chrom in matrix})} "
            f"report={report_path}"
        ),
    )
    return 0


def run_unstage(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> int:
    removed = 0
    skipped = 0
    actions: list[str] = []
    for preset, tag, chrom in matrix:
        for suffix in (".tsv.zst", ".chunks.json"):
            target = target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, suffix)
            actions.append(str(target))
            if not target.exists() and not target.is_symlink():
                skipped += 1
                continue
            if not target.is_symlink():
                print(f"WARNING: {target} is a regular file, not a symlink; skipping.", file=sys.stderr)
                skipped += 1
                continue
            if args.dry_run:
                continue
            target.unlink()
            removed += 1

    if args.dry_run:
        print("Planned unstage removals:")
        for action in actions:
            print(f"  {action}")
        if MANIFEST_SNAPSHOT.is_file():
            print(f"  restore manifest cache_entries from {MANIFEST_SNAPSHOT}")
        else:
            print("  no manifest snapshot present to restore")
        print(f"--unstage dry-run complete: removed=0 skipped={skipped}")
        return 0

    if MANIFEST_SNAPSHOT.is_file():
        snap = read_json(MANIFEST_SNAPSHOT)
        data = read_json(CANONICAL_MANIFEST) if CANONICAL_MANIFEST.is_file() else {}
        data["cache_entries"] = snap.get("cache_entries", {})
        write_json(CANONICAL_MANIFEST, data)
        MANIFEST_SNAPSHOT.unlink()
        print("Restored manifest cache_entries from snapshot.")
    else:
        print("WARNING: no manifest snapshot to restore.", file=sys.stderr)

    if MERGE_PRESERVE_WORK.exists():
        MERGE_PRESERVE_WORK.unlink()

    print(f"--unstage complete: removed={removed} skipped={skipped}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = normalize_args(parser.parse_args(argv), parser)
    enforce_chr1_scope_guard(args)
    matrix = resolve_matrix(args)
    print_matrix(matrix)
    if args.dry_run and not args.unstage:
        print_planned_actions(args, matrix)
        return 0

    with manifest_lock():
        if args.unstage:
            return run_unstage(args, matrix)
        with progress_log(args):
            emit_status(
                "matrix",
                event="resolved",
                detail=(
                    f"cells={len(matrix)} pairs={len(grouped_pairs(matrix))} "
                    f"report_dir={args.report_dir} progress_log={progress_log_path(args)}"
                ),
            )
            try:
                run_stage(args, matrix)
                run_provenance_check(args, matrix)
                return run_tests_and_report(args, matrix)
            except SystemExit as exc:
                emit_status(
                    "done",
                    event="failed",
                    detail=f"reason={exc}",
                )
                raise


if __name__ == "__main__":
    sys.exit(main())