#!/usr/bin/env python3
"""Run Rust and Python reference probes for diff cases and compare the output.

Usage:
  scripts/run_diff.py                       # run fast cases/diff/*.json cases
  scripts/run_diff.py --profile all         # run every cases/diff/*.json case
  scripts/run_diff.py cases/diff/foo.json   # run one or more explicit cases
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import subprocess
import sys
import time
from pathlib import Path

from compare_diff_outputs import DEFAULT_FIELDS, get_path
from diff_reference import run_case as run_reference_case


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def run_rust_probe(case_path: Path) -> dict:
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--bin", "diff_probe", "--", "--case", str(case_path)],
        cwd=repo_root(),
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def run_reference_probe(case_path: Path) -> dict:
    case = json.loads(case_path.read_text(encoding="utf-8"))
    return run_reference_case(case)


def case_profile(case_path: Path) -> str:
    case = json.loads(case_path.read_text(encoding="utf-8"))
    return str(case.get("profile", "fast")).lower()


def filter_by_profile(case_paths: list[Path], profile: str) -> list[Path]:
    if profile == "all":
        return case_paths
    return [case_path for case_path in case_paths if case_profile(case_path) == profile]


def compare(rust: dict, reference: dict, fields: list[str]) -> list[tuple[str, object, object]]:
    mismatches = []
    for field in fields:
        rv = get_path(rust, field)
        fv = get_path(reference, field)
        if rv != fv:
            mismatches.append((field, rv, fv))
    return mismatches


def run_one_case(case_path: Path, fields: list[str]) -> tuple[str, bool, list[str], float]:
    start = time.perf_counter()
    name = case_path.name
    lines: list[str] = []
    try:
        rust = run_rust_probe(case_path)
        reference = run_reference_probe(case_path)
    except subprocess.CalledProcessError as exc:
        lines.append(f"{name}: FAIL rust probe")
        if exc.stderr:
            lines.append(exc.stderr.rstrip())
        return name, False, lines, time.perf_counter() - start
    except Exception as exc:
        lines.append(f"{name}: FAIL {exc!r}")
        return name, False, lines, time.perf_counter() - start

    mismatches = compare(rust, reference, fields)
    if mismatches:
        lines.append(f"{name}: DIFF")
        for field, rv, fv in mismatches:
            lines.append(f"  {field}: rust={rv!r} reference={fv!r}")
        return name, False, lines, time.perf_counter() - start

    lines.append(f"{name}: OK {len(fields)} fields match")
    return name, True, lines, time.perf_counter() - start


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("cases", nargs="*", type=Path)
    parser.add_argument("--field", action="append", dest="fields")
    parser.add_argument(
        "--profile",
        choices=("fast", "slow", "all"),
        default="fast",
        help="case profile to run when no explicit cases are provided (default: fast)",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=1,
        help="number of diff cases to run concurrently (default: 1)",
    )
    args = parser.parse_args()

    if args.cases:
        case_paths = args.cases
    else:
        case_paths = sorted((repo_root() / "cases" / "diff").glob("*.json"))
        case_paths = filter_by_profile(case_paths, args.profile)

    fields = args.fields or DEFAULT_FIELDS
    jobs = max(1, args.jobs)
    failures: list[str] = []

    started = time.perf_counter()
    if jobs == 1 or len(case_paths) <= 1:
        results = [run_one_case(case_path, fields) for case_path in case_paths]
    else:
        with concurrent.futures.ProcessPoolExecutor(max_workers=jobs) as pool:
            futures = {
                pool.submit(run_one_case, case_path, fields): index
                for index, case_path in enumerate(case_paths)
            }
            indexed_results = []
            for future in concurrent.futures.as_completed(futures):
                indexed_results.append((futures[future], future.result()))
            results = [result for _, result in sorted(indexed_results, key=lambda item: item[0])]

    for name, ok, lines, elapsed in results:
        for line in lines:
            print(line)
        print(f"  elapsed={elapsed:.3f}s")
        if not ok:
            failures.append(name)

    if failures:
        print(f"\n{len(failures)} case(s) failed: {', '.join(failures)}")
        return 1
    print(f"\nall {len(case_paths)} case(s) passed in {time.perf_counter() - started:.3f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
