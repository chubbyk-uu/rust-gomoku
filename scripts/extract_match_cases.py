#!/usr/bin/env python3
"""Extract fixed match cases from Gomocup match result JSON files."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

BOARD_SIZE = 15
BLACK = 1
WHITE = -1


def side_for_index(index: int) -> int:
    return BLACK if index % 2 == 0 else WHITE


def side_name(side: int) -> str:
    return "black" if side == BLACK else "white"


def in_bounds(x: int, y: int) -> bool:
    return 0 <= x < BOARD_SIZE and 0 <= y < BOARD_SIZE


def winner_after(moves: list[dict[str, int]], x: int, y: int, side: int) -> bool:
    occupied = {(move["x"], move["y"]): move["side"] for move in moves}
    for dx, dy in [(1, 0), (0, 1), (1, 1), (1, -1)]:
        count = 1
        for sign in [1, -1]:
            cx = x + dx * sign
            cy = y + dy * sign
            while in_bounds(cx, cy) and occupied.get((cx, cy)) == side:
                count += 1
                cx += dx * sign
                cy += dy * sign
        if count >= 5:
            return True
    return False


def slug(value: str) -> str:
    cleaned = re.sub(r"[^a-zA-Z0-9]+", "_", value).strip("_").lower()
    return cleaned or "case"


def parse_plies(raw: str) -> list[int]:
    plies = []
    for part in raw.split(","):
        part = part.strip()
        if part:
            plies.append(int(part))
    return sorted(set(plies))


def valid_prefix(moves: list[dict[str, Any]], ply: int) -> bool:
    seen: set[tuple[int, int]] = set()
    prefix: list[dict[str, int]] = []
    for index, move in enumerate(moves[:ply]):
        x = int(move["x"])
        y = int(move["y"])
        side = int(move.get("side", side_for_index(index)))
        if side != side_for_index(index):
            return False
        if not in_bounds(x, y) or (x, y) in seen:
            return False
        normalized = {"x": x, "y": y, "side": side}
        prefix.append(normalized)
        seen.add((x, y))
        if winner_after(prefix, x, y, side):
            return False
    return True


def case_from_game(
    *,
    source: Path,
    game_index: int,
    game: dict[str, Any],
    ply: int,
    tags: list[str],
) -> dict[str, Any] | None:
    moves = game.get("moves") or []
    if ply <= 0 or ply >= len(moves):
        return None
    if not valid_prefix(moves, ply):
        return None
    prefix = [[int(move["x"]), int(move["y"])] for move in moves[:ply]]
    source_name = slug(source.stem)
    opening_index = game.get("opening_index", game.get("case_index", game_index))
    name = f"{source_name}_g{game_index}_case{opening_index}_ply{ply}"
    case_tags = list(dict.fromkeys([*tags, f"ply{ply}", source_name]))
    return {
        "name": name,
        "moves": prefix,
        "side_to_move": side_name(side_for_index(ply)),
        "tags": case_tags,
    }


def extract_cases(
    inputs: list[Path],
    plies: list[int],
    tags: list[str],
    max_cases: int | None,
    max_cases_per_input: int | None,
) -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    seen: set[tuple[tuple[int, int], ...]] = set()
    for path in inputs:
        input_count = 0
        data = json.loads(path.read_text(encoding="utf-8"))
        for game_index, game in enumerate(data.get("games", [])):
            for ply in plies:
                case = case_from_game(
                    source=path,
                    game_index=game_index,
                    game=game,
                    ply=ply,
                    tags=tags,
                )
                if case is None:
                    continue
                key = tuple((x, y) for x, y in case["moves"])
                if key in seen:
                    continue
                seen.add(key)
                cases.append(case)
                input_count += 1
                if max_cases is not None and len(cases) >= max_cases:
                    return cases
                if max_cases_per_input is not None and input_count >= max_cases_per_input:
                    break
            if max_cases_per_input is not None and input_count >= max_cases_per_input:
                break
    return cases


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", type=Path)
    parser.add_argument("--plies", default="10,20,30,40,50")
    parser.add_argument("--tag", action="append", default=[])
    parser.add_argument("--max-cases", type=int)
    parser.add_argument("--max-cases-per-input", type=int)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    cases = extract_cases(
        [path.expanduser().resolve() for path in args.inputs],
        parse_plies(args.plies),
        args.tag,
        args.max_cases,
        args.max_cases_per_input,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as handle:
        for case in cases:
            handle.write(json.dumps(case, ensure_ascii=False, separators=(",", ":")) + "\n")
    print(f"wrote {len(cases)} case(s) to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
