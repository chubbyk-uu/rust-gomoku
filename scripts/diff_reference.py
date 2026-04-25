#!/usr/bin/env python3
"""Emit Python reference probe output for a fixed differential test case."""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import replace
from pathlib import Path

REFERENCE_ROOT_ENV = "PYGOMOKU_REF_ROOT"


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def resolve_reference_root(ref_root: str | Path | None = None) -> Path:
    if ref_root is not None:
        candidates = [Path(ref_root).expanduser()]
    elif os.environ.get(REFERENCE_ROOT_ENV):
        candidates = [Path(os.environ[REFERENCE_ROOT_ENV]).expanduser()]
    else:
        candidates = [
            repo_root() / "reference" / "pygomoku",
            Path.home() / "python_ws" / "pygomoku",
        ]

    for candidate in candidates:
        if (candidate / "pygomoku").is_dir():
            return candidate.resolve()

    searched = ", ".join(str(candidate) for candidate in candidates)
    raise RuntimeError(
        "Python reference not found. Set "
        f"{REFERENCE_ROOT_ENV}=/path/to/pygomoku. Searched: {searched}"
    )


def import_reference(ref_root: str | Path | None = None) -> None:
    ref_root = resolve_reference_root(ref_root)
    sys.path.insert(0, str(ref_root))


def apply_runtime(config, runtime: dict):
    if not runtime:
        return config
    values = {}
    if "compute_vcf" in runtime:
        values["compute_vcf"] = bool(runtime["compute_vcf"])
    if "nonroot_vcf" in runtime:
        values["nonroot_vcf"] = bool(runtime["nonroot_vcf"])
    if "compute_vct" in runtime:
        values["compute_vct"] = bool(runtime["compute_vct"])
    if "root_vct_depth" in runtime:
        values["root_vct_depth"] = max(0, int(runtime["root_vct_depth"]))
    if "static_board" in runtime:
        values["static_board"] = bool(runtime["static_board"])
    if "dynamic_board_margin" in runtime:
        values["dynamic_board_margin"] = max(0, int(runtime["dynamic_board_margin"]))
    # Newer Rust-only runtime knobs may not exist in the current reference
    # dataclass. Ignore unsupported keys here; cases that need strict reference
    # parity should only use knobs available on both sides.
    values = {key: value for key, value in values.items() if hasattr(config.runtime, key)}
    if not values:
        return config
    return replace(config, runtime=replace(config.runtime, **values))


def trace_json(trace: dict | None):
    trace = trace or {}
    move = trace.get("vct_move")
    if move is not None and move >= 0:
        from pygomoku.board import move_to_xy

        move = list(move_to_xy(move))
    else:
        move = None
    return {
        "used_vcf": bool(trace.get("used_vcf", False)),
        "vcf_found": bool(trace.get("vcf_found", False)),
        "used_vct": bool(trace.get("used_vct", False)),
        "vct_triggered": bool(trace.get("vct_triggered", False)),
        "vct_found": bool(trace.get("vct_found", False)),
        "vct_move": move,
        "vct_accepted": bool(trace.get("vct_accepted", False)),
        "vct_reject_reason": trace.get("vct_reject_reason"),
        "tactical_path": trace.get("tactical_path", "alphabeta"),
    }


def run_case(case: dict, ref_root: str | Path | None = None) -> dict:
    import_reference(ref_root)
    from pygomoku.board import Board, move_to_xy, xy_to_move
    from pygomoku.config import load_default_config
    from pygomoku.search.root import RootSearcher, SearchLimits

    board = Board(side_to_move=case.get("first_side", 1))
    for x, y in case["moves"]:
        board.play(xy_to_move(x, y), board.side_to_move)

    config = apply_runtime(load_default_config(), case.get("runtime") or {})
    limits_json = case.get("limits") or {}
    limits = SearchLimits(
        max_depth=int(limits_json.get("max_depth", config.root_search.depth)),
        root_width=int(limits_json.get("root_width", config.root_search.wide)),
        node_limit=limits_json.get("node_limit"),
        time_limit_ms=limits_json.get("time_limit_ms"),
    )

    searcher = RootSearcher(config)
    result = searcher.search(board, limits)
    move = list(move_to_xy(result.move))
    return {
        "name": case["name"],
        "board": {
            "side_to_move": board.side_to_move,
            "winner": board.winner,
            "move_count": board.move_count,
            "zobrist_key": str(board.zobrist_key),
        },
        "root": {
            "move": move,
            "score": result.score,
            "depth": result.depth,
            "nodes": result.nodes,
            "trace": trace_json(searcher.last_trace),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", required=True, type=Path)
    parser.add_argument(
        "--ref-root",
        type=Path,
        help=f"Python reference root; defaults to ${REFERENCE_ROOT_ENV}, ./reference/pygomoku, then ~/python_ws/pygomoku",
    )
    args = parser.parse_args()
    case = json.loads(args.case.read_text(encoding="utf-8"))
    print(json.dumps(run_case(case, args.ref_root), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
