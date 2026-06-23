#!/usr/bin/env python3
"""Check Renju eval suppression against the full forbidden detector.

The Rust eval cache has crate-private board construction, so this script wraps
the ignored Rust test that can build arbitrary fixture boards internally.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case-file", type=Path, required=True)
    parser.add_argument("--release", action="store_true", help="run the Rust test in release mode")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    case_file = args.case_file.resolve()
    if not case_file.exists():
        raise SystemExit(f"case file not found: {case_file}")

    command = ["cargo", "test"]
    if args.release:
        command.append("--release")
    command.extend([
        "renju_eval_suppression_matches_detector_on_env_case_file",
        "--",
        "--ignored",
        "--nocapture",
    ])

    env = os.environ.copy()
    env["RENJU_EVAL_SUPPRESSION_CASE_FILE"] = str(case_file)
    return subprocess.run(command, cwd=repo_root(), env=env, check=False).returncode


if __name__ == "__main__":
    sys.exit(main())
