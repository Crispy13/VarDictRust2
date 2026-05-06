"""Stdlib-only smoke tests for scripts/e2e_sweep_gate.py.

Runs via: `python3 -m unittest scripts.tests.test_e2e_sweep_gate`.
Discovered by pytest if pytest is installed.
"""
from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from scripts import e2e_sweep_gate


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
    def test_progress_log_captures_status_and_warning_lines(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(report_dir=Path(report_dir))

            with e2e_sweep_gate.progress_log(args):
                e2e_sweep_gate.emit_status("tests", event="pair-start", active="T1-01/hg002")
                e2e_sweep_gate.emit_warning_summary(
                    "provenance",
                    {"missing_chunks": 2},
                    samples=["T1-01/hg002/chr1"],
                )

            progress_lines = (Path(report_dir) / "progress.log").read_text(encoding="utf-8").splitlines()

        self.assertEqual(progress_lines[0], "STATUS phase=tests event=pair-start active=T1-01/hg002 elapsed=00:00:00")
        self.assertIn("WARNING phase=provenance elapsed=00:00:00 warnings=2", progress_lines[1])

    def test_format_status_line_includes_progress_fields(self) -> None:
        with mock.patch("scripts.e2e_sweep_gate.time.monotonic", return_value=3661.0):
            line = e2e_sweep_gate.format_status_line(
                "tests",
                started_at=1.0,
                completed=3,
                total=44,
                active="T1-01/hg002",
                event="pair-pass",
                detail="warnings=0",
            )

        self.assertIn("STATUS phase=tests", line)
        self.assertIn("event=pair-pass", line)
        self.assertIn("progress=3/44", line)
        self.assertIn("active=T1-01/hg002", line)
        self.assertIn("elapsed=01:01:00", line)
        self.assertIn("warnings=0", line)

    def test_run_streaming_subprocess_returns_stdout_and_stderr(self) -> None:
        stdout_buffer = io.StringIO()
        stderr_buffer = io.StringIO()
        command = [
            sys.executable,
            "-c",
            (
                "import sys; "
                "print('alpha'); "
                "print('beta', file=sys.stderr); "
                "print('gamma')"
            ),
        ]

        with contextlib.redirect_stdout(stdout_buffer), contextlib.redirect_stderr(stderr_buffer):
            result = e2e_sweep_gate.run_streaming_subprocess(command, cwd=PROJECT_ROOT, env=dict(os.environ))

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.splitlines(), ["alpha", "gamma"])
        self.assertEqual(result.stderr.splitlines(), ["beta"])
        self.assertEqual(stdout_buffer.getvalue().splitlines(), ["alpha", "gamma"])
        self.assertEqual(stderr_buffer.getvalue().splitlines(), ["beta"])

    def test_run_tests_and_report_stops_after_first_failed_pair(self) -> None:
        matrix = [("T1-01", "hg002", "1"), ("T1-02", "na12878_exome", "1")]
        first_result = subprocess.CompletedProcess(
            ["cargo"],
            1,
            "running 1 test\nMismatch in tile: 1:100-200\n",
            "trial: skipped after MAX_FAILURES cap (10)\n",
        )
        second_result = subprocess.CompletedProcess(["cargo"], 0, "running 1 test\n", "")
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                cargo_extra_arg=[],
                test_threads=1,
                report_dir=Path(report_dir),
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    side_effect=[first_result, second_result],
                ) as run_subprocess:
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))

        self.assertEqual(rc, 1)
        self.assertEqual(run_subprocess.call_count, 1)
        self.assertEqual(payload["failures"][0]["preset"], "T1-01")
        self.assertEqual(payload["failures"][0]["tag"], "hg002")

    def test_fresh_stage_root_initializes_manifest_before_snapshot(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            manifest_path = root / "manifest.json"
            snapshot_path = root / ".manifest.cache_entries.before.json"

            with mock.patch.multiple(
                "scripts.e2e_sweep_gate",
                CANONICAL_MANIFEST=manifest_path,
                MANIFEST_SNAPSHOT=snapshot_path,
            ), mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                created_manifest = e2e_sweep_gate.ensure_stage_manifest()
                created_snapshot = e2e_sweep_gate.snapshot_cache_entries()

            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            snapshot = json.loads(snapshot_path.read_text(encoding="utf-8"))

        self.assertTrue(created_manifest)
        self.assertTrue(created_snapshot)
        self.assertEqual(manifest["vardictjava_commit"], "deadbeef")
        self.assertEqual(manifest["cache_entries"], {})
        self.assertEqual(snapshot, {"cache_entries": {}})

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