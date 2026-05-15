"""Stdlib-only smoke tests for scripts/sweep_fixtures_parallel.py.

Runs via: `python3 -m unittest scripts.tests.test_sweep_fixtures_parallel`.
Discovered by pytest if pytest is installed.
"""
from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path

from scripts import sweep_fixtures_parallel


PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent


class SweepFixturesParallelSmokeTest(unittest.TestCase):
    def test_shard_parallelism_summary_warns_when_workers_exceed_shards(self) -> None:
        summary, warning = sweep_fixtures_parallel.shard_parallelism_summary(6, 1)

        self.assertEqual(
            summary,
            "Parallelism:   shard process pool; discovered shards=1; effective workers=1/6",
        )
        self.assertIsNotNone(warning)
        self.assertIn("requested 6 workers", warning)
        self.assertIn("produced 1 shard", warning)

    def test_shard_parallelism_summary_omits_warning_when_capacity_matches(self) -> None:
        summary, warning = sweep_fixtures_parallel.shard_parallelism_summary(2, 3)

        self.assertEqual(
            summary,
            "Parallelism:   shard process pool; discovered shards=3; effective workers=2/2",
        )
        self.assertIsNone(warning)

    def test_help_describes_shard_and_preset_worker_models(self) -> None:
        result = subprocess.run(
            [sys.executable, "-m", "scripts.sweep_fixtures_parallel", "--help"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("one process per discovered shard", result.stdout)
        self.assertIn("one thread per selected preset", result.stdout)


if __name__ == "__main__":
    unittest.main()