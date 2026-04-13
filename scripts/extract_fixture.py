#!/usr/bin/env python3

import argparse
import subprocess
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract a single region payload from a v2 JSONL zstd archive."
    )
    parser.add_argument("--archive", required=True, help="Path to .jsonl.zst archive")
    parser.add_argument("--region", required=True, help="Target region, e.g. 1:9996-10582")
    return parser.parse_args()


def extract_region(archive: str, target_region: str) -> int:
    process = subprocess.Popen(
        ["zstd", "-dc", archive],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    assert process.stdout is not None
    try:
        for line in process.stdout:
            parts = line.rstrip("\n").split("\t", 3)
            if len(parts) != 4:
                continue
            chrom, start, end, payload = parts
            region = f"{chrom}:{start}-{end}"
            if region == target_region:
                try:
                    sys.stdout.write(payload)
                    sys.stdout.write("\n")
                    sys.stdout.flush()
                except BrokenPipeError:
                    pass
                finally:
                    process.stdout.close()
                    process.terminate()
                    process.wait()
                return 0
    finally:
        if process.stdout is not None and not process.stdout.closed:
            process.stdout.close()

    stderr = ""
    if process.stderr is not None:
        stderr = process.stderr.read().strip()
        process.stderr.close()
    return_code = process.wait()
    if return_code != 0:
        if stderr:
            print(stderr, file=sys.stderr)
        return return_code

    print(f"Region {target_region} not found in archive {archive}", file=sys.stderr)
    return 1


def main() -> int:
    args = parse_args()
    return extract_region(args.archive, args.region)


if __name__ == "__main__":
    raise SystemExit(main())