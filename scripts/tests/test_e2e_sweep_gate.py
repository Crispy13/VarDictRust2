"""Stdlib-only smoke tests for scripts/e2e_sweep_gate.py.

Runs via: `python3 -m unittest scripts.tests.test_e2e_sweep_gate`.
Discovered by pytest if pytest is installed.
"""
from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent
SOURCE_ROOT = PROJECT_ROOT / "tmp" / "sweep_fixtures_hg002_allchrom" / "output"


def _run(args: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    process_env = dict(os.environ)
    if env is not None:
        process_env.update(env)
    return subprocess.run(
        [sys.executable, "-m", "scripts.e2e_sweep_gate", *args],
        cwd=PROJECT_ROOT,
        capture_output=True,
        env=process_env,
        text=True,
    )


class GateSmokeTest(unittest.TestCase):
    def test_help_exits_zero(self) -> None:
        result = _run(["--help"])
        self.assertEqual(result.returncode, 0, result.stderr)
        for flag in ("--dry-run", "--unstage", "--all-presets", "--fixture-source"):
            self.assertIn(flag, result.stdout)

    def test_dry_run_single_cell(self) -> None:
        if not SOURCE_ROOT.is_dir():
            self.skipTest(f"missing fixture source: {SOURCE_ROOT}")
        result = _run(
            [
                "--dry-run", "--preset", "T1-01", "--tag", "hg002", "--chrom", "1",
                "--fixture-source", str(SOURCE_ROOT),
            ],
            env={"VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM": "1"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Resolved matrix: 1 cells", result.stdout)
        self.assertIn("T1-01 / hg002 / chr1", result.stdout)
        self.assertIn("--exact hg002_sweep::parity_e2e_sweep_hg002", result.stdout)

    def test_dry_run_all_presets_count(self) -> None:
        if not SOURCE_ROOT.is_dir():
            self.skipTest(f"missing fixture source: {SOURCE_ROOT}")
        result = _run(
            [
                "--dry-run", "--all-presets", "--tag", "hg002", "--chrom", "1",
                "--fixture-source", str(SOURCE_ROOT),
            ],
            env={"VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM": "1"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        for line in result.stdout.splitlines():
            if line.startswith("Resolved matrix:"):
                count = int(line.split()[2])
                self.assertGreaterEqual(count, 44)
                break
        else:
            self.fail("'Resolved matrix:' line not found in stdout")

    def test_chr1_guard_hard_fails_on_multichrom_root(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir, tempfile.TemporaryDirectory(
            dir=PROJECT_ROOT / "tmp"
        ) as source_dir:
            bed_root = Path(root_dir)
            (bed_root / "hg002").mkdir()
            (bed_root / "hg002" / "1.bed").touch()
            (bed_root / "hg002" / "2.bed").touch()

            result = _run([
                "--dry-run",
                "--preset",
                "T1-01",
                "--tag",
                "hg002",
                "--chrom",
                "1",
                "--sweep-bed-root",
                str(bed_root),
                "--fixture-source",
                source_dir,
            ])

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ERROR:", result.stderr)
        self.assertIn("VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1", result.stderr)
        self.assertRegex(result.stderr, r"(^|[^0-9])2($|[^0-9])")

    def test_chr1_guard_bypass_env_warns(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir, tempfile.TemporaryDirectory(
            dir=PROJECT_ROOT / "tmp"
        ) as source_dir:
            bed_root = Path(root_dir)
            (bed_root / "hg002").mkdir()
            (bed_root / "hg002" / "1.bed").touch()
            (bed_root / "hg002" / "2.bed").touch()

            result = _run(
                [
                    "--dry-run",
                    "--preset",
                    "T1-01",
                    "--tag",
                    "hg002",
                    "--chrom",
                    "1",
                    "--sweep-bed-root",
                    str(bed_root),
                    "--fixture-source",
                    source_dir,
                ],
                env={"VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM": "1"},
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("WARNING:", result.stderr)
        self.assertIn("VARDICT_E2E_SWEEP_ALLOW_MULTI_CHROM=1", result.stderr)
        self.assertIn("Resolved matrix: 1 cells", result.stdout)


if __name__ == "__main__":
    unittest.main()