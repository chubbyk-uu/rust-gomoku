"""Minimax searcher with alpha-beta pruning for Gomoku AI.

优化：
1. 置换表 (Transposition Table) - 缓存已评估局面，避免重复搜索
2. 迭代加深 (Iterative Deepening) - 用浅层搜索结果优化深层排序
3. 杀手启发 (Killer Heuristic) - 优先尝试在同深度产生截断的着法
"""

import math
from dataclasses import dataclass
from typing import Optional

from gomoku.ai.evaluator import DEFENSE_WEIGHT, evaluate, evaluate_local
from gomoku.ai.vcf import create_vcf_backend, init_zobrist
from gomoku.board import Board
from gomoku.config import (
    AI_MAX_CANDIDATES,
    AI_VCF_MAX_DEPTH,
    BOARD_SIZE,
    Player,
)

try:
    from gomoku.ai._eval_kernels import order_search_moves_native as _order_search_moves_native
    from gomoku.ai._eval_kernels import order_moves_by_hotness_native as _order_moves_by_hotness_native
except ImportError:  # pragma: no cover - optional native acceleration
    _order_search_moves_native = None
    _order_moves_by_hotness_native = None

# 置换表条目类型
_EXACT = 0
_LOWER_BOUND = 1  # alpha 截断，真实值 >= 存储值
_UPPER_BOUND = 2  # beta 截断，真实值 <= 存储值
_WHITE_OPENING_BAD_MOVE_FILTER_ENABLED = True
_WHITE_OPENING_BAD_MOVE_FILTER_ROOT_TOP_K = 8


@dataclass
class DecisionTrace:
    source: str = ""
    move: Optional[tuple[int, int]] = None
    completed_depth: int = 0
    score: Optional[float] = None
    notes: list[str] | None = None


