"""Stdlib-only smoke tests for scripts/e2e_sweep_gate.py.

Runs via: `python3 -m unittest scripts.tests.test_e2e_sweep_gate`.
Discovered by pytest if pytest is installed.
"""
from __future__ import annotations

import contextlib
import hashlib
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
    def test_warning_summary_defaults_unknown_keys_to_not_ready(self) -> None:
        summary = e2e_sweep_gate.warning_summary_payload(
            {
                "stage": {
                    "counts": {"unexpected_warning": 1},
                    "samples": ["sample-warning"],
                }
            }
        )

        self.assertEqual(summary["total"], 1)
        self.assertEqual(summary["by_key"], {"unexpected_warning": 1})
        self.assertEqual(summary["by_severity"]["unknown"], 1)
        self.assertEqual(summary["readiness_impact"]["status"], "not-ready")
        self.assertEqual(summary["readiness_impact"]["unknown_warning_keys"], ["unexpected_warning"])

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

    def test_single_chrom_selection_uses_prefix_filter_without_exact(self) -> None:
        args = mock.Mock(cargo_extra_arg=[], test_threads=6)
        selector, exact = e2e_sweep_gate.sweep_test_selection("hg002", ["2"])
        cmd = e2e_sweep_gate.sweep_test_command(args, selector, exact=exact)

        self.assertEqual(selector, "hg002_sweep::parity_e2e_sweep_hg002_chr2_")
        self.assertFalse(exact)
        self.assertIn("hg002_sweep::parity_e2e_sweep_hg002_chr2_", cmd)
        self.assertNotIn("--exact", cmd)
        self.assertEqual(cmd[-1], "--test-threads=6")

    def test_multi_chrom_selection_keeps_legacy_exact_selector(self) -> None:
        args = mock.Mock(cargo_extra_arg=[], test_threads=6)
        selector, exact = e2e_sweep_gate.sweep_test_selection("hg002", ["1", "2"])
        cmd = e2e_sweep_gate.sweep_test_command(args, selector, exact=exact)

        self.assertEqual(selector, "hg002_sweep::parity_e2e_sweep_hg002")
        self.assertTrue(exact)
        self.assertIn("--exact", cmd)
        self.assertIn("hg002_sweep::parity_e2e_sweep_hg002", cmd)

    def test_run_tests_and_report_writes_diagnosis_ready_failure_artifact(self) -> None:
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
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
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
        self.assertEqual(payload["artifact_type"], "config-e2e-sweep-report")
        self.assertEqual(payload["schema_version"], 2)
        self.assertEqual(payload["result"], "failed")
        self.assertEqual(payload["planned_cell_count"], 2)
        self.assertEqual(payload["planned_pair_count"], 2)
        self.assertEqual(payload["tested_cell_count"], 1)
        self.assertEqual(payload["tested_pair_count"], 1)
        self.assertTrue(payload["halted_early"])
        self.assertEqual(payload["failures"][0]["preset"], "T1-01")
        self.assertEqual(payload["failures"][0]["tag"], "hg002")
        self.assertEqual(payload["failures"][0]["chrom"], "1")
        self.assertEqual(payload["failures"][0]["region_str"], "1:100-200")
        self.assertEqual(payload["original_matrix_scope"]["presets"], ["T1-01", "T1-02"])
        self.assertEqual(payload["original_matrix_scope"]["tags"], ["hg002", "na12878_exome"])
        self.assertEqual(payload["original_matrix_scope"]["chroms"], ["1"])
        self.assertEqual(payload["warning_summary"]["total"], 0)
        self.assertEqual(payload["warning_summary"]["readiness_impact"]["status"], "ready")
        self.assertEqual(payload["diagnosis_artifact"]["consumer_skill"], "config-e2e-diagnosis")
        self.assertEqual(payload["diagnosis_artifact"]["default_action"], "consume-existing-artifact")
        self.assertTrue(payload["diagnosis_artifact"]["readiness"]["ready"])
        self.assertEqual(payload["diagnosis_artifact"]["readiness"]["status"], "ready")
        self.assertEqual(payload["diagnosis_artifact"]["test_threads"], 1)

    def test_run_tests_and_report_accepts_config_tile_failure_format(self) -> None:
        matrix = [("T1-01", "hg002", "1")]
        first_result = subprocess.CompletedProcess(
            ["cargo"],
            1,
            "running 1 test\nconfig=T1-01 tile=1:2324084-2324612\n",
            "trial: skipped after MAX_FAILURES cap (10)\n",
        )
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
                cargo_extra_arg=[],
                test_threads=2,
                report_dir=Path(report_dir),
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    return_value=first_result,
                ):
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))

        self.assertEqual(rc, 1)
        self.assertEqual(payload["failures"][0]["region_str"], "1:2324084-2324612")
        self.assertEqual(payload["diagnosis_artifact"]["default_action"], "consume-existing-artifact")
        self.assertTrue(payload["diagnosis_artifact"]["readiness"]["ready"])

    def test_run_tests_and_report_records_completed_full_matrix_on_pass(self) -> None:
        matrix = [("T1-01", "hg002", "1"), ("T1-01", "hg002", "2")]
        passing_result = subprocess.CompletedProcess(["cargo"], 0, "running 2 tests\n", "")
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
                cargo_extra_arg=[],
                test_threads=4,
                report_dir=Path(report_dir),
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    return_value=passing_result,
                ):
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))

        self.assertEqual(rc, 0)
        self.assertEqual(payload["result"], "passed")
        self.assertEqual(payload["planned_cell_count"], 2)
        self.assertEqual(payload["planned_pair_count"], 1)
        self.assertEqual(payload["tested_cell_count"], 2)
        self.assertEqual(payload["tested_pair_count"], 1)
        self.assertFalse(payload["halted_early"])
        self.assertEqual(payload["warning_summary"]["readiness_impact"]["status"], "ready")
        self.assertEqual(payload["diagnosis_artifact"]["default_action"], "none")
        self.assertEqual(payload["diagnosis_artifact"]["readiness"]["status"], "not-needed")

    def test_run_tests_and_report_records_single_chrom_reproducer(self) -> None:
        matrix = [("T1-01", "hg002", "2")]
        first_result = subprocess.CompletedProcess(
            ["cargo"],
            1,
            "running 1 test\nconfig=T1-01 tile=2:300-400\n",
            "",
        )
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            scoped_bed_root = Path(report_dir) / "scoped_beds"
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
                cargo_extra_arg=[],
                test_threads=6,
                report_dir=Path(report_dir),
                runtime_sweep_bed_root=scoped_bed_root,
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    return_value=first_result,
                ) as run_subprocess:
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))

        command = run_subprocess.call_args.args[0]
        self.assertEqual(rc, 1)
        self.assertNotIn("--exact", command)
        self.assertIn("hg002_sweep::parity_e2e_sweep_hg002_chr2_", command)
        self.assertEqual(payload["failures"][0]["region_str"], "2:300-400")
        self.assertIn("hg002_sweep::parity_e2e_sweep_hg002_chr2_", payload["failures"][0]["reproducer_cmd"])
        self.assertNotIn("--exact", payload["failures"][0]["reproducer_cmd"])
        self.assertEqual(payload["diagnosis_artifact"]["sweep_bed_root"], str(scoped_bed_root.resolve()))

    def test_run_tests_and_report_marks_failure_artifact_not_ready_without_region_str(self) -> None:
        matrix = [("T1-01", "hg002", "1")]
        first_result = subprocess.CompletedProcess(
            ["cargo"],
            1,
            "running 0 tests\n",
            "selector drift\n",
        )
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
                cargo_extra_arg=[],
                test_threads=3,
                report_dir=Path(report_dir),
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    return_value=first_result,
                ):
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))

        self.assertEqual(rc, 1)
        self.assertIsNone(payload["failures"][0]["region_str"])
        self.assertFalse(payload["diagnosis_artifact"]["readiness"]["ready"])
        self.assertEqual(payload["diagnosis_artifact"]["readiness"]["status"], "rerun-required")
        self.assertEqual(
            payload["diagnosis_artifact"]["readiness"]["reason"],
            "Failure artifact is missing parseable region_str evidence for one or more recorded failures.",
        )
        self.assertEqual(payload["diagnosis_artifact"]["default_action"], "rerun-phase1-sweep")

    def test_run_tests_and_report_marks_warning_blocked_artifact_not_ready(self) -> None:
        matrix = [("T1-01", "hg002", "1")]
        first_result = subprocess.CompletedProcess(
            ["cargo"],
            1,
            "running 1 test\nconfig=T1-01 tile=1:2324084-2324612\n",
            "trial: skipped after MAX_FAILURES cap (10)\n",
        )
        warning_summary = e2e_sweep_gate.warning_summary_payload(
            {
                "provenance": {
                    "counts": {"missing_generator_flags": 1},
                    "samples": ["missing-generator-flags"],
                }
            }
        )
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
                cargo_extra_arg=[],
                test_threads=2,
                report_dir=Path(report_dir),
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    return_value=first_result,
                ):
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix, warning_summary)

            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))

        self.assertEqual(rc, 1)
        self.assertEqual(payload["warning_summary"]["readiness_impact"]["status"], "not-ready")
        self.assertFalse(payload["diagnosis_artifact"]["readiness"]["ready"])
        self.assertEqual(payload["diagnosis_artifact"]["readiness"]["status"], "rerun-required")
        self.assertIn("missing_generator_flags", payload["diagnosis_artifact"]["readiness"]["reason"])
        self.assertEqual(payload["diagnosis_artifact"]["default_action"], "rerun-phase1-sweep")

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

    def test_stage_single_chrom_with_extra_beds_merges_scoped_cache_entry(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir, tempfile.TemporaryDirectory(
            dir=PROJECT_ROOT / "tmp"
        ) as source_dir, tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as bed_dir:
            root = Path(root_dir)
            source_root = Path(source_dir)
            bed_root = Path(bed_dir)
            output_root = root / "output"
            manifest_path = root / "manifest.json"
            snapshot_path = root / ".manifest.cache_entries.before.json"
            preserve_path = root / ".manifest.cache_entries.gate_working.json"

            (bed_root / "hg002").mkdir(parents=True)
            bed_one = bed_root / "hg002" / "1.bed"
            bed_two = bed_root / "hg002" / "2.bed"
            bed_one.write_text("1\t10\t20\n", encoding="utf-8")
            bed_two.write_text("2\t30\t40\n", encoding="utf-8")

            source_tsv = source_root / "T1-01" / "2" / "hg002_2.tsv.zst"
            source_tsv.parent.mkdir(parents=True)
            source_tsv.write_text("fixture\n", encoding="utf-8")
            (source_tsv.parent / "hg002_2.chunks.json").write_text("{}\n", encoding="utf-8")

            args = mock.Mock(
                fixture_source=source_root,
                sweep_bed_root=bed_root,
                allow_extra_beds=True,
                force=True,
                report_dir=root / "report",
            )
            matrix = [("T1-01", "hg002", "2")]

            with mock.patch.dict(os.environ, {"VARDICT_E2E_SWEEP_FIXTURE_ROOT": str(root)}), mock.patch.multiple(
                "scripts.e2e_sweep_gate",
                CANONICAL_FIXTURE_ROOT=root,
                CANONICAL_OUTPUT_ROOT=output_root,
                CANONICAL_MANIFEST=manifest_path,
                MANIFEST_SNAPSHOT=snapshot_path,
                MERGE_PRESERVE_WORK=preserve_path,
            ), mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                e2e_sweep_gate.run_stage(args, matrix)

            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            entry = manifest["cache_entries"]["T1-01:hg002"]
            digest = hashlib.sha256()
            digest.update(bed_two.read_bytes())

            self.assertEqual(entry["config"], "T1-01")
            self.assertEqual(entry["tag"], "hg002")
            self.assertEqual(entry["bed_sha256"], digest.hexdigest())
            self.assertEqual(args.runtime_sweep_bed_root, (root / "report" / "scoped_beds").resolve())
            self.assertTrue((args.runtime_sweep_bed_root / "hg002" / "2.bed").is_file())
            self.assertFalse((args.runtime_sweep_bed_root / "hg002" / "1.bed").exists())
            self.assertTrue((output_root / "T1-01" / "2" / "hg002_2.tsv.zst").is_symlink())

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
        self.assertIn("hg002_sweep::parity_e2e_sweep_hg002_chr1_", result.stdout)
        self.assertNotIn("--exact hg002_sweep::parity_e2e_sweep_hg002", result.stdout)

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