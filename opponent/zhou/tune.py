#!/usr/bin/env python3
"""五子棋 AI 评估函数自动调优器。

借鉴 autoresearch 的思路：参数变异 → AI 对战 → 评估胜率 → 保留改进 → 循环迭代。
使用进化策略（Evolutionary Strategy）优化评估函数中的各项分值和权重。
"""

import copy
import json
import math
import multiprocessing as mp
import os
import random
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

# 确保能 import gomoku
sys.path.insert(0, str(Path(__file__).parent / "src"))

from gomoku.board import Board
from gomoku.config import BOARD_SIZE, Player


# ============================================================================
# 参数化的评估器（不依赖全局变量，方便用不同参数对战）
# ============================================================================

@dataclass
class EvalParams:
    """评估函数的所有可调参数。"""

    # 棋型评分: (count, blocks) -> score
    score_five: int = 100_000
    score_open_four: int = 10_000   # 活四 (4,0)
    score_half_four: int = 1_000    # 冲四 (4,1)
    score_open_three: int = 1_000   # 活三 (3,0)
    score_sleep_three: int = 100    # 眠三 (3,1)
    score_open_two: int = 100       # 活二 (2,0)
    score_sleep_two: int = 10       # 眠二 (2,1)
    score_open_one: int = 10        # 活一 (1,0)
    score_sleep_one: int = 1        # 眠一 (1,1)

    # 组合棋型加分
    combo_double_open_three: int = 5_000
    combo_open_three_half_four: int = 5_000

    # 防守权重
    defense_weight: float = 1.5

    def to_score_table(self) -> dict[tuple[int, int], int]:
        return {
            (5, 0): self.score_five, (5, 1): self.score_five, (5, 2): self.score_five,
            (4, 0): self.score_open_four, (4, 1): self.score_half_four,
            (3, 0): self.score_open_three, (3, 1): self.score_sleep_three,
            (2, 0): self.score_open_two, (2, 1): self.score_sleep_two,
            (1, 0): self.score_open_one, (1, 1): self.score_sleep_one,
        }

    def to_dict(self) -> dict:
        return {
            "score_open_four": self.score_open_four,
            "score_half_four": self.score_half_four,
            "score_open_three": self.score_open_three,
            "score_sleep_three": self.score_sleep_three,
            "score_open_two": self.score_open_two,
            "score_sleep_two": self.score_sleep_two,
            "score_open_one": self.score_open_one,
            "score_sleep_one": self.score_sleep_one,
            "combo_double_open_three": self.combo_double_open_three,
            "combo_open_three_half_four": self.combo_open_three_half_four,
            "defense_weight": round(self.defense_weight, 3),
        }


_DIRECTIONS = [(1, 0), (0, 1), (1, 1), (1, -1)]


def evaluate_with_params(board: Board, ai_player: Player, params: EvalParams) -> int:
    """用指定参数集评估棋盘。"""
    opponent = Player.WHITE if ai_player == Player.BLACK else Player.BLACK
    ai_score = _score_for_params(board, ai_player, params)
    opp_score = _score_for_params(board, opponent, params)
    return ai_score - int(opp_score * params.defense_weight)


def _score_for_params(board: Board, player: Player, params: EvalParams) -> int:
    grid = board.grid
    score_table = params.to_score_table()
    total = 0
    open_threes = 0
    half_fours = 0

    for i in range(BOARD_SIZE):
        for j in range(BOARD_SIZE):
            if grid[i][j] != player:
                continue
            for dr, dc in _DIRECTIONS:
                prev_r, prev_c = i - dr, j - dc
                if (0 <= prev_r < BOARD_SIZE and 0 <= prev_c < BOARD_SIZE
                        and grid[prev_r][prev_c] == player):
                    continue

                count = 0
                r, c = i, j
                while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE and grid[r][c] == player:
                    count += 1
                    r += dr
                    c += dc

                blocks = 0
                if r < 0 or r >= BOARD_SIZE or c < 0 or c >= BOARD_SIZE or grid[r][c] != Player.NONE:
                    blocks += 1
                pr, pc = i - dr, j - dc
                if pr < 0 or pr >= BOARD_SIZE or pc < 0 or pc >= BOARD_SIZE or grid[pr][pc] != Player.NONE:
                    blocks += 1

                if count >= 5:
                    total += score_table[(5, 0)]
                elif blocks < 2:
                    total += score_table.get((count, blocks), 0)
                    if count >= 4 and blocks == 0:
                        pass  # open_fours (not used in combos currently)
                    elif count >= 4 and blocks == 1:
                        half_fours += 1
                    elif count == 3 and blocks == 0:
                        open_threes += 1

    if open_threes >= 2:
        total += params.combo_double_open_three
    if open_threes >= 1 and half_fours >= 1:
        total += params.combo_open_three_half_four

    return total


