"""Tests for scripts/sort_sweep_fixtures.py.

Runs via: `python3 -m unittest scripts.tests.test_sort_sweep_fixtures`.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts import sort_sweep_fixtures


PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent


def zstd_compress_plain(plain: Path, dest: Path) -> None:
    subprocess.run(["zstd", "-q", "-f", str(plain), "-o", str(dest)], check=True)


def zstd_read_text(path: Path) -> str:
    result = subprocess.run(["zstd", "-dc", str(path)], check=True, capture_output=True, text=True)
    return result.stdout


class SortSweepFixturesTest(unittest.TestCase):
    def test_staged_migration_sorts_fixture_and_updates_sidecar(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            fixture_root = root / "fixtures"
            output_dir = fixture_root / "output" / "T1-01" / "1"
            output_dir.mkdir(parents=True)
            plain = root / "input.tsv"
            plain.write_text("1:20-30\tb\n1:10-20\tc\n1:10-20\ta\n", encoding="utf-8")
            source_tsv = output_dir / "hg002_1.tsv.zst"
            zstd_compress_plain(plain, source_tsv)
            sidecar = output_dir / "hg002_1.chunks.json"
            sidecar.write_text(
                json.dumps(
                    {
                        "monolithic_md5": "0" * 32,
                        "monolithic_bytes": 1,
                        "num_chunks": 2,
                        "chunk_size": 10000,
                        "chunks": [
                            {"idx": 0, "byte_range": [0, 10], "wall_s": 1.5, "num_tiles": 2},
                            {"idx": 1, "byte_range": [10, 11], "wall_s": 2.5, "num_tiles": 3},
                        ],
                        "generated_at": "2026-01-01T00:00:00Z",
                        "vardict_commit": "abc",
                        "generator_flags": ["--output-only"],
                        "preset": "T1-01",
                        "bed_sha256": "def",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            (fixture_root / "manifest.json").write_text('{"cache_entries": {}}\n', encoding="utf-8")
            staged_root = root / "staged"

            rc = sort_sweep_fixtures.main(
                [
                    "--fixture-root",
                    str(fixture_root),
                    "--output-fixture-root",
                    str(staged_root),
                    "--sort-buffer-size",
                    "1M",
                ]
            )

            self.assertEqual(rc, 0)
            staged_tsv = staged_root / "output" / "T1-01" / "1" / "hg002_1.tsv.zst"
            self.assertEqual(zstd_read_text(staged_tsv), "1:10-20\ta\n1:10-20\tc\n1:20-30\tb\n")
            payload = json.loads((staged_tsv.parent / "hg002_1.chunks.json").read_text(encoding="utf-8"))
            self.assertEqual(payload["output_order"]["mode"], "sorted")
            self.assertEqual(payload["output_order"]["key"], "Region<TAB>row")
            self.assertEqual(payload["output_order"]["sort_buffer_size"], "1M")
            self.assertEqual(payload["num_chunks"], 1)
            self.assertEqual(payload["source_num_chunks"], 2)
            self.assertEqual(payload["chunks"][0]["num_tiles"], 5)
            self.assertTrue((staged_root / "manifest.json").exists())
            self.assertEqual(zstd_read_text(source_tsv), "1:20-30\tb\n1:10-20\tc\n1:10-20\ta\n")

    def test_dry_run_does_not_create_staged_output(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            fixture_root = root / "fixtures"
            output_dir = fixture_root / "output" / "T1-01" / "1"
            output_dir.mkdir(parents=True)
            plain = root / "input.tsv"
            plain.write_text("1:20-30\tb\n", encoding="utf-8")
            zstd_compress_plain(plain, output_dir / "hg002_1.tsv.zst")

            rc = sort_sweep_fixtures.main(["--fixture-root", str(fixture_root), "--dry-run"])

            self.assertEqual(rc, 0)
            self.assertFalse((fixture_root / "output" / "T1-01" / "1" / "hg002_1.tsv.zst.sort-work").exists())

    def test_in_place_migration_updates_symlink_target_without_replacing_link(self) -> None:
        with tempfile.TemporaryDirectory(dir=PROJECT_ROOT / "tmp") as root_dir:
            root = Path(root_dir)
            target_root = root / "target"
            target_dir = target_root / "output" / "T1-01" / "1"
            target_dir.mkdir(parents=True)
            plain = root / "input.tsv"
            plain.write_text("1:20-30\tb\n1:10-20\ta\n", encoding="utf-8")
            target_tsv = target_dir / "hg002_1.tsv.zst"
            zstd_compress_plain(plain, target_tsv)
            target_sidecar = target_dir / "hg002_1.chunks.json"
            target_sidecar.write_text(
                json.dumps(
                    {
                        "monolithic_md5": "0" * 32,
                        "monolithic_bytes": 1,
                        "num_chunks": 1,
                        "chunk_size": 10000,
                        "chunks": [{"idx": 0, "byte_range": [0, 1]}],
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            fixture_root = root / "fixtures"
            link_dir = fixture_root / "output" / "T1-01" / "1"
            link_dir.mkdir(parents=True)
            link_tsv = link_dir / "hg002_1.tsv.zst"
            link_sidecar = link_dir / "hg002_1.chunks.json"
            link_tsv.symlink_to(target_tsv)
            link_sidecar.symlink_to(target_sidecar)

            rc = sort_sweep_fixtures.main(
                [
                    "--fixture-root",
                    str(fixture_root),
                    "--configs",
                    "T1-01",
                    "--tags",
                    "hg002",
                    "--chroms",
                    "1",
                    "--in-place",
                    "--sort-buffer-size",
                    "1M",
                ]
            )

            self.assertEqual(rc, 0)
            self.assertTrue(link_tsv.is_symlink())
            self.assertTrue(link_sidecar.is_symlink())
            self.assertEqual(zstd_read_text(target_tsv), "1:10-20\ta\n1:20-30\tb\n")
            payload = json.loads(target_sidecar.read_text(encoding="utf-8"))
            self.assertEqual(payload["output_order"]["mode"], "sorted")

    def test_cli_requires_explicit_write_mode(self) -> None:
        result = subprocess.run(
            [sys.executable, "-m", "scripts.sort_sweep_fixtures", "--fixture-root", "tmp/does-not-matter"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Choose --in-place, --output-fixture-root, or --dry-run", result.stderr)


if __name__ == "__main__":
    unittest.main()
