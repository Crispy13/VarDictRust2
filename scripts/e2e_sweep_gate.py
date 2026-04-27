"""E2E sweep parity gate (manual-only).

Stages existing fixtures from --fixture-source into tmp/sweep_fixtures/output/,
populates manifest.json cache_entries via scripts.lib.merge_manifest, and runs
scoped parity_e2e_sweep cargo tests. Produces parity-failure-report.json on red.

Stdlib only.
"""
from __future__ import annotations

import argparse
import csv
import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
import time
from contextlib import contextmanager
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent
CANONICAL_OUTPUT_ROOT = PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output"
CANONICAL_MANIFEST = PROJECT_ROOT / "tmp" / "sweep_fixtures" / "manifest.json"
LOCK_FILE = PROJECT_ROOT / "tmp" / "sweep_fixtures" / ".manifest.lock"
MANIFEST_SNAPSHOT = PROJECT_ROOT / "tmp" / "sweep_fixtures" / ".manifest.cache_entries.before.json"
PARITY_ITERATION_DIR = PROJECT_ROOT / "tmp" / "parity-iteration"
FAILURE_REPORT = PARITY_ITERATION_DIR / "parity-failure-report.json"
LAST_PASS = PARITY_ITERATION_DIR / "last-pass.json"
PRESETS_TSV = PROJECT_ROOT / "scripts" / "config_presets.tsv"
SWEEP_BED_ROOT = PROJECT_ROOT / "tmp" / "sweep_beds"
DEFAULT_TAGS = ("hg002", "na12878_exome", "na12878_lowcov")
MERGE_PRESERVE_WORK = PROJECT_ROOT / "tmp" / "sweep_fixtures" / ".manifest.cache_entries.gate_working.json"


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
            "Chrom scoping is enforced through tmp/sweep_beds/<tag>/*.bed. The gate "
            "refuses extra BED chroms unless --allow-extra-beds is set."
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
        help="Warn instead of failing when tmp/sweep_beds/<tag>/ contains chroms outside the matrix.",
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
    args.report_dir = Path(args.report_dir).expanduser().resolve()
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
        print(f"  cargo test parity_e2e_sweep_{tag} with VARDICT_E2E_SWEEP_CONFIG={preset}")


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


def bed_sha256(tag: str) -> str:
    bed_paths = sorted((SWEEP_BED_ROOT / tag).glob("*.bed"))
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


def validate_bed_scope(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> None:
    grouped = grouped_chroms_by_tag(matrix)
    errors: list[str] = []
    warnings: list[str] = []
    for tag, chroms in grouped.items():
        bed_dir = SWEEP_BED_ROOT / tag
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


def stage_symlink(source: Path, target: Path, args: argparse.Namespace, staged_links: list[Path]) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink():
        existing = target.resolve()
        if existing == source.resolve():
            print(f"already linked: {target} -> {source}")
            return
        if not should_replace_link(target, existing, source, args):
            raise SystemExit(
                f"ERROR: {target} already points to {existing}; rerun with --force to replace it"
            )
        target.unlink()
    elif target.exists():
        raise SystemExit(f"ERROR: refusing to overwrite regular file at {target}")

    os.symlink(source.resolve(), target)
    staged_links.append(target)
    print(f"linked: {target} -> {source.resolve()}")


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

    snapshot_cache_entries()

    staged_links: list[Path] = []
    for preset, tag, chrom in matrix:
        tsv_source = source_path(fixture_source, preset, chrom, tag, ".tsv.zst")
        stage_symlink(tsv_source, target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, ".tsv.zst"), args, staged_links)

        chunks_source = source_path(fixture_source, preset, chrom, tag, ".chunks.json")
        if chunks_source.is_file():
            stage_symlink(
                chunks_source,
                target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, ".chunks.json"),
                args,
                staged_links,
            )
        else:
            print(
                f"WARNING: legacy shard, no chunks.json for {preset}/{tag}/chr{chrom} at {chunks_source}",
                file=sys.stderr,
            )

    for preset, tags in grouped_tags_by_preset(matrix).items():
        preserve_path = prepare_merge_preserve_file()
        logical_flags = f"--output-only --config {preset} --tags {','.join(tags)} --sweep-bed-root tmp/sweep_beds"
        merge_cache_entries(
            config_name=preset,
            tags_csv=",".join(tags),
            logical_flags=logical_flags,
            project_root=PROJECT_ROOT,
            sweep_bed_root=SWEEP_BED_ROOT,
            preserve_path=preserve_path,
            manifest_only=True,
            fixture_output_root=CANONICAL_OUTPUT_ROOT,
        )
        print(f"merged manifest cache_entries for {preset}: {','.join(tags)}")


