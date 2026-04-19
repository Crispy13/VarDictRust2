#!/usr/bin/env python3

import argparse
import shlex
import subprocess
import sys
from pathlib import Path


DEFAULT_CONFIG = "default"
DEFAULT_SAMPLE_NAME = "test_sample"
DEFAULT_RUST_BIN = Path("target/debug-release/vardict_rs")
DEFAULT_JAVA_BIN = Path("VarDictJava/build/install/VarDict/bin/VarDict")
DEFAULT_OUTPUT_DIR = Path("tmp/dual_run")
DEFAULT_BATCH_FILE = Path("testdata/parity_regions.tsv")
CONFIG_PRESETS = Path("scripts/config_presets.tsv")
JAVA_BUILD_TIMEOUT_SECONDS = 600
RUN_TIMEOUT_SECONDS = 120
DIFF_PREVIEW_LIMIT = 20
PUSH_INDICES = [0, 1, 2, 3, 4, 35, 36, 37, 70, 71]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Run VarDictJava and vardict_rs on the same inputs, normalize TSV output, "
            "and report parity PASS/FAIL."
        )
    )
    mode_group = parser.add_mutually_exclusive_group(required=True)
    mode_group.add_argument("--region", help="Genomic region in chr:start-end format")
    mode_group.add_argument(
        "--batch",
        nargs="?",
        const=str(DEFAULT_BATCH_FILE),
        help=(
            "Run all regions from a 3-column TSV file. "
            "Defaults to testdata/parity_regions.tsv when provided without a path"
        ),
    )
    mode_group.add_argument(
        "--push-only",
        action="store_true",
        help="Run only the 10 push regions from tests/parity_e2e.rs",
    )
    parser.add_argument("--bam", help="Path to BAM file")
    parser.add_argument("--ref", help="Path to reference FASTA")
    parser.add_argument(
        "--config",
        default=DEFAULT_CONFIG,
        help="Config preset name from scripts/config_presets.tsv",
    )
    parser.add_argument(
        "--all-configs",
        action="store_true",
        help="Run all config presets in file order",
    )
    parser.add_argument(
        "--sample-name",
        default=DEFAULT_SAMPLE_NAME,
        help="Sample name passed to both implementations",
    )
    parser.add_argument(
        "--rust-bin",
        default=str(DEFAULT_RUST_BIN),
        help="Path to the vardict_rs binary",
    )
    parser.add_argument(
        "--java-bin",
        default=str(DEFAULT_JAVA_BIN),
        help="Path to the VarDictJava launcher",
    )
    parser.add_argument(
        "--output-dir",
        default=str(DEFAULT_OUTPUT_DIR),
        help="Directory for saved TSV and diff outputs",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print resolved commands and paths",
    )
    args = parser.parse_args()

    if args.region:
        if not args.bam or not args.ref:
            parser.error("--bam and --ref are required when --region is used")
    elif args.bam or args.ref:
        parser.error("--bam and --ref are only valid with --region")

    return args


def detect_project_root() -> Path:
    script_path = Path(__file__).resolve()
    for candidate in [script_path.parent] + list(script_path.parents):
        if (candidate / "Cargo.toml").is_file():
            return candidate
    raise SystemExit("ERROR: Could not detect project root from script location")


def resolve_path(project_root: Path, raw_path: str) -> Path:
    candidate = Path(raw_path)
    if candidate.is_absolute():
        return candidate.resolve()
    return (project_root / candidate).resolve()


def load_config_presets(preset_path: Path) -> list:
    try:
        content = preset_path.read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"ERROR: Failed to read config presets {preset_path}: {error}")

    presets = []
    for line_number, raw_line in enumerate(content.splitlines(), start=1):
        if not raw_line.strip():
            continue
        fields = raw_line.split("\t")
        if len(fields) != 3:
            raise SystemExit(
                "ERROR: Expected 3 tab-separated fields in {} at line {}: {}".format(
                    preset_path, line_number, raw_line
                )
            )
        preset_name, flags, _description = fields
        presets.append((preset_name, flags))

    return presets


def load_config_preset(preset_path: Path, name: str) -> str:
    presets = dict(load_config_presets(preset_path))

    if name not in presets:
        available = ", ".join(sorted(presets))
        raise SystemExit(
            "ERROR: Unknown config preset '{}'. Available presets: {}".format(name, available)
        )

    return presets[name]


def load_all_config_names(preset_path: Path) -> list:
    return [preset_name for preset_name, _flags in load_config_presets(preset_path)]