class AISearcher:
    """基于 Minimax + Alpha-Beta 剪枝的五子棋 AI 搜索器。

    Attributes:
        depth: 搜索深度（建议 2~4，>3 时速度明显下降）。
        ai_player: AI 执棋颜色。
    """

    def __init__(self, depth: int = 3, ai_player: Player = Player.WHITE) -> None:
        self.depth = depth
        self.ai_player = ai_player
        self._opponent = Player.WHITE if ai_player == Player.BLACK else Player.BLACK
        # 置换表: zobrist_hash -> (depth, score, flag, best_move)
        self._tt: dict[int, tuple[int, float, int, Optional[tuple[int, int]]]] = {}
        # 杀手表: depth -> [move1, move2]
        self._killers: dict[int, list[tuple[int, int]]] = {}
        # Zobrist 哈希用的随机数表
        self._zobrist = init_zobrist()
        # VCF backend: 默认 Python，可切换到 Swift/auto
        self._vcf = create_vcf_backend(self._zobrist)
        self._hash = 0
        self._eval_cache: dict[int, float] = {}
        self.last_decision_trace = DecisionTrace()
        self._opening_root_filter_note: Optional[str] = None

    def find_best_move(self, board: Board) -> Optional[tuple[int, int]]:
        """为 AI 找出当前局面下的最优落子位置。

        使用迭代加深：从浅层搜索到目标深度，利用浅层结果优化排序。

        Args:
            board: 当前棋盘状态（不会被修改）。

        Returns:
            最优落子坐标 (row, col)；无候选点时返回 None。
        """
        self.last_decision_trace = DecisionTrace()
        self._opening_root_filter_note = None
        self._tt.clear()
        self._killers.clear()
        self._eval_cache.clear()
        self._vcf.reset()
        self._hash = self._compute_hash(board)

        vcf_move = self._vcf.find_winning_move(
            board,
            self.ai_player,
            AI_VCF_MAX_DEPTH,
            current_hash=self._hash,
        )
        if vcf_move is not None:
            self.last_decision_trace = DecisionTrace(
                source="vcf_win",
                move=vcf_move,
                completed_depth=AI_VCF_MAX_DEPTH,
                notes=[f"backend={type(self._vcf).__name__}"],
            )
            return vcf_move

        vcf_block = self._vcf.find_blocking_move(
            board,
            self.ai_player,
            AI_VCF_MAX_DEPTH,
            current_hash=self._hash,
        )
        if vcf_block is not None:
            self.last_decision_trace = DecisionTrace(
                source="vcf_block",
                move=vcf_block,
                completed_depth=AI_VCF_MAX_DEPTH,
                notes=[f"backend={type(self._vcf).__name__}"],
            )
            return vcf_block

        best_move = None
        best_score: Optional[float] = None
        # 迭代加深
        for d in range(1, self.depth + 1):
            score, move = self._minimax(board, d, -math.inf, math.inf, maximizing=True)
            if move is not None:
                best_move = move
                best_score = score
        self.last_decision_trace = DecisionTrace(
            source="minimax" if best_move is not None else "no_move",
            move=best_move,
            completed_depth=self.depth,
            score=best_score,
        )
        if self._opening_root_filter_note is not None:
            notes = self.last_decision_trace.notes or []
            notes.append(self._opening_root_filter_note)
            self.last_decision_trace.notes = notes
        return best_move

    def close(self) -> None:
        self._vcf.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _compute_hash(self, board: Board) -> int:
        h = 0
        for i in range(BOARD_SIZE):
            for j in range(BOARD_SIZE):
                p = board.grid[i][j]
                if p != Player.NONE:
                    h ^= self._zobrist[i][j][p]
        return h

    def _evaluate_current(self, board: Board) -> float:
        cached = self._eval_cache.get(self._hash)
        if cached is not None:
            return cached

        score = float(evaluate(board, self.ai_player))
        self._eval_cache[self._hash] = score
        return score

    def _minimax(
        self,
        board: Board,
        depth: int,
        alpha: float,
        beta: float,
        maximizing: bool,
    ) -> tuple[float, Optional[tuple[int, int]]]:
        orig_alpha = alpha

        # 置换表查询
        tt_entry = self._tt.get(self._hash)
        tt_move = None
        if tt_entry is not None:
            tt_depth, tt_score, tt_flag, tt_move = tt_entry
            if tt_depth >= depth:
                if tt_flag == _EXACT:
                    return tt_score, tt_move
                elif tt_flag == _LOWER_BOUND:
                    alpha = max(alpha, tt_score)
                elif tt_flag == _UPPER_BOUND:
                    beta = min(beta, tt_score)
                if alpha >= beta:
                    return tt_score, tt_move

        if depth == 0:
            return self._evaluate_current(board), None

        moves = board.get_candidate_moves()
        if not moves:
            return self._evaluate_current(board), None

        # 候选点排序：置换表最佳着法 > 杀手着法 > 评估排序
        current_player = self.ai_player if maximizing else self._opponent
        moves = self._order_moves(board, moves, current_player, depth, tt_move)
        if (
            maximizing
            and depth == self.depth
            and self._should_apply_white_opening_bad_move_filter(board)
        ):
            moves = self._filter_white_opening_root_moves(board, moves)

        best_move: Optional[tuple[int, int]] = None

        if maximizing:
            best_score: float = -math.inf
            for row, col in moves:
                board.place(row, col, current_player)
                self._hash ^= self._zobrist[row][col][current_player]

                if board.check_win(row, col):
                    self._hash ^= self._zobrist[row][col][current_player]
                    board.undo()
                    self._tt_store(depth, 100_000.0, _EXACT, (row, col), orig_alpha, beta)
                    return 100_000.0, (row, col)

                score, _ = self._minimax(board, depth - 1, alpha, beta, False)

                self._hash ^= self._zobrist[row][col][current_player]
                board.undo()

                if score > best_score:
                    best_score = score
                    best_move = (row, col)
                alpha = max(alpha, score)
                if beta <= alpha:
                    self._add_killer(depth, (row, col))
                    break

            self._tt_store(depth, best_score, _EXACT, best_move, orig_alpha, beta)
            return best_score, best_move
        else:
            best_score = math.inf
            for row, col in moves:
                board.place(row, col, current_player)
                self._hash ^= self._zobrist[row][col][current_player]

                if board.check_win(row, col):
                    self._hash ^= self._zobrist[row][col][current_player]
                    board.undo()
                    self._tt_store(depth, -100_000.0, _EXACT, (row, col), orig_alpha, beta)
                    return -100_000.0, (row, col)

                score, _ = self._minimax(board, depth - 1, alpha, beta, True)

                self._hash ^= self._zobrist[row][col][current_player]
                board.undo()

                if score < best_score:
                    best_score = score
                    best_move = (row, col)
                beta = min(beta, score)
                if beta <= alpha:
                    self._add_killer(depth, (row, col))
                    break

            self._tt_store(depth, best_score, _EXACT, best_move, orig_alpha, beta)
            return best_score, best_move

    def _tt_store(
        self,
        depth: int,
        score: float,
        flag: int,
        best_move: Optional[tuple[int, int]],
        orig_alpha: float,
        beta: float,
    ) -> None:
        if score <= orig_alpha:
            flag = _UPPER_BOUND
        elif score >= beta:
            flag = _LOWER_BOUND
        else:
            flag = _EXACT

        existing = self._tt.get(self._hash)
        if existing is None or existing[0] <= depth:
            self._tt[self._hash] = (depth, score, flag, best_move)

    def _order_moves(
        self,
        board: Board,
        moves: list[tuple[int, int]],
        current_player: Player,
        depth: int,
        tt_move: Optional[tuple[int, int]],
        max_candidates: Optional[int] = AI_MAX_CANDIDATES,
    ) -> list[tuple[int, int]]:
        """排序候选着法：TT最佳 > 杀手着法 > 局部攻防热度。"""
        killers = self._killers.get(depth, []).copy()
        opponent = self._opponent if current_player == self.ai_player else self.ai_player

        if _order_search_moves_native is not None:
            return list(
                _order_search_moves_native(
                    board.grid,
                    moves,
                    current_player,
                    opponent,
                    tt_move,
                    killers,
                    DEFENSE_WEIGHT,
                    max_candidates,
                )
            )

        killer_set = set(killers)
        priority = []
        normal = []
        for r, c in moves:
            if tt_move and (r, c) == tt_move:
                priority.insert(0, (r, c))
            elif (r, c) in killer_set:
                priority.append((r, c))
            else:
                normal.append((r, c))

        # 只看局部热度，不再对每个候选点做一次真实落子 + 全盘扫描。
        grid = board.grid
        if _order_moves_by_hotness_native is not None:
            scored_moves = list(
                _order_moves_by_hotness_native(
                    grid,
                    normal,
                    current_player,
                    opponent,
                    DEFENSE_WEIGHT,
                    None,
                )
            )
        else:
            scored: list[tuple[int, int, float]] = []
            for r, c in normal:
                attack_score = evaluate_local(grid, r, c, current_player)
                defend_score = evaluate_local(grid, r, c, opponent)
                hotness = attack_score + defend_score * DEFENSE_WEIGHT
                scored.append((r, c, hotness))
            scored.sort(key=lambda item: item[2], reverse=True)
            scored_moves = [(r, c) for r, c, _ in scored]

        ordered = priority.copy()
        if max_candidates is None:
            ordered.extend(scored_moves)
            return ordered

        remaining_slots = max(max_candidates - len(priority), 0)
        ordered.extend(scored_moves[:remaining_slots])
        return ordered[:max_candidates]

    def _add_killer(self, depth: int, move: tuple[int, int]) -> None:
        if depth not in self._killers:
            self._killers[depth] = []
        kl = self._killers[depth]
        if move not in kl:
            kl.insert(0, move)
            if len(kl) > 2:
                kl.pop()

    def _should_apply_white_opening_bad_move_filter(self, board: Board) -> bool:
        return (
            _WHITE_OPENING_BAD_MOVE_FILTER_ENABLED
            and self.ai_player == Player.WHITE
            and len(board.move_history) == 1
        )

    @staticmethod
    def _is_diagonal_contact_opening_move(
        board: Board,
        move: tuple[int, int],
    ) -> bool:
        if len(board.move_history) != 1:
            return False
        row, col, player = board.move_history[0]
        if player != Player.BLACK:
            return False
        return abs(move[0] - row) == 1 and abs(move[1] - col) == 1

    def _white_opening_bad_move_should_eliminate(
        self,
        board: Board,
        move: tuple[int, int],
    ) -> bool:
        return self._is_diagonal_contact_opening_move(board, move)

    def _filter_white_opening_root_moves(
        self,
        board: Board,
        moves: list[tuple[int, int]],
    ) -> list[tuple[int, int]]:
        head = moves[:_WHITE_OPENING_BAD_MOVE_FILTER_ROOT_TOP_K]
        tail = moves[_WHITE_OPENING_BAD_MOVE_FILTER_ROOT_TOP_K:]
        survivors = [
            move
            for move in head
            if not self._white_opening_bad_move_should_eliminate(board, move)
        ]
        eliminated = len(head) - len(survivors)
        if not survivors:
            self._opening_root_filter_note = None
            return moves
        ordered = survivors + tail
        center = BOARD_SIZE // 2
        indexed_moves = list(enumerate(ordered))
        indexed_moves.sort(
            key=lambda item: (
                abs(item[1][0] - center) + abs(item[1][1] - center),
                item[0],
            )
        )
        self._opening_root_filter_note = (
            f"white_opening_safe_filter_kept={len(survivors)}/{len(head)} "
            f"eliminated={eliminated}"
        )
        return [move for _, move in indexed_moves]
