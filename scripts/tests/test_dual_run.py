"""Focused tests for scripts/dual_run.py helpers.

Runs via: `python3 -m unittest scripts.tests.test_dual_run`.
"""
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts import dual_run


PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent


def _write_jsonl(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"meta": True}) + "\n" + json.dumps(payload) + "\n", encoding="utf-8")


class DualRunSnapshotDiscoveryTest(unittest.TestCase):
    def test_diff_module_outputs_matches_lowercase_rust_dash_region_snapshot(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as output_dir:
            root = Path(output_dir)
            _write_jsonl(
                root / "module_snapshots" / "java" / "cigar_parser" / "cigar_parser_3_137225_137233.jsonl",
                {"value": "same"},
            )
            _write_jsonl(
                root / "module_snapshots" / "rust" / "cigar_parser" / "cigar_parser_3_137225-137233.jsonl",
                {"value": "same"},
            )

            results = dual_run.diff_module_outputs(root, ["cigar_parser"], "3:137225-137233")

        self.assertEqual(results, [{"module": "cigar_parser", "result": "PASS", "detail": ""}])

    def test_find_snapshot_files_accepts_legacy_uppercase_rust_snapshot(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as output_dir:
            root = Path(output_dir)
            path = root / "module_snapshots" / "rust" / "realigner" / "REALIGNER_3_137225-137233.jsonl"
            _write_jsonl(path, {"value": "same"})

            files = dual_run.find_snapshot_files(
                root / "module_snapshots" / "rust" / "realigner",
                "realigner",
                ("3", "137225", "137233"),
            )

        self.assertEqual(files, [path])


if __name__ == "__main__":
    unittest.main()