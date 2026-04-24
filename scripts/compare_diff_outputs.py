#!/usr/bin/env python3
"""Compare Rust and Python reference differential probe outputs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


DEFAULT_FIELDS = [
    "name",
    "board.side_to_move",
    "board.winner",
    "board.move_count",
    "board.zobrist_key",
    "root.move",
    "root.score",
    "root.depth",
    "root.trace.used_vcf",
    "root.trace.vcf_found",
    "root.trace.used_vct",
    "root.trace.vct_triggered",
    "root.trace.vct_found",
    "root.trace.vct_move",
    "root.trace.vct_accepted",
    "root.trace.vct_reject_reason",
    "root.trace.tactical_path",
]


def get_path(payload: dict, path: str):
    cur = payload
    for part in path.split("."):
        cur = cur[part]
    return cur


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rust", required=True, type=Path)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--field", action="append", dest="fields")
    args = parser.parse_args()

    rust = json.loads(args.rust.read_text(encoding="utf-8"))
    reference = json.loads(args.reference.read_text(encoding="utf-8"))
    fields = args.fields or DEFAULT_FIELDS

    mismatches = []
    for field in fields:
        rv = get_path(rust, field)
        fv = get_path(reference, field)
        if rv != fv:
            mismatches.append((field, rv, fv))

    if mismatches:
        print("DIFF")
        for field, rv, fv in mismatches:
            print(f"{field}: rust={rv!r} reference={fv!r}")
        raise SystemExit(1)

    print(f"OK {len(fields)} fields match")


if __name__ == "__main__":
    main()
