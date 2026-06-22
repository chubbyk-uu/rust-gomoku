#!/usr/bin/env python3
"""Generate deterministic Renju forbidden-move candidate fixtures.

It emits legal-looking 15x15 positions with alternating stones and one black
candidate per line. By default it leaves `expected` as `none`; with oracle
flags it can fill `expected` from `renju_forbid` and optionally cross-check
forbidden/none against Rapfi.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path
from typing import Any

BOARD_SIZE = 15


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def generate_case(index: int, rng: random.Random, min_plies: int, max_plies: int) -> dict[str, Any]:
    ply_count = rng.randint(min_plies, max_plies)
    occupied: set[tuple[int, int]] = set()
    moves: list[dict[str, int]] = []
    side = 1
    center = BOARD_SIZE // 2

    # Start near center so generated positions look like normal games and
    # produce useful candidate neighborhoods.
    first = (center, center)
    occupied.add(first)
    moves.append({"x": first[0], "y": first[1], "side": side})
    side = -side

    while len(moves) < ply_count:
        if occupied:
            anchors = list(occupied)
            ax, ay = rng.choice(anchors)
            candidates = [
                (ax + dx, ay + dy)
                for dx in range(-2, 3)
                for dy in range(-2, 3)
                if not (dx == 0 and dy == 0)
            ]
            rng.shuffle(candidates)
        else:
            candidates = []
        candidates.extend((rng.randrange(BOARD_SIZE), rng.randrange(BOARD_SIZE)) for _ in range(8))

        placed = None
        for x, y in candidates:
            if 0 <= x < BOARD_SIZE and 0 <= y < BOARD_SIZE and (x, y) not in occupied:
                placed = (x, y)
                break
        if placed is None:
            break
        occupied.add(placed)
        moves.append({"x": placed[0], "y": placed[1], "side": side})
        side = -side

    near_empty = []
    for ax, ay in occupied:
        for dx in range(-2, 3):
            for dy in range(-2, 3):
                point = (ax + dx, ay + dy)
                if (
                    0 <= point[0] < BOARD_SIZE
                    and 0 <= point[1] < BOARD_SIZE
                    and point not in occupied
                ):
                    near_empty.append(point)
    if near_empty:
        candidate = rng.choice(sorted(set(near_empty)))
    else:
        empty = [
            (x, y) for y in range(BOARD_SIZE) for x in range(BOARD_SIZE) if (x, y) not in occupied
        ]
        candidate = rng.choice(empty)
    return {
        "name": f"random_seed_case_{index}",
        "board_size": BOARD_SIZE,
        "moves": moves,
        "candidate": {"x": candidate[0], "y": candidate[1]},
        "expected": "none",
        "notes": "Generated scaffold case; expected value is not oracle-filled.",
    }


def import_oracle_compare():
    scripts_dir = Path(__file__).resolve().parent
    if str(scripts_dir) not in sys.path:
        sys.path.insert(0, str(scripts_dir))
    import renju_oracle_compare  # noqa: PLC0415

    return renju_oracle_compare


def fill_expected_from_renju_forbid(
    cases: list[dict[str, Any]], *, renju_forbid_root: Path | None, timeout_seconds: float
) -> None:
    oracle = import_oracle_compare()
    root = oracle.resolve_renju_forbid_root(renju_forbid_root)
    ok, detail = oracle.renju_forbid_available(root)
    if not ok:
        raise RuntimeError(f"renju_forbid unavailable: {detail}")
    fixtures = [oracle.parse_fixture(case, index + 1) for index, case in enumerate(cases)]
    results = oracle.run_renju_forbid(fixtures, root=root, timeout_seconds=timeout_seconds)
    for case, result in zip(cases, results):
        case["expected"] = result
        case["notes"] = "Generated case; expected value filled by renju_forbid oracle."


def verify_against_rapfi(
    cases: list[dict[str, Any]], *, rapfi_bin: Path | None, timeout_seconds: float
) -> None:
    oracle = import_oracle_compare()
    binary = oracle.resolve_rapfi_bin(rapfi_bin)
    ok, detail = oracle.rapfi_available(binary)
    if not ok or binary is None:
        raise RuntimeError(f"Rapfi unavailable: {detail}")
    fixtures = [oracle.parse_fixture(case, index + 1) for index, case in enumerate(cases)]
    mismatches = []
    for fixture in fixtures:
        points = oracle.rapfi_forbidden_points(
            fixture, rapfi_bin=binary, timeout_seconds=timeout_seconds
        )
        actual = (fixture.candidate.x, fixture.candidate.y) in points
        expected = oracle.expected_forbidden(fixture)
        if actual != expected:
            mismatches.append(fixture.name)
    if mismatches:
        raise RuntimeError(f"Rapfi mismatch for generated cases: {', '.join(mismatches)}")
    for case in cases:
        case["notes"] += " Rapfi forbidden/none cross-check passed."


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--count", type=int, default=10)
    parser.add_argument("--min-plies", type=int, default=5)
    parser.add_argument("--max-plies", type=int, default=40)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--fill-renju-forbid", action="store_true")
    parser.add_argument("--verify-rapfi", action="store_true")
    parser.add_argument("--renju-forbid-root", type=Path)
    parser.add_argument("--rapfi-bin", type=Path)
    parser.add_argument("--oracle-timeout", type=float, default=10.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    rng = random.Random(args.seed)
    cases = [
        generate_case(index, rng, args.min_plies, args.max_plies)
        for index in range(max(0, args.count))
    ]
    if args.fill_renju_forbid:
        fill_expected_from_renju_forbid(
            cases, renju_forbid_root=args.renju_forbid_root, timeout_seconds=args.oracle_timeout
        )
    if args.verify_rapfi:
        if not args.fill_renju_forbid:
            raise RuntimeError("--verify-rapfi requires --fill-renju-forbid")
        verify_against_rapfi(cases, rapfi_bin=args.rapfi_bin, timeout_seconds=args.oracle_timeout)
    text = "\n".join(json.dumps(case, separators=(",", ":")) for case in cases)
    if text:
        text += "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
