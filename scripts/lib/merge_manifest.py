"""Merge E2E sweep cache metadata into a manifest.

This module started as the A1 lift of the single-config germline helper from
``scripts/gen_e2e_sweep_golden.sh`` and now also owns the somatic and
multi-config cache-entry mergers used by the sweep golden generator.
"""

import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


BAM_PATHS = {
    "hg002": "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    "hg005_exome": "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG005-EEogPU_v02-KIT-Av5_CGCATACA_L008.posiSrt.markDup.bam",
    "na12878_exome": "testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam",
    "na12878_lowcov": "testdata/NA12878.mapped.ILLUMINA.bwa.CEU.low_coverage.20121211.bam",
}
PAIR_PATHS = {
    "wes_il_pair": (
        "testdata/WES_IL_T_1.bwa.dedup.bam",
        "testdata/WES_IL_N_1.bwa.dedup.bam",
    ),
}
REFERENCE_FAI = "testdata/hs37d5.fa.fai"
SOMATIC_REFERENCE_FAI = "testdata/GRCh38.d1.vd1.fa.fai"
SOMATIC_REFERENCE_FA = "testdata/GRCh38.d1.vd1.fa"


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _sha256_concat(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in paths:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def _deterministic_missing_sha256(*labels: str) -> str:
    return hashlib.sha256("|".join(labels).encode("utf-8")).hexdigest()


def _env_flag_is_truthy(value: str | None) -> bool:
    return value is not None and value.strip().lower() in {"1", "true", "yes"}


def _expected_fixture_path(fixture_output_root: Path, config_name: str, tag: str, chrom: str) -> Path:
    if config_name == "default":
        return fixture_output_root / chrom / f"{tag}_{chrom}.tsv.zst"
    return fixture_output_root / config_name / chrom / f"{tag}_{chrom}.tsv.zst"


def _discover_bed_paths(project_root: Path, sweep_bed_root: Path, tag: str) -> list[Path]:
    return sorted(Path(path) for path in glob.glob(str(project_root / sweep_bed_root / tag / "*.bed")))


def _fixture_root(project_root: Path) -> Path:
    """Resolve the sweep fixture root.

    Honors ``VARDICT_E2E_SWEEP_FIXTURE_ROOT``; falls back to ``tmp/sweep_fixtures`` so
    default behavior is byte-identical when the env var is unset. Relative values
    resolve under ``project_root``.
    """

    raw = os.environ.get("VARDICT_E2E_SWEEP_FIXTURE_ROOT")
    if raw is None or not raw.strip():
        return project_root / "tmp/sweep_fixtures"
    candidate = Path(raw).expanduser()
    if not candidate.is_absolute():
        candidate = project_root / candidate
    return candidate


def _default_manifest_path(project_root: Path) -> Path:
    return _fixture_root(project_root) / "manifest.json"


def _default_preserve_path(project_root: Path) -> Path:
    return _fixture_root(project_root) / ".manifest.cache_entries.before.json"


def _resolve_manifest_path(project_root: Path, manifest_path: Path | str | None) -> Path:
    if manifest_path is None:
        return _default_manifest_path(project_root)
    return Path(manifest_path)


def _resolve_preserve_path(project_root: Path, preserve_path: Path | str | None) -> Path:
    if preserve_path is None:
        return _default_preserve_path(project_root)
    return Path(preserve_path)


def _load_manifest(manifest_path: Path, preserve_path: Path) -> tuple[dict, dict]:
    if not manifest_path.exists():
        raise SystemExit(f"ERROR: manifest not found after generator run: {manifest_path}")

    with manifest_path.open("r", encoding="utf-8") as handle:
        manifest = json.load(handle)

    preserved_entries = {}
    if preserve_path.exists():
        with preserve_path.open("r", encoding="utf-8") as handle:
            preserved_entries = json.load(handle)

    return manifest, preserved_entries


def _resolve_vardict_commit(manifest: dict, project_root: Path) -> str:
    vardict_commit = manifest.get("vardictjava_commit")
    if vardict_commit:
        return vardict_commit
    return subprocess.run(
        ["git", "-C", str(project_root / "VarDictJava"), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def _write_manifest(manifest: dict, manifest_path: Path, preserve_path: Path) -> None:
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=manifest_path.parent, delete=False) as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")
        temp_name = handle.name

    os.replace(temp_name, manifest_path)
    if preserve_path.exists():
        preserve_path.unlink()


def _reference_sha256(project_root: Path, primary: str, fallback: str | None = None) -> str:
    primary_path = project_root / primary
    if primary_path.exists():
        return _sha256_file(primary_path)
    if fallback is not None:
        fallback_path = project_root / fallback
        if fallback_path.exists():
            return _sha256_file(fallback_path)
    return _deterministic_missing_sha256(primary, fallback or "")


def _germline_bam_stat(project_root: Path, tag: str) -> list[dict]:
    bam_rel = BAM_PATHS[tag]
    bam_path = project_root / bam_rel
    return [{
        "path": bam_rel,
        "size": bam_path.stat().st_size,
        "mtime_unix": int(bam_path.stat().st_mtime),
    }]


def _somatic_bam_stat(project_root: Path, tag: str) -> list[dict]:
    tumor_rel, normal_rel = PAIR_PATHS[tag]
    tumor_path = project_root / tumor_rel
    normal_path = project_root / normal_rel
    return [
        {
            "path": tumor_rel,
            "size": tumor_path.stat().st_size,
            "mtime_unix": int(tumor_path.stat().st_mtime),
            "role": "tumor",
        },
        {
            "path": normal_rel,
            "size": normal_path.stat().st_size,
            "mtime_unix": int(normal_path.stat().st_mtime),
            "role": "normal",
        },
    ]


def _resolve_project_root(project_root: str | None) -> str:
    if project_root is not None:
        return project_root
    return subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def _validate_manifest_only_fixtures(
    config_name: str,
    tags: list[str],
    project_root: Path,
    sweep_bed_root: Path,
    fixture_output_root: Path | str | None,
    fixture_chroms: list[str] | None = None,
) -> None:
    if fixture_output_root is None:
        fixture_output_root = _fixture_root(project_root) / "output"
    else:
        fixture_output_root = Path(fixture_output_root)
        if not fixture_output_root.is_absolute():
            fixture_output_root = project_root / fixture_output_root
    fixture_output_root = fixture_output_root.resolve()
    scoped_chroms = None
    if fixture_chroms is not None:
        scoped_chroms = []
        for chrom in fixture_chroms:
            chrom = chrom.strip()
            if chrom and chrom not in scoped_chroms:
                scoped_chroms.append(chrom)

    missing = []
    for tag in tags:
        bed_paths = _discover_bed_paths(project_root, sweep_bed_root, tag)
        # A tag with zero BEDs keeps A1's SystemExit path; no chroms can be enumerated.
        if not bed_paths:
            raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
        if scoped_chroms is None:
            chroms = [bed_path.stem for bed_path in bed_paths]
        else:
            bed_root = project_root / sweep_bed_root / tag
            available_chroms = {bed_path.stem for bed_path in bed_paths}
            missing_beds = [bed_root / f"{chrom}.bed" for chrom in scoped_chroms if chrom not in available_chroms]
            if missing_beds:
                message = "Missing BED files for --manifest-only scoped chroms:\n" + "\n".join(
                    str(path) for path in missing_beds
                )
                raise FileNotFoundError(message)
            chroms = scoped_chroms
        for chrom in chroms:
            expected = _expected_fixture_path(fixture_output_root, config_name, tag, chrom)
            if not expected.exists():
                missing.append(expected)

    if missing:
        message = "Missing fixture outputs for --manifest-only:\n" + "\n".join(
            str(path) for path in missing
        )
        raise FileNotFoundError(message)


def merge_cache_entries(
    config_name: str,
    tags_csv: str,
    logical_flags: str,
    project_root: Path | str,
    sweep_bed_root: Path | str,
    manifest_path: Path | str | None = None,
    preserve_path: Path | str | None = None,
    manifest_only: bool = False,
    fixture_output_root: Path | str | None = None,
    fixture_chroms: list[str] | None = None,
) -> None:
    """Merge per-tag cache metadata for one germline config into manifest.json."""
    project_root = Path(project_root)
    sweep_bed_root = Path(sweep_bed_root)
    manifest_path = _resolve_manifest_path(project_root, manifest_path)
    preserve_path = _resolve_preserve_path(project_root, preserve_path)

    logical_flags = " ".join(logical_flags.split())
    tags = [tag for tag in tags_csv.split(",") if tag]

    if manifest_only:
        _validate_manifest_only_fixtures(
            config_name=config_name,
            tags=tags,
            project_root=project_root,
            sweep_bed_root=sweep_bed_root,
            fixture_output_root=fixture_output_root,
            fixture_chroms=fixture_chroms,
        )

    manifest, preserved_entries = _load_manifest(manifest_path, preserve_path)
    reference_sha256 = _reference_sha256(project_root, REFERENCE_FAI)
    generator_flags_hash = hashlib.sha256(logical_flags.encode("utf-8")).hexdigest()
    vardict_commit = _resolve_vardict_commit(manifest, project_root)

    cache_entries = dict(preserved_entries)
    for tag in tags:
        bed_paths = _discover_bed_paths(project_root, sweep_bed_root, tag)
        if not bed_paths:
            raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
        key = f"{config_name}:{tag}"
        cache_entries[key] = {
            "config": config_name,
            "tag": tag,
            "bed_sha256": _sha256_concat(bed_paths),
            "bam_stat": _germline_bam_stat(project_root, tag),
            "reference_sha256": reference_sha256,
            "generator_flags_hash": generator_flags_hash,
            "vardictjava_commit": vardict_commit,
        }

    manifest["cache_entries"] = cache_entries
    _write_manifest(manifest, manifest_path, preserve_path)


def merge_cache_entries_somatic(
    config_name: str,
    tags_csv: str,
    logical_flags: str,
    project_root: Path | str,
    sweep_bed_root: Path | str,
    manifest_path: Path | str | None = None,
    preserve_path: Path | str | None = None,
) -> None:
    """Merge per-tag cache metadata for one somatic config into manifest.json."""
    project_root = Path(project_root)
    sweep_bed_root = Path(sweep_bed_root)
    manifest_path = _resolve_manifest_path(project_root, manifest_path)
    preserve_path = _resolve_preserve_path(project_root, preserve_path)

    logical_flags = " ".join(logical_flags.split())
    tags = [tag for tag in tags_csv.split(",") if tag]

    manifest, preserved_entries = _load_manifest(manifest_path, preserve_path)
    reference_sha256 = _reference_sha256(project_root, SOMATIC_REFERENCE_FAI, SOMATIC_REFERENCE_FA)
    generator_flags_hash = hashlib.sha256(logical_flags.encode("utf-8")).hexdigest()
    vardict_commit = _resolve_vardict_commit(manifest, project_root)

    cache_entries = dict(preserved_entries)
    for tag in tags:
        bed_paths = _discover_bed_paths(project_root, sweep_bed_root, tag)
        if not bed_paths:
            raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
        key = f"{config_name}:somatic:{tag}"
        cache_entries[key] = {
            "config": config_name,
            "tag": tag,
            "bed_sha256": _sha256_concat(bed_paths),
            "bam_stat": _somatic_bam_stat(project_root, tag),
            "reference_sha256": reference_sha256,
            "generator_flags_hash": generator_flags_hash,
            "vardictjava_commit": vardict_commit,
        }

    manifest["cache_entries"] = cache_entries
    _write_manifest(manifest, manifest_path, preserve_path)


def merge_cache_entries_many(
    config_names_csv: str,
    tags_csv: str,
    project_root: Path | str,
    sweep_bed_root: Path | str,
    manifest_path: Path | str | None = None,
    preserve_path: Path | str | None = None,
) -> None:
    """Merge per-tag cache metadata for multiple germline configs into manifest.json."""
    project_root = Path(project_root)
    sweep_bed_root = Path(sweep_bed_root)
    manifest_path = _resolve_manifest_path(project_root, manifest_path)
    preserve_path = _resolve_preserve_path(project_root, preserve_path)

    configs = [config for config in config_names_csv.split(",") if config]
    tags = [tag for tag in tags_csv.split(",") if tag]

    manifest, preserved_entries = _load_manifest(manifest_path, preserve_path)
    reference_sha256 = _reference_sha256(project_root, REFERENCE_FAI)
    vardict_commit = _resolve_vardict_commit(manifest, project_root)

    cache_entries = dict(preserved_entries)
    for config_name in configs:
        for tag in tags:
            bed_paths = _discover_bed_paths(project_root, sweep_bed_root, tag)
            if not bed_paths:
                raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
            logical_flags = f"--output-only --config {config_name} --tags {tag} --sweep-bed-root {sweep_bed_root}"
            generator_flags_hash = hashlib.sha256(logical_flags.encode("utf-8")).hexdigest()
            cache_entries[f"{config_name}:{tag}"] = {
                "config": config_name,
                "tag": tag,
                "bed_sha256": _sha256_concat(bed_paths),
                "bam_stat": _germline_bam_stat(project_root, tag),
                "reference_sha256": reference_sha256,
                "generator_flags_hash": generator_flags_hash,
                "vardictjava_commit": vardict_commit,
            }

    manifest["cache_entries"] = cache_entries
    _write_manifest(manifest, manifest_path, preserve_path)


def merge_cache_entries_somatic_many(
    config_names_csv: str,
    tags_csv: str,
    project_root: Path | str,
    sweep_bed_root: Path | str,
    manifest_path: Path | str | None = None,
    preserve_path: Path | str | None = None,
) -> None:
    """Merge per-tag cache metadata for multiple somatic configs into manifest.json."""
    project_root = Path(project_root)
    sweep_bed_root = Path(sweep_bed_root)
    manifest_path = _resolve_manifest_path(project_root, manifest_path)
    preserve_path = _resolve_preserve_path(project_root, preserve_path)

    configs = [config for config in config_names_csv.split(",") if config]
    tags = [tag for tag in tags_csv.split(",") if tag]

    manifest, preserved_entries = _load_manifest(manifest_path, preserve_path)
    reference_sha256 = _reference_sha256(project_root, SOMATIC_REFERENCE_FAI, SOMATIC_REFERENCE_FA)
    vardict_commit = _resolve_vardict_commit(manifest, project_root)

    cache_entries = dict(preserved_entries)
    for config_name in configs:
        for tag in tags:
            bed_paths = _discover_bed_paths(project_root, sweep_bed_root, tag)
            if not bed_paths:
                raise SystemExit(f"ERROR: no BED files found for {tag} under {sweep_bed_root}")
            logical_flags = (
                f"--output-only --config {config_name} --pair-tags {tag} --tags  --sweep-bed-root {sweep_bed_root}"
            )
            generator_flags_hash = hashlib.sha256(logical_flags.encode("utf-8")).hexdigest()
            cache_entries[f"{config_name}:somatic:{tag}"] = {
                "config": config_name,
                "tag": tag,
                "bed_sha256": _sha256_concat(bed_paths),
                "bam_stat": _somatic_bam_stat(project_root, tag),
                "reference_sha256": reference_sha256,
                "generator_flags_hash": generator_flags_hash,
                "vardictjava_commit": vardict_commit,
            }

    manifest["cache_entries"] = cache_entries
    _write_manifest(manifest, manifest_path, preserve_path)

def _add_path_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--project-root", dest="project_root")
    parser.add_argument("--sweep-bed-root", dest="sweep_bed_root")
    parser.add_argument("--manifest-path", dest="manifest_path")
    parser.add_argument("--preserve-path", dest="preserve_path")


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Merge E2E sweep cache metadata into a manifest."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    cache_entries = subparsers.add_parser(
        "cache-entries",
        help="Merge germline cache entries for one config.",
    )
    cache_entries.add_argument("--config", dest="config_name")
    cache_entries.add_argument("--tags", dest="tags_csv")
    cache_entries.add_argument("--logical-flags", dest="logical_flags")
    _add_path_arguments(cache_entries)
    cache_entries.add_argument("--manifest-only", action="store_true", default=False)
    cache_entries.add_argument(
        "--fixture-output-root",
        dest="fixture_output_root",
        help="Directory containing existing fixture outputs for --manifest-only.",
    )
    cache_entries.add_argument(
        "--fixture-chroms",
        dest="fixture_chroms_csv",
        help="Comma-separated chrom stems to validate under --fixture-output-root for scoped staging.",
    )

    cache_entries_somatic = subparsers.add_parser(
        "cache-entries-somatic",
        help="Merge somatic cache entries for one config.",
    )
    cache_entries_somatic.add_argument("--config", dest="config_name", required=True)
    cache_entries_somatic.add_argument("--tags", dest="tags_csv", required=True)
    cache_entries_somatic.add_argument("--logical-flags", dest="logical_flags", required=True)
    _add_path_arguments(cache_entries_somatic)

    cache_entries_many = subparsers.add_parser(
        "cache-entries-many",
        help="Merge germline cache entries for multiple configs.",
    )
    cache_entries_many.add_argument("--configs", dest="config_names_csv", required=True)
    cache_entries_many.add_argument("--tags", dest="tags_csv", required=True)
    _add_path_arguments(cache_entries_many)

    cache_entries_somatic_many = subparsers.add_parser(
        "cache-entries-somatic-many",
        help="Merge somatic cache entries for multiple configs.",
    )
    cache_entries_somatic_many.add_argument("--configs", dest="config_names_csv", required=True)
    cache_entries_somatic_many.add_argument("--tags", dest="tags_csv", required=True)
    _add_path_arguments(cache_entries_somatic_many)

    return parser


def main(argv=None) -> None:
    parser = _build_parser()
    if argv is None:
        argv = sys.argv[1:]
    else:
        argv = list(argv)
    if argv in (["--help"], ["-h"]):
        parser.print_help()
        return
    if argv and argv[0].startswith("--"):
        argv = ["cache-entries"] + list(argv)

    args = parser.parse_args(argv)

    if args.command == "cache-entries":
        config_name = args.config_name or os.environ.get("CONFIG_NAME")
        tags_csv = args.tags_csv or os.environ.get("TAGS_CSV")
        logical_flags = args.logical_flags
        if logical_flags is None:
            logical_flags = os.environ.get("LOGICAL_FLAGS")

        project_root = args.project_root or os.environ.get("PROJECT_ROOT")
        project_root = _resolve_project_root(project_root)
        sweep_bed_root = args.sweep_bed_root or os.environ.get("SWEEP_BED_ROOT") or "tmp/sweep_beds"
        manifest_path = args.manifest_path or os.environ.get("MANIFEST_PATH")
        preserve_path = args.preserve_path or os.environ.get("PRESERVE_PATH")
        manifest_only = args.manifest_only or _env_flag_is_truthy(os.environ.get("MANIFEST_ONLY"))
        fixture_output_root = args.fixture_output_root or os.environ.get("FIXTURE_OUTPUT_ROOT")
        fixture_chroms_csv = args.fixture_chroms_csv or os.environ.get("FIXTURE_CHROMS")
        fixture_chroms = None
        if fixture_chroms_csv:
            fixture_chroms = [chrom for chrom in fixture_chroms_csv.split(",") if chrom]

        if config_name is None:
            parser.error("--config is required when CONFIG_NAME is unset")
        if tags_csv is None:
            parser.error("--tags is required when TAGS_CSV is unset")
        if logical_flags is None:
            parser.error("--logical-flags is required when LOGICAL_FLAGS is unset")

        merge_cache_entries(
            config_name=config_name,
            tags_csv=tags_csv,
            logical_flags=logical_flags,
            project_root=project_root,
            sweep_bed_root=sweep_bed_root,
            manifest_path=manifest_path,
            preserve_path=preserve_path,
            manifest_only=manifest_only,
            fixture_output_root=fixture_output_root,
            fixture_chroms=fixture_chroms,
        )
        return

    project_root = _resolve_project_root(args.project_root)
    sweep_bed_root = args.sweep_bed_root or "tmp/sweep_beds"

    if args.command == "cache-entries-somatic":
        merge_cache_entries_somatic(
            config_name=args.config_name,
            tags_csv=args.tags_csv,
            logical_flags=args.logical_flags,
            project_root=project_root,
            sweep_bed_root=sweep_bed_root,
            manifest_path=args.manifest_path,
            preserve_path=args.preserve_path,
        )
        return

    if args.command == "cache-entries-many":
        merge_cache_entries_many(
            config_names_csv=args.config_names_csv,
            tags_csv=args.tags_csv,
            project_root=project_root,
            sweep_bed_root=sweep_bed_root,
            manifest_path=args.manifest_path,
            preserve_path=args.preserve_path,
        )
        return

    merge_cache_entries_somatic_many(
        config_names_csv=args.config_names_csv,
        tags_csv=args.tags_csv,
        project_root=project_root,
        sweep_bed_root=sweep_bed_root,
        manifest_path=args.manifest_path,
        preserve_path=args.preserve_path,
    )


if __name__ == "__main__":
    main()