# ============================================================================
# 参数化的搜索器
# ============================================================================

class ParamSearcher:
    """使用自定义参数的 Minimax 搜索器。"""

    def __init__(self, depth: int, ai_player: Player, params: EvalParams):
        self.depth = depth
        self.ai_player = ai_player
        self._opponent = Player.WHITE if ai_player == Player.BLACK else Player.BLACK
        self.params = params

    def find_best_move(self, board: Board) -> Optional[tuple[int, int]]:
        _, move = self._minimax(board, self.depth, -math.inf, math.inf, True)
        return move

    def _minimax(self, board, depth, alpha, beta, maximizing):
        if depth == 0:
            return evaluate_with_params(board, self.ai_player, self.params), None

        moves = board.get_candidate_moves()
        if not moves:
            return evaluate_with_params(board, self.ai_player, self.params), None

        current_player = self.ai_player if maximizing else self._opponent
        scored = []
        for r, c in moves:
            board.place(r, c, current_player)
            s = evaluate_with_params(board, self.ai_player, self.params)
            board.undo()
            scored.append((r, c, s))
        scored.sort(key=lambda x: x[2], reverse=maximizing)
        moves = [(r, c) for r, c, _ in scored[:MAX_CANDIDATES]]

        best_move = None
        if maximizing:
            best_score = -math.inf
            for row, col in moves:
                board.place(row, col, current_player)
                if board.check_win(row, col):
                    board.undo()
                    return 100_000.0, (row, col)
                score, _ = self._minimax(board, depth - 1, alpha, beta, False)
                board.undo()
                if score > best_score:
                    best_score = score
                    best_move = (row, col)
                alpha = max(alpha, score)
                if beta <= alpha:
                    break
            return best_score, best_move
        else:
            best_score = math.inf
            for row, col in moves:
                board.place(row, col, current_player)
                if board.check_win(row, col):
                    board.undo()
                    return -100_000.0, (row, col)
                score, _ = self._minimax(board, depth - 1, alpha, beta, True)
                board.undo()
                if score < best_score:
                    best_score = score
                    best_move = (row, col)
                beta = min(beta, score)
                if beta <= alpha:
                    break
            return best_score, best_move


# ============================================================================
# AI vs AI 对战
# ============================================================================

MAX_MOVES = 120  # 单局最大步数
RANDOM_OPENING_MOVES = 2  # 随机开局步数（前N步随机下，制造多样性）
MAX_CANDIDATES = 15  # 搜索时最多考虑的候选点数
GAME_TIMEOUT = 60  # 单局超时秒数


def _random_opening(board: Board, n_moves: int) -> None:
    """在棋盘上随机落 n_moves 步棋作为开局。"""
    center = BOARD_SIZE // 2
    # 开局区域限制在中心 5x5 范围内
    candidates = [(r, c) for r in range(center - 2, center + 3)
                  for c in range(center - 2, center + 3)]
    random.shuffle(candidates)

    current = Player.BLACK
    placed = 0
    for r, c in candidates:
        if placed >= n_moves:
            break
        if board.place(r, c, current):
            if board.check_win(r, c):
                board.undo()
                continue
            current = Player.WHITE if current == Player.BLACK else Player.BLACK
            placed += 1


def play_game(params_black: EvalParams, params_white: EvalParams,
              depth: int = 2, random_opening: int = RANDOM_OPENING_MOVES) -> str:
    """两套参数对战一局，返回 "black" / "white" / "draw"。"""
    board = Board()
    deadline = time.time() + GAME_TIMEOUT

    # 随机开局：前几步随机下，制造局面多样性
    if random_opening > 0:
        _random_opening(board, random_opening)

    searcher_b = ParamSearcher(depth, Player.BLACK, params_black)
    searcher_w = ParamSearcher(depth, Player.WHITE, params_white)

    # 确定当前该谁下
    n_existing = len(board.move_history)
    current = Player.BLACK if n_existing % 2 == 0 else Player.WHITE

    for _ in range(MAX_MOVES):
        if time.time() > deadline:
            return "draw"  # 超时判和
        searcher = searcher_b if current == Player.BLACK else searcher_w
        move = searcher.find_best_move(board)
        if move is None:
            return "draw"
        r, c = move
        board.place(r, c, current)
        if board.check_win(r, c):
            return "black" if current == Player.BLACK else "white"
        if board.is_full():
            return "draw"
        current = Player.WHITE if current == Player.BLACK else Player.BLACK

    return "draw"