def run_provenance_check(args: argparse.Namespace, matrix: list[tuple[str, str, str]]) -> None:
    del args
    live_commit = live_vardictjava_commit()
    grouped_tags = grouped_tags_by_preset(matrix)
    for preset, tag, chrom in matrix:
        chunks_path = target_path(CANONICAL_OUTPUT_ROOT, preset, chrom, tag, ".chunks.json")
        if not chunks_path.exists():
            print(
                f"WARNING: legacy shard, no chunks.json for {preset}/{tag}/chr{chrom}",
                file=sys.stderr,
            )
            continue

        payload = read_json(chunks_path)
        vardict_commit = payload.get("vardict_commit")
        if vardict_commit is None:
            print(
                f"WARNING: legacy shard, missing vardict_commit in {chunks_path}",
                file=sys.stderr,
            )
        elif vardict_commit != live_commit:
            raise SystemExit(
                f"ERROR: provenance mismatch for {preset}/{tag}/chr{chrom}: "
                f"vardict_commit={vardict_commit} live={live_commit}"
            )

        expected_flags = f"--output-only --config {preset} --tags {','.join(grouped_tags[preset])} --sweep-bed-root tmp/sweep_beds"
        optional_checks = {
            "generator_flags": expected_flags,
            "preset": preset,
            "bed_sha256": bed_sha256(tag),
        }
        for key, expected_value in optional_checks.items():
            actual_value = payload.get(key)
            if actual_value is None:
                print(f"WARNING: legacy shard, missing {key} in {chunks_path}", file=sys.stderr)
            elif str(actual_value) != str(expected_value):
                print(
                    f"WARNING: legacy shard, {key} mismatch in {chunks_path}: "
                    f"expected={expected_value} actual={actual_value}",
                    file=sys.stderr,
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
    chroms_by_pair: dict[tuple[str, str], list[str]] = {}
    for preset, tag, chrom in matrix:
        chroms_by_pair.setdefault((preset, tag), [])
        if chrom not in chroms_by_pair[(preset, tag)]:
            chroms_by_pair[(preset, tag)].append(chrom)

    for preset, tag in grouped_pairs(matrix):
        test_name = f"parity_e2e_sweep_{tag}"
        env = dict(os.environ)
        env["VARDICT_E2E_SWEEP_CONFIG"] = preset
        cmd = [
            "cargo",
            "test",
            "--profile",
            "debug-release",
            "--test",
            "parity_e2e_sweep",
            "--",
            "--include-ignored",
            "--exact",
            test_name,
            "--test-threads=1",
            *args.cargo_extra_arg,
        ]
        print(f"Running: VARDICT_E2E_SWEEP_CONFIG={preset} {' '.join(cmd)}")
        result = subprocess.run(
            cmd,
            cwd=PROJECT_ROOT,
            env=env,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            print(f"PASS: {preset}/{tag}")
            continue

        combined_output = "\n".join(part for part in (result.stdout, result.stderr) if part)
        stderr_tail = result.stderr.splitlines()[-50:]
        reproducer = (
            f"VARDICT_E2E_SWEEP_CONFIG={preset} cargo test --profile debug-release "
            f"--test parity_e2e_sweep -- --include-ignored --exact {test_name} --test-threads=1"
        )
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
        print(f"FAIL: {preset}/{tag} exit={result.returncode}")

    report_path = failure_report_path(args)
    if failures:
        payload = failure_report_base(matrix, report_commit)
        payload["failures"] = failures
        write_json_atomic(report_path, payload)
        return 1

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
    print(
        f"GATE_PASSED: {len(matrix)} shards green "
        f"(presets={len({preset for preset, _, _ in matrix})}, "
        f"tags={len({tag for _, tag, _ in matrix})}, chroms={len({chrom for _, _, chrom in matrix})})"
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
    matrix = resolve_matrix(args)
    print_matrix(matrix)
    if args.dry_run and not args.unstage:
        print_planned_actions(args, matrix)
        return 0

    with manifest_lock():
        if args.unstage:
            return run_unstage(args, matrix)
        run_stage(args, matrix)
        run_provenance_check(args, matrix)
        return run_tests_and_report(args, matrix)


if __name__ == "__main__":
    sys.exit(main())