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

    def test_cache_fingerprint_match_passes_readiness_check(self) -> None:
        payload = {
            "monolithic_md5": hashlib.md5(b"fixture\n").hexdigest(),
            "monolithic_bytes": len(b"fixture\n"),
            "generator_flags": "--output-only --config T1-01 --tags hg002",
            "preset": "T1-01",
            "bed_sha256": "abc123",
        }
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            tsv = root / "hg002_1.tsv.zst"
            chunks = root / "hg002_1.chunks.json"
            tsv.write_bytes(b"compressed")
            chunks.write_text(json.dumps(payload), encoding="utf-8")

            with mock.patch(
                "scripts.e2e_sweep_gate.decompressed_tsv_md5_and_bytes",
                return_value=(payload["monolithic_md5"], payload["monolithic_bytes"]),
            ):
                warning = e2e_sweep_gate.cache_fingerprint_warning(tsv, chunks, payload)

        self.assertIsNone(warning)

    def test_cache_fingerprint_missing_md5_blocks_readiness(self) -> None:
        payload = {"monolithic_bytes": 8}
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            tsv = root / "hg002_1.tsv.zst"
            chunks = root / "hg002_1.chunks.json"
            tsv.write_bytes(b"compressed")

            warning = e2e_sweep_gate.cache_fingerprint_warning(tsv, chunks, payload)

        self.assertEqual(warning[0], "missing_monolithic_md5")

    def test_cache_fingerprint_non_object_payload_blocks_readiness(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            tsv = root / "hg002_1.tsv.zst"
            chunks = root / "hg002_1.chunks.json"
            tsv.write_bytes(b"compressed")

            warning = e2e_sweep_gate.cache_fingerprint_warning(tsv, chunks, ["not", "an", "object"])

        self.assertEqual(warning[0], "incompatible_chunks_json")

    def test_cache_fingerprint_mismatch_blocks_readiness(self) -> None:
        payload = {
            "monolithic_md5": hashlib.md5(b"old\n").hexdigest(),
            "monolithic_bytes": len(b"new\n"),
            "generator_flags": "--output-only --config T1-01 --tags hg002",
            "preset": "T1-01",
            "bed_sha256": "abc123",
        }
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            tsv = root / "hg002_1.tsv.zst"
            chunks = root / "hg002_1.chunks.json"
            tsv.write_bytes(b"compressed")

            with mock.patch(
                "scripts.e2e_sweep_gate.decompressed_tsv_md5_and_bytes",
                return_value=(hashlib.md5(b"new\n").hexdigest(), len(b"new\n")),
            ):
                warning = e2e_sweep_gate.cache_fingerprint_warning(tsv, chunks, payload)

        self.assertEqual(warning[0], "mismatch_monolithic_md5")

    def test_backfilled_sidecar_without_gate_provenance_blocks_readiness(self) -> None:
        payload = {
            "monolithic_md5": hashlib.md5(b"fixture\n").hexdigest(),
            "monolithic_bytes": len(b"fixture\n"),
            "backfilled": True,
        }
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            tsv = root / "hg002_1.tsv.zst"
            chunks = root / "hg002_1.chunks.json"
            tsv.write_bytes(b"compressed")

            warning = e2e_sweep_gate.cache_fingerprint_warning(tsv, chunks, payload)

        self.assertEqual(warning[0], "incompatible_backfilled_chunks")

    def test_cache_fingerprint_warning_routes_as_not_ready(self) -> None:
        summary = e2e_sweep_gate.warning_summary_payload(
            {
                "provenance": {
                    "counts": {"mismatch_monolithic_md5": 1},
                    "samples": ["sample-warning"],
                }
            }
        )

        self.assertEqual(summary["readiness_impact"]["status"], "not-ready")
        self.assertEqual(summary["readiness_impact"]["not_ready_warning_keys"], ["mismatch_monolithic_md5"])

    def test_run_provenance_check_invalid_chunks_json_warns_instead_of_crashing(self) -> None:
        matrix = [("T1-01", "hg002", "1")]
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            output_root = root / "output"
            chunks = output_root / "T1-01" / "1" / "hg002_1.chunks.json"
            chunks.parent.mkdir(parents=True, exist_ok=True)
            chunks.write_text("{not json\n", encoding="utf-8")
            args = mock.Mock(sweep_bed_root=root / "beds")

            with mock.patch.object(e2e_sweep_gate, "CANONICAL_OUTPUT_ROOT", output_root), mock.patch(
                "scripts.e2e_sweep_gate.live_vardictjava_commit",
                return_value="deadbeef",
            ), mock.patch(
                "scripts.e2e_sweep_gate.runtime_sweep_bed_root",
                return_value=root / "beds",
            ), mock.patch("scripts.e2e_sweep_gate.emit_status"), mock.patch(
                "scripts.e2e_sweep_gate.emit_warning_summary"
            ):
                warnings = e2e_sweep_gate.run_provenance_check(args, matrix)

        self.assertEqual(warnings["counts"], {"incompatible_chunks_json": 1})

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

    def test_run_streaming_subprocess_mirrors_output_to_report_dir_logs(self) -> None:
        stdout_buffer = io.StringIO()
        stderr_buffer = io.StringIO()
        command = [
            sys.executable,
            "-c",
            (
                "import sys; "
                "print('stdout-one'); "
                "print('stderr-one', file=sys.stderr); "
                "print('stdout-two')"
            ),
        ]

        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            with contextlib.redirect_stdout(stdout_buffer), contextlib.redirect_stderr(stderr_buffer):
                result = e2e_sweep_gate.run_streaming_subprocess(
                    command,
                    cwd=PROJECT_ROOT,
                    env=dict(os.environ),
                    report_dir=Path(report_dir),
                    log_role="cargo T1-01/hg002",
                    status_phase="tests",
                )

            log_paths = sorted((Path(report_dir) / "child-logs").glob("cargo-T1-01-hg002-*.log"))
            stdout_log = [path for path in log_paths if path.name.endswith(".stdout.log")][0]
            stderr_log = [path for path in log_paths if path.name.endswith(".stderr.log")][0]
            stdout_log_lines = stdout_log.read_text(encoding="utf-8").splitlines()
            stderr_log_lines = stderr_log.read_text(encoding="utf-8").splitlines()

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.splitlines(), ["stdout-one", "stdout-two"])
        self.assertEqual(result.stderr.splitlines(), ["stderr-one"])
        self.assertEqual(len(log_paths), 2)
        self.assertEqual(stdout_log_lines, ["stdout-one", "stdout-two"])
        self.assertEqual(stderr_log_lines, ["stderr-one"])
        self.assertIn("STATUS phase=tests event=child-output-log", stdout_buffer.getvalue())
        self.assertEqual(stderr_buffer.getvalue().splitlines(), ["stderr-one"])

    def test_idle_diagnostic_detail_includes_liveness_without_kill_action(self) -> None:
        with mock.patch("scripts.e2e_sweep_gate.time.monotonic", return_value=125.0):
            detail = e2e_sweep_gate.idle_diagnostic_detail(
                child_pid=12345,
                started_at=100.0,
                last_byte_monotonic=110.0,
                last_byte_wall=1_800_000_000.0,
                liveness="alive=1,state=S-sleeping",
            )

        self.assertIn("child_pid=12345", detail)
        self.assertIn("elapsed_seconds=25.0", detail)
        self.assertIn("last_byte_at=2027-01-15T08:00:00Z", detail)
        self.assertIn("idle_seconds=15.0", detail)
        self.assertIn("liveness=alive=1,state=S-sleeping", detail)
        self.assertIn("action=diagnostic-only", detail)

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

    def test_run_tests_and_report_sets_heartbeat_env_and_report_logs(self) -> None:
        matrix = [("T1-01", "hg002", "1")]
        passing_result = subprocess.CompletedProcess(["cargo"], 0, "running 1 test\n", "")
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            args = mock.Mock(
                sweep_bed_root=PROJECT_ROOT / "tmp" / "sweep_beds",
                fixture_source=PROJECT_ROOT / "tmp" / "sweep_fixtures" / "output",
                cargo_extra_arg=[],
                test_threads=2,
                report_dir=Path(report_dir),
                idle_diagnostic_seconds=30.0,
            )
            with mock.patch("scripts.e2e_sweep_gate.live_vardictjava_commit", return_value="deadbeef"):
                with mock.patch(
                    "scripts.e2e_sweep_gate.run_streaming_subprocess",
                    return_value=passing_result,
                ) as run_subprocess:
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

        self.assertEqual(rc, 0)
        call_kwargs = run_subprocess.call_args.kwargs
        heartbeat_value = call_kwargs["env"]["VARDICT_E2E_SWEEP_HEARTBEAT_LOG"]
        self.assertTrue(heartbeat_value.startswith(str(Path(report_dir) / "heartbeats")))
        self.assertEqual(call_kwargs["report_dir"], Path(report_dir))
        self.assertEqual(call_kwargs["log_role"], "cargo-T1-01-hg002-pair001")
        self.assertEqual(call_kwargs["idle_diagnostic_seconds"], 30.0)
        self.assertEqual(call_kwargs["status_phase"], "tests")

    def test_collect_runtime_records_parses_terminal_heartbeat(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as report_dir:
            heartbeat_path = Path(report_dir) / "heartbeat.log"
            heartbeat_path.write_text(
                "\n".join(
                    [
                        "HEARTBEAT ts=1 phase=start config=T1-01 tag=hg002 chrom=1 chunk=0 trial=hg002_sweep::parity_e2e_sweep_hg002_chr1_chunk000 tiles=1 region_str=1:10-20",
                        "HEARTBEAT ts=2 phase=end-ok config=T1-01 tag=hg002 chrom=1 chunk=0 trial=hg002_sweep::parity_e2e_sweep_hg002_chr1_chunk000 status=passed region_str=1:10-20 total_ms=1500 cache_ms=10 java_load_ms=20 rust_run_ms=1400 diff_ms=30",
                        "not a heartbeat",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )

            records, summary = e2e_sweep_gate.collect_runtime_records(
                heartbeat_path,
                preset="T1-01",
                tag="hg002",
                test_threads=4,
                cargo_test_name="hg002_sweep::parity_e2e_sweep_hg002_chr1_",
                command="cargo test ...",
                wrapper_pair_seconds=2.25,
                libtest_seconds=1.75,
            )

        self.assertEqual(summary["malformed_heartbeat_lines"], 1)
        self.assertEqual(summary["missing_heartbeat_logs"], 0)
        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["schema_version"], 1)
        self.assertEqual(record["preset"], "T1-01")
        self.assertEqual(record["tag"], "hg002")
        self.assertEqual(record["chrom"], "1")
        self.assertEqual(record["chunk"], 0)
        self.assertEqual(record["region_str"], "1:10-20")
        self.assertEqual(record["status"], "passed")
        self.assertEqual(record["telemetry_status"], "complete")
        self.assertEqual(record["total_seconds"], 1.5)
        self.assertEqual(record["phase_seconds"]["rust_run"], 1.4)
        self.assertEqual(record["test_threads"], 4)
        self.assertEqual(record["wrapper_pair_seconds"], 2.25)
        self.assertEqual(record["libtest_seconds"], 1.75)

    def test_run_tests_and_report_writes_runtime_jsonl_and_artifact_summary(self) -> None:
        matrix = [("T1-01", "hg002", "1")]

        def fake_run_streaming_subprocess(*_args, **kwargs):
            heartbeat_path = Path(kwargs["env"]["VARDICT_E2E_SWEEP_HEARTBEAT_LOG"])
            heartbeat_path.parent.mkdir(parents=True, exist_ok=True)
            heartbeat_path.write_text(
                "\n".join(
                    [
                        "HEARTBEAT ts=1 phase=start config=T1-01 tag=hg002 chrom=1 chunk=0 trial=hg002_sweep::parity_e2e_sweep_hg002_chr1_chunk000 tiles=1 region_str=1:10-20",
                        "HEARTBEAT ts=2 phase=end-ok config=T1-01 tag=hg002 chrom=1 chunk=0 trial=hg002_sweep::parity_e2e_sweep_hg002_chr1_chunk000 status=passed region_str=1:10-20 total_ms=1200 cache_ms=10 java_load_ms=20 rust_run_ms=1100 diff_ms=30",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            return subprocess.CompletedProcess(
                ["cargo"],
                0,
                "running 1 test\ntest result: ok. 1 passed; 0 failed; finished in 1.23s\n",
                "",
            )

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
                    side_effect=fake_run_streaming_subprocess,
                ):
                    rc = e2e_sweep_gate.run_tests_and_report(args, matrix)

            runtime_path = Path(report_dir) / "cell-runtimes.jsonl"
            runtime_records = [json.loads(line) for line in runtime_path.read_text(encoding="utf-8").splitlines()]
            payload = json.loads((Path(report_dir) / "parity-failure-report.json").read_text(encoding="utf-8"))
            last_pass = json.loads((Path(report_dir) / "last-pass.json").read_text(encoding="utf-8"))

        self.assertEqual(rc, 0)
        self.assertEqual(len(runtime_records), 1)
        self.assertEqual(runtime_records[0]["total_seconds"], 1.2)
        self.assertEqual(runtime_records[0]["test_threads"], 4)
        self.assertEqual(payload["runtime_summary"]["cell_runtimes_path"], str(runtime_path.resolve()))
        self.assertEqual(payload["runtime_summary"]["tested_cell_count_with_telemetry"], 1)
        self.assertEqual(payload["runtime_summary"]["missing_telemetry_count"], 0)
        self.assertEqual(payload["runtime_summary"]["duration_seconds"]["max"], 1.2)
        self.assertEqual(payload["diagnosis_artifact"]["readiness"]["status"], "not-needed")
        self.assertEqual(last_pass["runtime_summary"]["runtime_record_count"], 1)

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