def _play_game_wrapper(args):
    """multiprocessing 用的包装函数。"""
    params_black, params_white, depth = args
    return play_game(params_black, params_white, depth)


def evaluate_params(candidate: EvalParams, baseline: EvalParams,
                    num_games: int = 6, depth: int = 2,
                    pool: Optional[mp.Pool] = None) -> dict:
    """评估候选参数 vs 基线参数的胜率。

    各执黑白各半，保证公平性。返回 {wins, losses, draws, win_rate}。
    """
    half = num_games // 2
    tasks = []
    # 候选执黑 half 局
    for _ in range(half):
        tasks.append((candidate, baseline, depth))
    # 候选执白 half 局
    for _ in range(half):
        tasks.append((baseline, candidate, depth))

    if pool:
        results = pool.map(_play_game_wrapper, tasks)
    else:
        results = [_play_game_wrapper(t) for t in tasks]

    wins = 0
    losses = 0
    draws = 0
    for i, result in enumerate(results):
        if i < half:
            # 候选执黑
            if result == "black":
                wins += 1
            elif result == "white":
                losses += 1
            else:
                draws += 1
        else:
            # 候选执白
            if result == "white":
                wins += 1
            elif result == "black":
                losses += 1
            else:
                draws += 1

    total = wins + losses + draws
    win_rate = (wins + 0.5 * draws) / total if total > 0 else 0.5
    return {"wins": wins, "losses": losses, "draws": draws, "win_rate": win_rate}


# ============================================================================
# 进化策略：变异与选择
# ============================================================================

# 可调参数的名称和变异范围（相对比例）
TUNABLE_INT_PARAMS = [
    "score_open_four", "score_half_four", "score_open_three",
    "score_sleep_three", "score_open_two", "score_sleep_two",
    "score_open_one", "score_sleep_one",
    "combo_double_open_three", "combo_open_three_half_four",
]

TUNABLE_FLOAT_PARAMS = [
    "defense_weight",
]


def mutate(params: EvalParams, mutation_rate: float = 0.3,
           mutation_strength: float = 0.3) -> EvalParams:
    """对参数进行随机变异。

    mutation_rate: 每个参数被变异的概率。
    mutation_strength: 变异幅度（相对于当前值的比例）。
    """
    new = copy.deepcopy(params)

    for name in TUNABLE_INT_PARAMS:
        if random.random() < mutation_rate:
            val = getattr(new, name)
            delta = int(val * mutation_strength * random.gauss(0, 1))
            new_val = max(1, val + delta)  # 至少为 1
            setattr(new, name, new_val)

    for name in TUNABLE_FLOAT_PARAMS:
        if random.random() < mutation_rate:
            val = getattr(new, name)
            delta = val * mutation_strength * random.gauss(0, 1)
            new_val = max(0.1, val + delta)
            setattr(new, name, round(new_val, 3))

    return new


# ============================================================================
# 主循环
# ============================================================================

RESULTS_FILE = Path(__file__).parent / "tune_results.jsonl"
BEST_PARAMS_FILE = Path(__file__).parent / "best_params.json"


def load_best_params() -> EvalParams:
    """加载已保存的最佳参数，没有则返回默认。"""
    if BEST_PARAMS_FILE.exists():
        with open(BEST_PARAMS_FILE) as f:
            data = json.load(f)
        p = EvalParams()
        for k, v in data.items():
            if hasattr(p, k):
                setattr(p, k, v)
        print(f"  已加载保存的最佳参数: {BEST_PARAMS_FILE}")
        return p
    return EvalParams()


def save_best_params(params: EvalParams):
    with open(BEST_PARAMS_FILE, "w") as f:
        json.dump(params.to_dict(), f, indent=2, ensure_ascii=False)


