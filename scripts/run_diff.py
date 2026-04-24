#!/usr/bin/env python3
"""Run Rust and Python reference probes for diff cases and compare the output.

Usage:
  scripts/run_diff.py                       # run every cases/diff/*.json
  scripts/run_diff.py cases/diff/foo.json   # run one or more specific cases
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
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


def compare(rust: dict, reference: dict, fields: list[str]) -> list[tuple[str, object, object]]:
    mismatches = []
    for field in fields:
        rv = get_path(rust, field)
        fv = get_path(reference, field)
        if rv != fv:
            mismatches.append((field, rv, fv))
    return mismatches


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("cases", nargs="*", type=Path)
    parser.add_argument("--field", action="append", dest="fields")
    args = parser.parse_args()

    if args.cases:
        case_paths = args.cases
    else:
        case_paths = sorted((repo_root() / "cases" / "diff").glob("*.json"))

    fields = args.fields or DEFAULT_FIELDS
    failures: list[str] = []
    for case_path in case_paths:
        name = case_path.name
        try:
            rust = run_rust_probe(case_path)
            reference = run_reference_probe(case_path)
        except subprocess.CalledProcessError as exc:
            print(f"{name}: FAIL rust probe")
            print(exc.stderr)
            failures.append(name)
            continue
        except Exception as exc:
            print(f"{name}: FAIL {exc!r}")
            failures.append(name)
            continue
        mismatches = compare(rust, reference, fields)
        if mismatches:
            print(f"{name}: DIFF")
            for field, rv, fv in mismatches:
                print(f"  {field}: rust={rv!r} reference={fv!r}")
            failures.append(name)
        else:
            print(f"{name}: OK {len(fields)} fields match")

    if failures:
        print(f"\n{len(failures)} case(s) failed: {', '.join(failures)}")
        return 1
    print(f"\nall {len(case_paths)} case(s) passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
