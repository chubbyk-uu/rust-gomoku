#!/usr/bin/env python3
"""Micro-benchmarks for Gomoku search hotspots.

This script is read-only with respect to game semantics. It measures the
current cost of the main hot paths so later Cython work can target the
highest-value replacements first.
"""

from __future__ import annotations

import statistics
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from gomoku.ai.evaluator import evaluate, evaluate_local, evaluate_local_python
from gomoku.ai.searcher import AISearcher
from gomoku.ai.vcf import PythonVCFBackend, init_zobrist
from gomoku.board import Board
from gomoku.config import Player


@dataclass(frozen=True)
class BenchCase:
    name: str
    func: Callable[[], object]
    iterations: int


def make_board(moves: list[tuple[int, int, Player]]) -> Board:
    board = Board()
    for row, col, player in moves:
        ok = board.place(row, col, player)
        if not ok:
            raise ValueError(f"invalid move in fixture: {(row, col, player)}")
    return board


def run_case(case: BenchCase) -> tuple[float, float, float]:
    samples_ms: list[float] = []
    for _ in range(5):
        t0 = time.perf_counter()
        for _ in range(case.iterations):
            case.func()
        elapsed_ms = (time.perf_counter() - t0) * 1000
        samples_ms.append(elapsed_ms)
    return min(samples_ms), statistics.median(samples_ms), max(samples_ms)


def format_ms(value: float) -> str:
    if value >= 1000:
        return f"{value / 1000:.3f}s"
    return f"{value:.2f}ms"


def main() -> None:
    tactical_board = make_board(
        [
            (7, 7, Player.BLACK),
            (7, 8, Player.WHITE),
            (6, 7, Player.BLACK),
            (8, 8, Player.WHITE),
            (5, 7, Player.BLACK),
            (6, 8, Player.WHITE),
            (8, 7, Player.BLACK),
            (7, 6, Player.WHITE),
            (9, 7, Player.BLACK),
            (5, 8, Player.WHITE),
            (10, 7, Player.BLACK),
            (7, 9, Player.WHITE),
        ]
    )
    midgame_board = make_board(
        [
            (7, 7, Player.BLACK),
            (7, 8, Player.WHITE),
            (8, 8, Player.BLACK),
            (6, 7, Player.WHITE),
            (8, 7, Player.BLACK),
            (6, 8, Player.WHITE),
            (9, 7, Player.BLACK),
            (5, 8, Player.WHITE),
            (7, 6, Player.BLACK),
            (7, 9, Player.WHITE),
            (6, 6, Player.BLACK),
            (8, 9, Player.WHITE),
            (9, 8, Player.BLACK),
            (5, 7, Player.WHITE),
            (10, 7, Player.BLACK),
            (4, 8, Player.WHITE),
            (8, 6, Player.BLACK),
            (6, 9, Player.WHITE),
        ]
    )
    vcf_board = make_board(
        [
            (7, 0, Player.WHITE),
            (7, 1, Player.WHITE),
            (7, 2, Player.WHITE),
            (5, 3, Player.WHITE),
            (6, 3, Player.WHITE),
            (8, 3, Player.WHITE),
            (14, 14, Player.BLACK),
        ]
    )
    win_check_board = make_board(
        [
            (5, 0, Player.WHITE),
            (5, 1, Player.WHITE),
            (5, 2, Player.WHITE),
            (5, 3, Player.WHITE),
            (5, 4, Player.WHITE),
        ]
    )

    vcf_backend = PythonVCFBackend(init_zobrist())
    search_depth_2 = AISearcher(depth=2, ai_player=Player.WHITE)
    search_depth_1 = AISearcher(depth=1, ai_player=Player.WHITE)

    cases = [
        BenchCase(
            name="Board.check_win",
            func=lambda: win_check_board.check_win(5, 4),
            iterations=200_000,
        ),
        BenchCase(
            name="Board.get_candidate_moves",
            func=midgame_board.get_candidate_moves,
            iterations=20_000,
        ),
        BenchCase(
            name="evaluate_local_python",
            func=lambda: evaluate_local_python(tactical_board.grid, 7, 5, Player.BLACK),
            iterations=50_000,
        ),
        BenchCase(
            name="evaluate_local(native wrapper)",
            func=lambda: evaluate_local(tactical_board.grid, 7, 5, Player.BLACK),
            iterations=50_000,
        ),
        BenchCase(
            name="evaluate(full board)",
            func=lambda: evaluate(midgame_board, Player.WHITE),
            iterations=3_000,
        ),
        BenchCase(
            name="VCF.find_winning_move",
            func=lambda: vcf_backend.find_winning_move(vcf_board, Player.WHITE, max_depth=8),
            iterations=200,
        ),
        BenchCase(
            name="AISearcher.find_best_move(depth=1)",
            func=lambda: search_depth_1.find_best_move(vcf_board),
            iterations=100,
        ),
        BenchCase(
            name="AISearcher.find_best_move(depth=2)",
            func=lambda: search_depth_2.find_best_move(midgame_board),
            iterations=20,
        ),
    ]

    print("Gomoku hotspot benchmark")
    print(f"Python: {sys.version.split()[0]}")
    print(f"Project root: {ROOT}")
    print()
    print(f"{'Case':36} {'Iter':>8} {'Min':>12} {'Median':>12} {'Max':>12} {'Per call':>12}")
    print("-" * 98)

    try:
        for case in cases:
            best_ms, median_ms, worst_ms = run_case(case)
            per_call_ms = median_ms / case.iterations
            print(
                f"{case.name:36} "
                f"{case.iterations:8d} "
                f"{format_ms(best_ms):>12} "
                f"{format_ms(median_ms):>12} "
                f"{format_ms(worst_ms):>12} "
                f"{format_ms(per_call_ms):>12}"
            )
    finally:
        search_depth_1.close()
        search_depth_2.close()
        vcf_backend.close()


if __name__ == "__main__":
    main()
