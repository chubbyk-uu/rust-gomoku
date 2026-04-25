"""Gomoku board logic."""

import copy
from typing import Optional

from gomoku.config import AI_CANDIDATE_RANGE, BOARD_SIZE, Player

try:
    from gomoku.ai._eval_kernels import candidate_moves_native as _candidate_moves_native
except ImportError:  # pragma: no cover - optional native acceleration
    _candidate_moves_native = None


class Board:
    """封装五子棋棋盘状态与操作。

    Attributes:
        grid: 15x15 二维列表，值为 Player 枚举。
        move_history: 落子历史，每条记录为 (row, col, player)。
        last_move: 最后一手坐标 (row, col)，棋盘为空时为 None。
    """

    def __init__(self) -> None:
        self.grid: list[list[Player]] = [[Player.NONE] * BOARD_SIZE for _ in range(BOARD_SIZE)]
        self.move_history: list[tuple[int, int, Player]] = []
        self.last_move: Optional[tuple[int, int]] = None

    # ------------------------------------------------------------------
    # Mutation
    # ------------------------------------------------------------------

    def place(self, row: int, col: int, player: Player) -> bool:
        """在 (row, col) 落子。

        Args:
            row: 行坐标 [0, BOARD_SIZE)。
            col: 列坐标 [0, BOARD_SIZE)。
            player: 落子方，必须是 Player.BLACK 或 Player.WHITE。

        Returns:
            True 表示落子成功；False 表示坐标越界或该位置已有棋子。
        """
        if not (0 <= row < BOARD_SIZE and 0 <= col < BOARD_SIZE):
            return False
        if self.grid[row][col] != Player.NONE:
            return False
        self.grid[row][col] = player
        self.move_history.append((row, col, player))
        self.last_move = (row, col)
        return True

    def undo(self) -> Optional[tuple[int, int, Player]]:
        """撤销最后一手落子。

        Returns:
            被撤销的 (row, col, player)；历史为空时返回 None。
        """
        if not self.move_history:
            return None
        row, col, player = self.move_history.pop()
        self.grid[row][col] = Player.NONE
        self.last_move = (
            (self.move_history[-1][0], self.move_history[-1][1]) if self.move_history else None
        )
        return row, col, player

    # ------------------------------------------------------------------
    # Query
    # ------------------------------------------------------------------

    def check_win(self, row: int, col: int) -> bool:
        """检查 (row, col) 处的棋子是否构成五连珠。

        Args:
            row: 行坐标。
            col: 列坐标。

        Returns:
            True 表示该位置构成胜利；空位返回 False。
        """
        player = self.grid[row][col]
        if player == Player.NONE:
            return False
        directions = [(1, 0), (0, 1), (1, 1), (1, -1)]
        for dr, dc in directions:
            count = 1
            for sign in (1, -1):
                step = 1
                while True:
                    r = row + sign * dr * step
                    c = col + sign * dc * step
                    if 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE and self.grid[r][c] == player:
                        count += 1
                        step += 1
                    else:
                        break
            if count >= 5:
                return True
        return False

    def get_candidate_moves(self) -> list[tuple[int, int]]:
        """返回所有邻近已有棋子的空位（候选落子点）。

        若棋盘为空，直接返回天元（中心点）。优先走与 Python 语义对齐的原生加速；
        原生扩展不可用时退回 Python 实现。

        Returns:
            候选坐标列表 [(row, col), ...]。
        """
        if _candidate_moves_native is not None:
            return list(_candidate_moves_native(self.grid, AI_CANDIDATE_RANGE))
        return self._get_candidate_moves_python()

    def _get_candidate_moves_python(self) -> list[tuple[int, int]]:
        """Python fallback for candidate move generation."""
        if not self.move_history:
            return [(BOARD_SIZE // 2, BOARD_SIZE // 2)]

        candidates: list[tuple[int, int]] = []
        radius = AI_CANDIDATE_RANGE
        for i in range(BOARD_SIZE):
            for j in range(BOARD_SIZE):
                if self.grid[i][j] != Player.NONE:
                    continue
                found = False
                for di in range(-radius, radius + 1):
                    for dj in range(-radius, radius + 1):
                        ni, nj = i + di, j + dj
                        if (
                            0 <= ni < BOARD_SIZE
                            and 0 <= nj < BOARD_SIZE
                            and self.grid[ni][nj] != Player.NONE
                        ):
                            found = True
                            break
                    if found:
                        break
                if found:
                    candidates.append((i, j))
        return candidates

    def is_full(self) -> bool:
        """棋盘是否已落满。

        Returns:
            True 表示无空位。
        """
        return all(
            self.grid[i][j] != Player.NONE for i in range(BOARD_SIZE) for j in range(BOARD_SIZE)
        )

    def copy(self) -> "Board":
        """返回棋盘的深拷贝，用于 AI 搜索时的模拟落子。

        Returns:
            新的 Board 实例，状态与当前相同。
        """
        new_board = Board()
        new_board.grid = copy.deepcopy(self.grid)
        new_board.move_history = self.move_history.copy()
        new_board.last_move = self.last_move
        return new_board
