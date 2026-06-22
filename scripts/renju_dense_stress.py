#!/usr/bin/env python3
"""Reproducible dense-position stress generator for Renju forbidden detection.

Each position is built from a fixed forbidden-shape *skeleton* placed at the
board centre, plus seeded random nearby interference stones. The output is then
labelled with the local Rust detector (`renju_rule_probe`) so it can be fed
straight into `scripts/renju_oracle_compare.py`, where Rapfi and `renju_forbid`
act as reverse oracles against the detector's labels.

The generator is fully deterministic for a given `--seed`: skeletons are
iterated in sorted order and each gets a fixed per-skeleton seed offset, so the
same command always produces byte-identical output. This replaces the earlier
throwaway heredoc that used `hash(name)` (which varies across processes).

Typical use:

    python3 scripts/renju_dense_stress.py --skeleton all --count 400 --seed 20 \
        --output /tmp/renju_dense_seed20.jsonl
    python3 scripts/renju_oracle_compare.py --case-file /tmp/renju_dense_seed20.jsonl --quiet

Any non-zero mismatch in the second step is either a detector bug or a
classification-convention difference (see docs/renju-forbidden-design.md).
"""

from __future__ import annotations

import argparse
import json
import random
import subprocess
import sys
import tempfile
from pathlib import Path

BOARD_SIZE = 15
CENTER = BOARD_SIZE // 2

# Skeletons are black-stone offsets (dx, dy) relative to the candidate, which is
# always the board centre. Each is a known forbidden / near-forbidden seed; the
# generator adds random interference around it to probe edge cases.
SKELETONS: dict[str, list[tuple[int, int]]] = {
    # vertical + horizontal true open-three cross (base double-three)
    "cross_three": [(-1, 0), (1, 0), (0, -1), (0, 1)],
    # crossed double-four seed
    "ff_cross": [(-2, 0), (-1, 0), (1, 0), (0, -2), (0, -1), (0, 1)],
    # same-line double-four seed (BBB_X_BBB)
    "ff_inline": [(-4, 0), (-3, 0), (-2, 0), (2, 0), (3, 0), (4, 0)],
    # overline seed: candidate fills to six in a row
    "overline": [(-3, 0), (-2, 0), (-1, 0), (1, 0), (2, 0)],
    # overline seed: candidate fills to seven in a row
    "ol_seven": [(-3, 0), (-2, 0), (-1, 0), (1, 0), (2, 0), (3, 0)],
}

# Fixed per-skeleton seed offset so different skeletons get independent but
# reproducible random streams. Derived from sorted order, never from hash().
SKELETON_SEED_OFFSET = {name: index * 100003 for index, name in enumerate(sorted(SKELETONS))}


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def in_bounds(x: int, y: int) -> bool:
    return 0 <= x < BOARD_SIZE and 0 <= y < BOARD_SIZE


def generate_case(skeleton: str, index: int, rng, *, radius: int,
                  min_extra: int, max_extra: int, black_prob: float) -> dict:
    occupied: set[tuple[int, int]] = {(CENTER, CENTER)}  # reserve the candidate
    moves: list[dict[str, int]] = []

    for dx, dy in SKELETONS[skeleton]:
        x, y = CENTER + dx, CENTER + dy
        moves.append({"x": x, "y": y, "side": 1})
        occupied.add((x, y))

    extra = rng.randint(min_extra, max_extra)
    for _ in range(extra):
        for _attempt in range(20):
            x = CENTER + rng.randint(-radius, radius)
            y = CENTER + rng.randint(-radius, radius)
            if in_bounds(x, y) and (x, y) not in occupied:
                side = 1 if rng.random() < black_prob else -1
                moves.append({"x": x, "y": y, "side": side})
                occupied.add((x, y))
                break

    return {
        "name": f"{skeleton}_{index}",
        "board_size": BOARD_SIZE,
        "moves": moves,
        "candidate": {"x": CENTER, "y": CENTER},
    }


def label_with_detector(cases: list[dict], *, timeout: float) -> None:
    """Fill each case's `expected` with the local Rust detector classification."""
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as handle:
        tmp_path = Path(handle.name)
        for case in cases:
            handle.write(json.dumps(case) + "\n")
    try:
        proc = subprocess.run(
            ["cargo", "run", "--quiet", "--bin", "renju_rule_probe", "--",
             "--case-file", str(tmp_path)],
            cwd=repo_root(), text=True, capture_output=True,
            timeout=timeout, check=False,
        )
    finally:
        tmp_path.unlink(missing_ok=True)

    if proc.returncode != 0:
        raise SystemExit(
            f"renju_rule_probe failed (exit {proc.returncode}):\n{proc.stderr}"
        )

    labels: dict[str, str] = {}
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        name, kind = line.split(maxsplit=1)
        labels[name] = kind

    for case in cases:
        kind = labels.get(case["name"])
        if kind is None:
            raise SystemExit(f"detector did not return a label for {case['name']}")
        case["expected"] = kind


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--skeleton", default="all",
                        choices=["all", *sorted(SKELETONS)],
                        help="which skeleton(s) to generate (default: all)")
    parser.add_argument("--count", type=int, default=400,
                        help="positions per skeleton (default: 400)")
    parser.add_argument("--seed", type=int, default=20)
    parser.add_argument("--radius", type=int, default=3,
                        help="interference stones are placed within this Chebyshev radius")
    parser.add_argument("--min-extra", type=int, default=3)
    parser.add_argument("--max-extra", type=int, default=8)
    parser.add_argument("--black-prob", type=float, default=0.7,
                        help="probability an interference stone is black (default: 0.7)")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--no-label", action="store_true",
                        help="skip detector labelling (omit the expected field)")
    parser.add_argument("--label-timeout", type=float, default=120.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.min_extra < 0 or args.max_extra < args.min_extra:
        raise SystemExit("invalid --min-extra/--max-extra range")

    skeletons = sorted(SKELETONS) if args.skeleton == "all" else [args.skeleton]
    cases: list[dict] = []
    for skeleton in skeletons:
        rng = random.Random(args.seed + SKELETON_SEED_OFFSET[skeleton])
        for index in range(max(0, args.count)):
            cases.append(generate_case(
                skeleton, index, rng,
                radius=args.radius, min_extra=args.min_extra,
                max_extra=args.max_extra, black_prob=args.black_prob,
            ))

    if not args.no_label:
        label_with_detector(cases, timeout=args.label_timeout)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as handle:
        for case in cases:
            handle.write(json.dumps(case) + "\n")

    summary: dict[str, int] = {}
    for case in cases:
        kind = case.get("expected", "unlabelled")
        summary[kind] = summary.get(kind, 0) + 1
    print(f"generated {len(cases)} cases -> {args.output}")
    print("distribution: " + ", ".join(f"{k}:{v}" for k, v in sorted(summary.items())))
    return 0


if __name__ == "__main__":
    sys.exit(main())
