#!/usr/bin/env python3
"""M7 dual-run harness (sudo-rs-inspired).

Runs VarDictJava and the Rust port against the same region and diffs the
TSV outputs line-by-line after sort. Intended for ad-hoc parity exploration
on regions not covered by `tests/parity_e2e.rs`.

Usage:
    scripts/dual_run.py --region 20:1515416-1518921 \\
        --bam testdata/NA12878.chrom20.ILLUMINA.bwa.CEU.exome.20121211.bam \\
        --ref testdata/hs37d5.fa

Exit codes:
    0  — outputs match (sorted line equality)
    1  — mismatch (first 20 diff lines printed)
    2  — environment or subprocess failure
"""

from __future__ import annotations

import argparse
import os
import pathlib
import subprocess
import sys
import tempfile


REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
VARDICT_BIN = REPO_ROOT / "VarDictJava/build/install/VarDict/bin/VarDict"


def build_vardict_if_needed() -> None:
    if VARDICT_BIN.is_file() and os.access(VARDICT_BIN, os.X_OK):
        return
    print("Building VarDictJava...", file=sys.stderr)
    subprocess.run(
        ["./gradlew", "installDist", "-q"],
        cwd=REPO_ROOT / "VarDictJava",
        check=True,
    )


def run_java(region: str, bam: str, ref: str, extra_flags: list[str], out_path: str) -> None:
    cmd = [str(VARDICT_BIN), "-G", ref, "-b", bam, "-N", "test_sample", "-th", "1"]
    cmd.extend(extra_flags)
    cmd.extend(["-R", region])
    with open(out_path, "w") as out:
        subprocess.run(cmd, check=True, stdout=out)


def run_rust(region: str, bam: str, ref: str, out_path: str) -> None:
    env = os.environ.copy()
    env["DUAL_REGION"] = region
    env["DUAL_BAM"] = bam
    env["DUAL_REF"] = ref
    env["DUAL_OUT"] = out_path
    cmd = [
        "cargo",
        "test",
        "--profile",
        "debug-release",
        "--test",
        "dual_run_emit",
        "dual_run_emit",
        "--",
        "--include-ignored",
        "--exact",
        "--nocapture",
    ]
    subprocess.run(cmd, cwd=REPO_ROOT, check=True, env=env)


def sorted_lines(path: str) -> list[str]:
    with open(path) as f:
        return sorted(line for line in (line.rstrip("\n") for line in f) if line)


def diff_sorted(java_path: str, rust_path: str) -> tuple[bool, list[str]]:
    j = sorted_lines(java_path)
    r = sorted_lines(rust_path)
    if j == r:
        return True, []
    diff: list[str] = []
    for idx, (jl, rl) in enumerate(zip(j, r)):
        if jl != rl:
            diff.append(f"  [line {idx}]\n    java: {jl}\n    rust: {rl}")
        if len(diff) >= 20:
            break
    if len(j) != len(r):
        diff.append(f"  (java lines: {len(j)}, rust lines: {len(r)})")
    return False, diff


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--region", required=True, help="chr:start-end")
    parser.add_argument("--bam", required=True)
    parser.add_argument("--ref", required=True)
    parser.add_argument(
        "--java-flag",
        action="append",
        default=[],
        help="Extra VarDictJava flag (repeat for multiple)",
    )
    parser.add_argument("--keep", action="store_true", help="Keep temp TSV files")
    args = parser.parse_args()

    try:
        build_vardict_if_needed()
    except subprocess.CalledProcessError as e:
        print(f"gradle build failed: {e}", file=sys.stderr)
        return 2

    tmp_dir = pathlib.Path(tempfile.mkdtemp(prefix="dual_run_", dir=REPO_ROOT / "tmp"))
    java_out = tmp_dir / "java.tsv"
    rust_out = tmp_dir / "rust.tsv"

    try:
        run_java(args.region, args.bam, args.ref, args.java_flag, str(java_out))
        run_rust(args.region, args.bam, args.ref, str(rust_out))
    except subprocess.CalledProcessError as e:
        print(f"subprocess failed: {e}", file=sys.stderr)
        return 2

    ok, diff = diff_sorted(str(java_out), str(rust_out))
    if ok:
        print(f"PARITY OK: {args.region} (outputs in {tmp_dir} — use --keep to retain)")
        if not args.keep:
            import shutil

            shutil.rmtree(tmp_dir)
        return 0

    print(f"PARITY MISMATCH: {args.region}")
    for line in diff:
        print(line)
    print(f"\nJava output: {java_out}")
    print(f"Rust output: {rust_out}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
