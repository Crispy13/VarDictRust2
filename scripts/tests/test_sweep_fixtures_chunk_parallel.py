"""Stdlib-only tests for scripts/sweep_fixtures_chunk_parallel.py.

Runs via: `python3 -m unittest scripts.tests.test_sweep_fixtures_chunk_parallel`.
Discovered by pytest if pytest is installed.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

from scripts import sweep_fixtures_chunk_parallel
from scripts import sweep_fixtures_parallel as base


PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent


def fake_compress_path(path: Path) -> None:
    path.rename(path.with_name(f"{path.name}.zst"))


class SweepFixturesChunkParallelTest(unittest.TestCase):
    def test_help_describes_chunk_workers(self) -> None:
        result = subprocess.run(
            [sys.executable, "-m", "scripts.sweep_fixtures_chunk_parallel", "--help"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--chunk-workers", result.stdout)
        self.assertIn("chunked output-only shard", result.stdout)
        self.assertIn("execution. Chunk parallelism only activates", result.stdout)

    def test_chunk_parallelism_guard_requires_output_only_chunking_and_workers(self) -> None:
        self.assertFalse(
            sweep_fixtures_chunk_parallel.chunk_parallelism_enabled(
                effective_output_only=False,
                need_chunking=True,
                chunk_workers=2,
            )
        )
        self.assertFalse(
            sweep_fixtures_chunk_parallel.chunk_parallelism_enabled(
                effective_output_only=True,
                need_chunking=False,
                chunk_workers=2,
            )
        )
        self.assertFalse(
            sweep_fixtures_chunk_parallel.chunk_parallelism_enabled(
                effective_output_only=True,
                need_chunking=True,
                chunk_workers=1,
            )
        )
        self.assertTrue(
            sweep_fixtures_chunk_parallel.chunk_parallelism_enabled(
                effective_output_only=True,
                need_chunking=True,
                chunk_workers=2,
            )
        )

    def test_parallel_chunk_merge_preserves_chunk_index_order(self) -> None:
        with self.chunk_parallel_fixture() as fixture:
            result = fixture.run(chunk_workers=3)
            output_path, chunks_path = base.shard_output_paths(
                base.output_dir(fixture.output_root, None, fixture.shard.chrom),
                fixture.shard.tag,
                fixture.shard.chrom,
            )

            self.assertEqual(result.status, "success", result.error)
            self.assertTrue(output_path.exists())
            self.assertTrue(chunks_path.exists())
            self.assertEqual(output_path.read_bytes(), b"chunk-0\nchunk-1\nchunk-2\n")

            payload = json.loads(chunks_path.read_text(encoding="utf-8"))
            self.assertEqual([chunk["idx"] for chunk in payload["chunks"]], [0, 1, 2])
            self.assertEqual(payload["num_chunks"], 3)
            self.assertFalse(base.output_staging_dir(fixture.output_root, None, fixture.shard).exists())

    def test_parallel_chunk_failure_preserves_staging_and_skips_final_outputs(self) -> None:
        with self.chunk_parallel_fixture() as fixture:
            with mock.patch.dict(os.environ, {"FAIL_CHUNK": "1"}, clear=False):
                result = fixture.run(chunk_workers=3)

            output_path, chunks_path = base.shard_output_paths(
                base.output_dir(fixture.output_root, None, fixture.shard.chrom),
                fixture.shard.tag,
                fixture.shard.chrom,
            )
            output_staging = base.output_staging_dir(fixture.output_root, None, fixture.shard)
            log_path = base.log_file_path(fixture.output_root, None, fixture.shard)

            self.assertEqual(result.status, "failed")
            self.assertIn("chunk 1", result.error)
            self.assertFalse(output_path.exists())
            self.assertFalse(chunks_path.exists())
            self.assertTrue(output_staging.exists())
            self.assertTrue(any(output_staging.iterdir()))
            self.assertTrue(log_path.exists())
            self.assertIn("preserved_artifacts=", log_path.read_text(encoding="utf-8"))

    def chunk_parallel_fixture(self) -> "ChunkParallelFixture":
        return ChunkParallelFixture(self)


class ChunkParallelFixture:
    def __init__(self, test_case: unittest.TestCase) -> None:
        self.test_case = test_case
        self._tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self._tempdir.name)
        self.output_root = self.root / "out"
        self.output_root.mkdir()
        self.reference = self.root / "ref.fa"
        self.reference.write_text(">chr1\nACGT\n", encoding="utf-8")
        self.bed_path = self.root / "chr1.bed"
        self.bed_path.write_text(
            "".join(
                f"chr1\t{start}\t{start + 10}\n"
                for start in (0, 20, 40, 60, 80)
            ),
            encoding="utf-8",
        )
        self.vardict_rel = Path("fake_vardict.py")
        self.vardict_path = self.root / self.vardict_rel
        self.vardict_path.write_text(
            textwrap.dedent(
                """
                #!/usr/bin/env python3
                import os
                import pathlib
                import re
                import sys
                import time

                chunk_path = pathlib.Path(sys.argv[-1])
                match = re.search(r"(\\d+)", chunk_path.stem)
                chunk_idx = int(match.group(1)) if match else 0
                delays = {0: 0.30, 1: 0.05, 2: 0.15}
                time.sleep(delays.get(chunk_idx, 0.0))
                if os.environ.get("FAIL_CHUNK") == str(chunk_idx):
                    sys.stderr.write(f"fail-{chunk_idx}\\n")
                    sys.exit(7)
                sys.stdout.write(f"chunk-{chunk_idx}\\n")
                sys.stderr.write(f"log-{chunk_idx}\\n")
                """
            ).lstrip(),
            encoding="utf-8",
        )
        self.vardict_path.chmod(self.vardict_path.stat().st_mode | stat.S_IEXEC)
        self.shard = base.Shard(tag="hg002", chrom="chr1", bed_path=self.bed_path)

    def __enter__(self) -> "ChunkParallelFixture":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self._tempdir.cleanup()

    def run(self, *, chunk_workers: int) -> base.ShardResult:
        with mock.patch.object(base, "VARDICT_BIN_REL", self.vardict_rel), mock.patch.object(
            base,
            "compress_path",
            side_effect=fake_compress_path,
        ), mock.patch.object(base, "get_vardict_commit", return_value="deadbeef"):
            return sweep_fixtures_chunk_parallel.execute_shard_run(
                shard=self.shard,
                force=False,
                output_only=True,
                config_name=None,
                config_flags=(),
                root=self.root,
                output_root=self.output_root,
                chunk_size=2,
                bam_arg="fake.bam",
                reference=self.reference,
                chunk_workers=chunk_workers,
            )


if __name__ == "__main__":
    unittest.main()