def load_regions_file(path: Path) -> list:
    try:
        content = path.read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"ERROR: Failed to read regions file {path}: {error}")

    regions = []
    for line_number, raw_line in enumerate(content.splitlines(), start=1):
        if not raw_line.strip():
            continue
        fields = raw_line.split("\t")
        if len(fields) != 3:
            raise SystemExit(
                "ERROR: Expected 3 tab-separated fields in {} at line {}: {}".format(
                    path, line_number, raw_line
                )
            )
        regions.append(tuple(fields))

    return regions


def is_executable_file(path: Path) -> bool:
    return path.is_file() and path.exists() and path.stat().st_mode & 0o111 != 0


def build_java_if_needed(java_bin: Path, project_root: Path, verbose: bool) -> None:
    default_java_bin = (project_root / DEFAULT_JAVA_BIN).resolve()
    if is_executable_file(java_bin):
        return

    if java_bin != default_java_bin:
        raise SystemExit(
            "ERROR: Java binary not found or not executable at {}. "
            "Custom --java-bin paths are not auto-built.".format(java_bin)
        )

    vardict_dir = (project_root / "VarDictJava").resolve()
    command = ["./gradlew", "installDist", "-q"]
    if verbose:
        print("Building VarDictJava: {} (cwd={})".format(shell_join(command), vardict_dir))
    try:
        completed = subprocess.run(
            command,
            cwd=str(vardict_dir),
            capture_output=True,
            text=True,
            check=False,
            timeout=JAVA_BUILD_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        raise SystemExit(
            "ERROR: Timed out building VarDictJava after {}s: {}".format(
                JAVA_BUILD_TIMEOUT_SECONDS, error
            )
        )

    if completed.returncode != 0:
        raise SystemExit(
            "ERROR: VarDictJava build failed with exit code {}\nSTDOUT:\n{}\nSTDERR:\n{}".format(
                completed.returncode,
                completed.stdout.strip(),
                completed.stderr.strip(),
            )
        )

    if not is_executable_file(java_bin):
        raise SystemExit(
            "ERROR: VarDictJava binary still missing after build: {}".format(java_bin)
        )


def ensure_rust_bin_exists(rust_bin: Path) -> None:
    if is_executable_file(rust_bin):
        return
    raise SystemExit(
        "ERROR: Rust binary not found or not executable at {}. "
        "Build it with: cargo build --profile debug-release".format(rust_bin)
    )


def run_impl(
    binary: Path,
    region: str,
    bam: Path,
    ref: Path,
    sample_name: str,
    extra_flags: str,
    is_java: bool,
    verbose: bool,
) -> str:
    thread_flag = ["-th", "1"] if is_java else ["--th", "1"]
    command = [
        str(binary),
        "-G",
        str(ref),
        "-b",
        str(bam),
        "-N",
        sample_name,
    ]
    command.extend(thread_flag)
    command.extend(["-R", region])
    command.extend(shlex.split(extra_flags))

    if verbose:
        label = "java" if is_java else "rust"
        print("Running {}: {}".format(label, shell_join(command)))

    try:
        completed = subprocess.run(
            command,
            capture_output=True,
            text=True,
            check=False,
            timeout=RUN_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        label = "Java" if is_java else "Rust"
        raise SystemExit(
            "ERROR: {} command timed out after {}s: {}".format(
                label, RUN_TIMEOUT_SECONDS, error
            )
        )

    if completed.returncode != 0:
        label = "Java" if is_java else "Rust"
        raise SystemExit(
            "ERROR: {} command failed with exit code {}\nCommand: {}\nSTDOUT:\n{}\nSTDERR:\n{}".format(
                label,
                completed.returncode,
                shell_join(command),
                completed.stdout.strip(),
                completed.stderr.strip(),
            )
        )

    return completed.stdout


def normalize_tsv(text: str) -> list:
    normalized = []
    for raw_line in text.splitlines():
        stripped = raw_line.rstrip()
        if not stripped:
            continue
        if stripped.startswith("#"):
            continue
        normalized.append(stripped)
    normalized.sort()
    return normalized


def compare_outputs(java_out: str, rust_out: str) -> tuple:
    java_lines = normalize_tsv(java_out)
    rust_lines = normalize_tsv(rust_out)
    if java_lines == rust_lines:
        return True, ""

    summary = [
        "Normalized TSV outputs differ.",
        "Java lines: {}".format(len(java_lines)),
        "Rust lines: {}".format(len(rust_lines)),
        "First differing normalized lines:",
    ]

    diff_count = 0
    max_len = max(len(java_lines), len(rust_lines))
    for index in range(max_len):
        java_line = java_lines[index] if index < len(java_lines) else "<missing>"
        rust_line = rust_lines[index] if index < len(rust_lines) else "<missing>"
        if java_line == rust_line:
            continue
        summary.append("[{}] java: {}".format(index, java_line))
        summary.append("[{}] rust: {}".format(index, rust_line))
        diff_count += 1
        if diff_count >= DIFF_PREVIEW_LIMIT:
            break

    if diff_count == 0:
        summary.append("No differing preview lines captured; line counts differ only.")

    return False, "\n".join(summary)


def save_outputs(
    output_dir: Path,
    region: str,
    config_name: str,
    java_out: str,
    rust_out: str,
    diff_text: str,
) -> tuple:
    output_dir.mkdir(parents=True, exist_ok=True)
    diff_dir = output_dir / "diffs"
    diff_dir.mkdir(parents=True, exist_ok=True)
    safe_region = region.replace(":", "_").replace("-", "_")
    java_path = output_dir / "{}_{}_java.tsv".format(safe_region, config_name)
    rust_path = output_dir / "{}_{}_rust.tsv".format(safe_region, config_name)
    diff_path = diff_dir / "{}_{}_diff.txt".format(safe_region, config_name)

    java_path.write_text(java_out, encoding="utf-8")
    rust_path.write_text(rust_out, encoding="utf-8")

    if diff_text:
        diff_path.write_text(diff_text + "\n", encoding="utf-8")
    elif diff_path.exists():
        diff_path.unlink()

    return java_path, rust_path, diff_path


def shell_join(command: list) -> str:
    return " ".join(shlex.quote(part) for part in command)


def run_comparison(
    region: str,
    bam_path: Path,
    ref_path: Path,
    config_name: str,
    extra_flags: str,
    args: argparse.Namespace,
    java_bin: Path,
    rust_bin: Path,
    output_dir: Path,
) -> dict:
    java_out = run_impl(
        binary=java_bin,
        region=region,
        bam=bam_path,
        ref=ref_path,
        sample_name=args.sample_name,
        extra_flags=extra_flags,
        is_java=True,
        verbose=args.verbose,
    )
    rust_out = run_impl(
        binary=rust_bin,
        region=region,
        bam=bam_path,
        ref=ref_path,
        sample_name=args.sample_name,
        extra_flags=extra_flags,
        is_java=False,
        verbose=args.verbose,
    )

    matches, diff_text = compare_outputs(java_out, rust_out)
    java_path, rust_path, diff_path = save_outputs(
        output_dir=output_dir,
        region=region,
        config_name=config_name,
        java_out=java_out,
        rust_out=rust_out,
        diff_text=diff_text,
    )

    return {
        "region": region,
        "config": config_name,
        "result": "PASS" if matches else "FAIL",
        "java_path": java_path,
        "rust_path": rust_path,
        "diff_path": diff_path if diff_text else None,
        "diff_text": diff_text,
        "error": "",
    }


def run_batch(
    regions: list,
    configs: list,
    args: argparse.Namespace,
    project_root: Path,
    java_bin: Path,
    rust_bin: Path,
    output_dir: Path,
    preset_path: Path,
) -> list:
    config_flags = {config_name: load_config_preset(preset_path, config_name) for config_name in configs}
    total = len(regions) * len(configs)
    results = []
    step = 0

    for region, bam_raw, ref_raw in regions:
        bam_path = resolve_path(project_root, bam_raw)
        ref_path = resolve_path(project_root, ref_raw)

        for config_name in configs:
            step += 1
            if args.verbose:
                print("[{}/{}] Running {} with config {}...".format(step, total, region, config_name))

            try:
                if not bam_path.is_file():
                    raise SystemExit("ERROR: BAM file not found: {}".format(bam_path))
                if not ref_path.is_file():
                    raise SystemExit("ERROR: Reference FASTA not found: {}".format(ref_path))

                result = run_comparison(
                    region=region,
                    bam_path=bam_path,
                    ref_path=ref_path,
                    config_name=config_name,
                    extra_flags=config_flags[config_name],
                    args=args,
                    java_bin=java_bin,
                    rust_bin=rust_bin,
                    output_dir=output_dir,
                )
            except SystemExit as error:
                result = {
                    "region": region,
                    "config": config_name,
                    "result": "ERROR",
                    "java_path": None,
                    "rust_path": None,
                    "diff_path": None,
                    "diff_text": "",
                    "error": str(error),
                }
            except Exception as error:  # pragma: no cover - defensive isolation for subprocess/file errors
                result = {
                    "region": region,
                    "config": config_name,
                    "result": "ERROR",
                    "java_path": None,
                    "rust_path": None,
                    "diff_path": None,
                    "diff_text": "",
                    "error": "ERROR: {}".format(error),
                }

            results.append(result)

    return results


def print_summary_table(results: list) -> int:
    print("=== Dual-Run Summary ===")

    region_width = max(len("region"), *(len(result["region"]) for result in results))
    config_width = max(len("config"), *(len(result["config"]) for result in results))
    print(
        "{:<{}} | {:<{}} | {}".format(
            "region",
            region_width,
            "config",
            config_width,
            "result",
        )
    )

    pass_count = 0
    fail_count = 0
    error_count = 0
    for result in results:
        print(
            "{:<{}} | {:<{}} | {}".format(
                result["region"],
                region_width,
                result["config"],
                config_width,
                result["result"],
            )
        )
        if result["result"] == "PASS":
            pass_count += 1
        elif result["result"] == "FAIL":
            fail_count += 1
        else:
            error_count += 1

    print(
        "Total: {} PASS, {} FAIL, {} ERROR out of {}".format(
            pass_count,
            fail_count,
            error_count,
            len(results),
        )
    )

    for result in results:
        if result["result"] == "FAIL" and result["diff_path"]:
            print("DIFF {} [{}]: {}".format(result["region"], result["config"], result["diff_path"]))
        if result["result"] == "ERROR" and result["error"]:
            print("ERROR {} [{}]: {}".format(result["region"], result["config"], result["error"]))

    return 0 if fail_count == 0 and error_count == 0 else 1


def main() -> int:
    args = parse_args()
    project_root = detect_project_root()

    rust_bin = resolve_path(project_root, args.rust_bin)
    java_bin = resolve_path(project_root, args.java_bin)
    output_dir = resolve_path(project_root, args.output_dir)
    preset_path = (project_root / CONFIG_PRESETS).resolve()
    batch_path = resolve_path(project_root, args.batch) if args.batch else None

    if args.verbose:
        print("Project root: {}".format(project_root))
        print("Preset file: {}".format(preset_path))
        if args.region:
            print("BAM: {}".format(resolve_path(project_root, args.bam)))
            print("REF: {}".format(resolve_path(project_root, args.ref)))
        if batch_path:
            print("Batch file: {}".format(batch_path))
        print("Java bin: {}".format(java_bin))
        print("Rust bin: {}".format(rust_bin))
        print("Output dir: {}".format(output_dir))

    build_java_if_needed(java_bin, project_root, args.verbose)
    ensure_rust_bin_exists(rust_bin)

    if args.all_configs:
        config_names = load_all_config_names(preset_path)
    else:
        load_config_preset(preset_path, args.config)
        config_names = [args.config]

    if args.batch or args.push_only:
        all_regions = load_regions_file(batch_path if batch_path else resolve_path(project_root, str(DEFAULT_BATCH_FILE)))
        if args.push_only:
            if max(PUSH_INDICES) >= len(all_regions):
                raise SystemExit(
                    "ERROR: Push indices require at least {} regions, found {}".format(
                        max(PUSH_INDICES) + 1,
                        len(all_regions),
                    )
                )
            selected_regions = [all_regions[index] for index in PUSH_INDICES]
        else:
            selected_regions = all_regions

        return print_summary_table(
            run_batch(
                regions=selected_regions,
                configs=config_names,
                args=args,
                project_root=project_root,
                java_bin=java_bin,
                rust_bin=rust_bin,
                output_dir=output_dir,
                preset_path=preset_path,
            )
        )

    bam_path = resolve_path(project_root, args.bam)
    ref_path = resolve_path(project_root, args.ref)
    if not bam_path.is_file():
        raise SystemExit("ERROR: BAM file not found: {}".format(bam_path))
    if not ref_path.is_file():
        raise SystemExit("ERROR: Reference FASTA not found: {}".format(ref_path))

    if len(config_names) > 1:
        return print_summary_table(
            run_batch(
                regions=[(args.region, args.bam, args.ref)],
                configs=config_names,
                args=args,
                project_root=project_root,
                java_bin=java_bin,
                rust_bin=rust_bin,
                output_dir=output_dir,
                preset_path=preset_path,
            )
        )

    result = run_comparison(
        region=args.region,
        bam_path=bam_path,
        ref_path=ref_path,
        config_name=config_names[0],
        extra_flags=load_config_preset(preset_path, config_names[0]),
        args=args,
        java_bin=java_bin,
        rust_bin=rust_bin,
        output_dir=output_dir,
    )

    if result["result"] == "PASS":
        print("PASS: {}".format(args.region))
        if args.verbose:
            print("Saved Java output to {}".format(result["java_path"]))
            print("Saved Rust output to {}".format(result["rust_path"]))
        return 0

    print("FAIL: {}".format(args.region))
    print(result["diff_text"])
    print("Saved Java output to {}".format(result["java_path"]))
    print("Saved Rust output to {}".format(result["rust_path"]))
    print("Saved diff output to {}".format(result["diff_path"]))
    return 1


if __name__ == "__main__":
    sys.exit(main())