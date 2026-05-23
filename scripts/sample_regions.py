#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import random
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


VALID_CHROMS = [str(chrom) for chrom in range(1, 23)] + ["X", "Y"]
DEFAULT_SAMTOOLS = "/home/eck/software/miniconda3/envs/vdr/bin/samtools"


@dataclass(frozen=True, order=True)
class Region:
    chrom: str
    start: int
    end: int

    def format(self) -> str:
        return f"{self.chrom}:{self.start}-{self.end}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Sample covered genomic regions from a BAM for parity testing."
    )
    parser.add_argument("bam", help="Path to the indexed BAM file.")
    parser.add_argument("ref", help="Path to the reference FASTA.")
    parser.add_argument("count", type=int, help="Number of regions to generate.")
    parser.add_argument(
        "--samtools",
        default=os.environ.get("SAMTOOLS", DEFAULT_SAMTOOLS),
        help="Path to the samtools binary.",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=42,
        help="Random seed used for reproducible sampling.",
    )
    return parser.parse_args()


def chrom_sort_key(chrom: str) -> tuple[int, str]:
    if chrom.isdigit():
        return (0, f"{int(chrom):02d}")
    return (1, chrom)


class RegionSampler:
    def __init__(self, bam_path: str, ref_path: str, samtools_path: str, seed: int) -> None:
        self.bam_path = bam_path
        self.ref_path = ref_path
        self.samtools_path = samtools_path
        self.random = random.Random(seed)
        self.chrom_lengths = self._load_chrom_lengths()
        self.chrom_read_counts = self._load_chrom_read_counts()

    def _run_samtools(self, *args: str) -> subprocess.CompletedProcess[str]:
        command = [self.samtools_path, *args]
        return subprocess.run(
            command,
            check=True,
            capture_output=True,
            text=True,
        )

    def _load_chrom_lengths(self) -> dict[str, int]:
        fai_path = Path(f"{self.ref_path}.fai")
        if not fai_path.exists():
            raise RuntimeError(f"Reference index not found: {fai_path}")

        chrom_lengths: dict[str, int] = {}
        with fai_path.open("r", encoding="utf-8") as handle:
            for line in handle:
                fields = line.rstrip("\n").split("\t")
                if len(fields) < 2:
                    continue
                chrom = fields[0]
                if chrom not in VALID_CHROMS:
                    continue
                chrom_lengths[chrom] = int(fields[1])

        if not chrom_lengths:
            raise RuntimeError(f"No numeric chromosomes found in {fai_path}")
        return chrom_lengths

    def _load_chrom_read_counts(self) -> dict[str, int]:
        idxstats_output = self._run_samtools("idxstats", self.bam_path).stdout.splitlines()
        chrom_read_counts: dict[str, int] = {}

        for line in idxstats_output:
            fields = line.split("\t")
            if len(fields) < 4:
                continue
            chrom = fields[0]
            if chrom not in self.chrom_lengths:
                continue
            mapped_reads = int(fields[2])
            if mapped_reads >= 1000:
                chrom_read_counts[chrom] = mapped_reads

        if not chrom_read_counts:
            raise RuntimeError(f"No chromosomes with at least 1000 mapped reads in {self.bam_path}")
        return chrom_read_counts

    def _allocate_counts_per_chrom(self, requested_count: int) -> dict[str, int]:
        total_reads = sum(self.chrom_read_counts.values())
        raw_allocations: list[tuple[str, int, float]] = []
        assigned = 0

        for chrom, read_count in self.chrom_read_counts.items():
            raw_count = requested_count * read_count / total_reads
            base_count = int(raw_count)
            raw_allocations.append((chrom, base_count, raw_count - base_count))
            assigned += base_count

        remaining = requested_count - assigned
        raw_allocations.sort(key=lambda item: (-item[2], chrom_sort_key(item[0])))

        allocations = {chrom: base_count for chrom, base_count, _ in raw_allocations}
        for chrom, _, _ in raw_allocations[:remaining]:
            allocations[chrom] += 1

        return {chrom: count for chrom, count in allocations.items() if count > 0}

    def _sample_read_starts(self, chrom: str, sample_size: int) -> list[int]:
        command = [self.samtools_path, "view", self.bam_path, chrom]
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        reservoir: list[int] = []
        seen_reads = 0

        assert process.stdout is not None
        for line in process.stdout:
            fields = line.split("\t", 5)
            if len(fields) < 4:
                continue
            try:
                start = int(fields[3])
            except ValueError:
                continue
            if start <= 0:
                continue

            seen_reads += 1
            if len(reservoir) < sample_size:
                reservoir.append(start)
                continue

            replace_index = self.random.randrange(seen_reads)
            if replace_index < sample_size:
                reservoir[replace_index] = start

        assert process.stderr is not None
        stderr = process.stderr.read().strip()
        return_code = process.wait()
        if return_code != 0:
            raise RuntimeError(f"samtools view failed for {chrom}: {stderr}")

        self.random.shuffle(reservoir)
        return reservoir

    def _pick_region_size(self) -> int:
        bucket = self.random.random()
        if bucket < 0.4:
            return self.random.randint(200, 500)
        if bucket < 0.8:
            return self.random.randint(1000, 5000)
        return self.random.randint(5000, 10000)

    def _build_region(self, chrom: str, read_start: int) -> Region:
        chrom_length = self.chrom_lengths[chrom]
        region_size = min(self._pick_region_size(), chrom_length)
        offset = self.random.randint(0, max(region_size - 1, 0))

        start = max(1, read_start - offset)
        end = start + region_size - 1
        if end > chrom_length:
            end = chrom_length
            start = max(1, end - region_size + 1)

        return Region(chrom=chrom, start=start, end=end)

    @staticmethod
    def _overlaps(candidate: Region, existing_regions: list[Region]) -> bool:
        for region in existing_regions:
            if region.chrom != candidate.chrom:
                continue
            if candidate.start <= region.end and candidate.end >= region.start:
                return True
        return False

    def _region_has_reads(self, region: Region) -> bool:
        region_spec = region.format()
        count_text = self._run_samtools("view", "-c", self.bam_path, region_spec).stdout.strip()
        return int(count_text or "0") > 0

    def sample_regions(self, requested_count: int) -> list[Region]:
        allocations = self._allocate_counts_per_chrom(requested_count)
        accepted_regions: list[Region] = []

        for chrom in sorted(allocations, key=chrom_sort_key):
            quota = allocations[chrom]
            sample_multiplier = 80

            while len([region for region in accepted_regions if region.chrom == chrom]) < quota:
                chrom_regions = [region for region in accepted_regions if region.chrom == chrom]
                needed = quota - len(chrom_regions)
                sample_size = max(needed * sample_multiplier, 100)
                candidate_starts = self._sample_read_starts(chrom, sample_size)
                if not candidate_starts:
                    break

                for read_start in candidate_starts:
                    if len(chrom_regions) >= quota:
                        break
                    candidate = self._build_region(chrom, read_start)
                    if self._overlaps(candidate, chrom_regions):
                        continue
                    if not self._region_has_reads(candidate):
                        continue
                    chrom_regions.append(candidate)
                    accepted_regions.append(candidate)

                if len(chrom_regions) >= quota:
                    break

                sample_multiplier *= 2
                if sample_multiplier > 640:
                    raise RuntimeError(
                        f"Unable to generate {quota} non-overlapping covered regions for chromosome {chrom}"
                    )

        if len(accepted_regions) != requested_count:
            raise RuntimeError(
                f"Generated {len(accepted_regions)} regions, expected {requested_count} for {self.bam_path}"
            )

        accepted_regions.sort(key=lambda region: (chrom_sort_key(region.chrom), region.start, region.end))
        return accepted_regions


def main() -> int:
    args = parse_args()

    if args.count <= 0:
        raise RuntimeError("COUNT must be a positive integer")

    sampler = RegionSampler(
        bam_path=args.bam,
        ref_path=args.ref,
        samtools_path=args.samtools,
        seed=args.seed,
    )

    for region in sampler.sample_regions(args.count):
        print(f"{region.format()}\t{args.bam}\t{args.ref}")

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        stderr = error.stderr.strip() if error.stderr else str(error)
        print(stderr, file=sys.stderr)
        raise SystemExit(error.returncode) from error
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1) from error