def log_result(gen: int, win_rate: float, accepted: bool, params: EvalParams, detail: dict):
    entry = {
        "generation": gen,
        "win_rate": round(win_rate, 4),
        "accepted": accepted,
        "wins": detail["wins"],
        "losses": detail["losses"],
        "draws": detail["draws"],
        "params": params.to_dict(),
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
    }
    with open(RESULTS_FILE, "a") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def main():
    import argparse
    parser = argparse.ArgumentParser(description="五子棋 AI 评估函数自动调优")
    parser.add_argument("--generations", type=int, default=50, help="迭代代数")
    parser.add_argument("--games", type=int, default=20, help="每代对战局数（偶数）")
    parser.add_argument("--depth", type=int, default=3, help="搜索深度（2=快速,3=慢但更准）")
    parser.add_argument("--workers", type=int, default=0, help="并行进程数（0=CPU数-1）")
    parser.add_argument("--mutation-rate", type=float, default=0.4, help="变异概率")
    parser.add_argument("--mutation-strength", type=float, default=0.25, help="变异幅度")
    parser.add_argument("--threshold", type=float, default=0.55, help="胜率阈值，高于此才接受")
    parser.add_argument("--population", type=int, default=5, help="每代候选数量")
    parser.add_argument("--confirm-games", type=int, default=40, help="确认赛局数")
    args = parser.parse_args()

    if args.games % 2 != 0:
        args.games += 1
    if args.confirm_games % 2 != 0:
        args.confirm_games += 1

    workers = args.workers or max(1, mp.cpu_count() - 1)

    print("=" * 60)
    print("  五子棋 AI 评估函数自动调优器 (锦标赛模式)")
    print("=" * 60)
    print(f"  迭代代数: {args.generations}")
    print(f"  每代候选: {args.population} 个")
    print(f"  初赛对战: {args.games} 局 (各执黑白 {args.games // 2} 局)")
    print(f"  确认赛: {args.confirm_games} 局")
    print(f"  搜索深度: {args.depth}")
    print(f"  并行进程: {workers}")
    print(f"  变异概率: {args.mutation_rate}")
    print(f"  变异幅度: {args.mutation_strength}")
    print("=" * 60)
    sys.stdout.flush()

    best = load_best_params()
    print(f"\n  基线参数: {best.to_dict()}\n")
    sys.stdout.flush()

    accepted_count = 0
    pool = mp.Pool(workers)

    try:
        for gen in range(1, args.generations + 1):
            t0 = time.time()

            # 生成多个候选
            candidates = [mutate(best, args.mutation_rate, args.mutation_strength)
                          for _ in range(args.population)]

            # 初赛：每个候选 vs 当前最佳
            results = []
            for i, cand in enumerate(candidates):
                detail = evaluate_params(cand, best,
                                         num_games=args.games,
                                         depth=args.depth,
                                         pool=pool)
                results.append((i, detail["win_rate"], detail))

            # 找胜率最高的候选
            results.sort(key=lambda x: x[1], reverse=True)
            best_idx, best_wr, best_detail = results[0]
            best_candidate = candidates[best_idx]

            elapsed = time.time() - t0

            # 打印初赛结果
            wrs = " | ".join(f"{wr:.0%}" for _, wr, _ in results)
            print(
                f"  [{gen:3d}/{args.generations}] "
                f"初赛胜率: [{wrs}]  最高={best_wr:.1%} "
                f"({best_detail['wins']}W-{best_detail['losses']}L-{best_detail['draws']}D) "
                f"({elapsed:.1f}s)"
            )

            # 如果初赛最佳胜率 >= 60%，进入确认赛
            if best_wr >= 0.60:
                confirm = evaluate_params(best_candidate, best,
                                          num_games=args.confirm_games,
                                          depth=args.depth,
                                          pool=pool)
                confirm_wr = confirm["win_rate"]
                print(
                    f"         确认赛: {confirm_wr:.1%} "
                    f"({confirm['wins']}W-{confirm['losses']}L-{confirm['draws']}D)"
                )

                if confirm_wr >= args.threshold:
                    accepted_count += 1
                    best = best_candidate
                    save_best_params(best)
                    print(f"         ACCEPTED ✓ -> {best.to_dict()}")
                else:
                    print(f"         确认赛未通过 ✗")
            else:
                print(f"         初赛未达标 ✗")

            log_result(gen, best_wr, best_wr >= 0.60, best_candidate, best_detail)
            sys.stdout.flush()

    except KeyboardInterrupt:
        print("\n\n  调优已中断。")
    finally:
        pool.close()
        pool.join()

    print("\n" + "=" * 60)
    print(f"  调优完成! 共接受 {accepted_count} 次改进")
    print(f"  最终最佳参数: {best.to_dict()}")
    print(f"  参数已保存到: {BEST_PARAMS_FILE}")
    print(f"  详细日志: {RESULTS_FILE}")
    print("=" * 60)


if __name__ == "__main__":
    main()
