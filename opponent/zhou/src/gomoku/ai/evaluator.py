"""Board evaluation for Gomoku AI."""

from collections.abc import Iterable, Iterator

from gomoku.board import Board
from gomoku.config import BOARD_SIZE, Player

# 评分表：(连子数, 封堵端数) -> 分值
SCORE_TABLE: dict[tuple[int, int], int] = {
    (5, 0): 100_000,
    (5, 1): 100_000,
    (5, 2): 100_000,
    (4, 0): 10_000,  # 活四
    (4, 1): 1_000,  # 冲四
    (3, 0): 1_000,  # 活三
    (3, 1): 100,  # 眠三
    (2, 0): 100,  # 活二
    (2, 1): 10,  # 眠二
    (1, 0): 10,  # 活一
    (1, 1): 1,
}

# 组合棋型加分
COMBO_DOUBLE_OPEN_THREE: int = 5_000  # 双活三 ≈ 必杀
COMBO_OPEN_THREE_HALF_FOUR: int = 5_000  # 活三+冲四 ≈ 必杀

# 防守加权：对手威胁分的额外乘数
DEFENSE_WEIGHT: float = 1.5

_DIRECTIONS: list[tuple[int, int]] = [(1, 0), (0, 1), (1, 1), (1, -1)]
_BROKEN_FOUR_PATTERNS: tuple[str, ...] = ("10111", "11011", "11101")
_BROKEN_THREE_PATTERNS: tuple[str, ...] = ("011010", "010110")
_LOCAL_PATTERN_SCORES: dict[str, int] = {
    "11111": SCORE_TABLE[(5, 0)],
    "011110": SCORE_TABLE[(4, 0)],
    "211110": SCORE_TABLE[(4, 1)],
    "011112": SCORE_TABLE[(4, 1)],
    "10111": SCORE_TABLE[(4, 1)],
    "11011": SCORE_TABLE[(4, 1)],
    "11101": SCORE_TABLE[(4, 1)],
    "01110": SCORE_TABLE[(3, 0)],
    "011010": SCORE_TABLE[(3, 0)],
    "010110": SCORE_TABLE[(3, 0)],
}

try:
    from gomoku.ai._eval_kernels import evaluate_local_native as _evaluate_local_native
    from gomoku.ai._eval_kernels import evaluate_native as _evaluate_native
except ImportError:  # pragma: no cover - optional native acceleration
    _evaluate_local_native = None
    _evaluate_native = None


def get_score(count: int, blocks: int) -> int:
    """根据连子数和封堵端数返回单条线的评分。

    Args:
        count: 连续同色棋子数量。
        blocks: 两端中被对方棋子或边界封堵的端数（0、1 或 2）。

    Returns:
        该棋型的分值；无法形成威胁时返回 0。
    """
    if count >= 5:
        return SCORE_TABLE[(5, 0)]
    if blocks >= 2:
        return 0
    return SCORE_TABLE.get((count, blocks), 0)


def evaluate(board: Board, ai_player: Player) -> int:
    """评估当前棋盘对 ai_player 的净分值。

    计入组合棋型加分和防守加权。

    Args:
        board: 当前棋盘状态。
        ai_player: AI 执棋颜色。

    Returns:
        AI 总分 − 对手加权总分（正值对 AI 有利）。
    """
    if _evaluate_native is not None:
        return int(_evaluate_native(board.grid, ai_player, DEFENSE_WEIGHT))
    return evaluate_python(board, ai_player)


def evaluate_python(board: Board, ai_player: Player) -> int:
    """Pure Python fallback for full-board evaluation."""
    opponent = Player.WHITE if ai_player == Player.BLACK else Player.BLACK
    ai_score = _score_for(board, ai_player)
    opp_score = _score_for(board, opponent)
    return ai_score - int(opp_score * DEFENSE_WEIGHT)


def evaluate_local(grid: list[list[Player]], r: int, c: int, player: Player) -> int:
    """极速评估某个空点的局部攻防价值，仅扫描穿过该点的 4 条线。"""
    if _evaluate_local_native is not None:
        return int(_evaluate_local_native(grid, r, c, player))
    return evaluate_local_python(grid, r, c, player)


def evaluate_local_python(grid: list[list[Player]], r: int, c: int, player: Player) -> int:
    """极速评估某个空点的局部攻防价值，仅扫描穿过该点的 4 条线。"""
    if grid[r][c] != Player.NONE:
        return 0

    score = 0
    for dr, dc in _DIRECTIONS:
        score += _score_local_direction(grid, r, c, player, dr, dc)
    return score


def _score_for(board: Board, player: Player) -> int:
    """计算单方棋子的棋盘总分（含组合棋型加分）。

    Args:
        board: 当前棋盘状态。
        player: 待评估的一方。

    Returns:
        该方所有棋型分值之和 + 组合加分。
    """
    grid = board.grid
    total = 0
    half_fours = 0  # 冲四数
    open_threes = 0  # 活三数

    for i in range(BOARD_SIZE):
        for j in range(BOARD_SIZE):
            if grid[i][j] != player:
                continue
            for dr, dc in _DIRECTIONS:
                prev_r, prev_c = i - dr, j - dc
                if (
                    0 <= prev_r < BOARD_SIZE
                    and 0 <= prev_c < BOARD_SIZE
                    and grid[prev_r][prev_c] == player
                ):
                    continue

                count = 0
                r, c = i, j
                while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE and grid[r][c] == player:
                    count += 1
                    r += dr
                    c += dc

                blocks = 0
                if (
                    r < 0
                    or r >= BOARD_SIZE
                    or c < 0
                    or c >= BOARD_SIZE
                    or grid[r][c] != Player.NONE
                ):
                    blocks += 1
                pr, pc = i - dr, j - dc
                if (
                    pr < 0
                    or pr >= BOARD_SIZE
                    or pc < 0
                    or pc >= BOARD_SIZE
                    or grid[pr][pc] != Player.NONE
                ):
                    blocks += 1

                total += get_score(count, blocks)

                # 统计关键棋型数量
                if blocks < 2:
                    if count >= 4 and blocks == 1:
                        half_fours += 1
                    elif count == 3 and blocks == 0:
                        open_threes += 1

    # 模式匹配：补上跳四 / 跳三这类非纯连续棋型的识别。
    for line in _iter_line_strings(grid, player):
        broken_fours = sum(_count_pattern(line, pattern) for pattern in _BROKEN_FOUR_PATTERNS)
        broken_threes = sum(_count_pattern(line, pattern) for pattern in _BROKEN_THREE_PATTERNS)
        total += broken_fours * SCORE_TABLE[(4, 1)]
        total += broken_threes * SCORE_TABLE[(3, 0)]
        half_fours += broken_fours
        open_threes += broken_threes

    # 组合棋型加分
    if open_threes >= 2:
        total += COMBO_DOUBLE_OPEN_THREE
    if open_threes >= 1 and half_fours >= 1:
        total += COMBO_OPEN_THREE_HALF_FOUR

    return total


def _score_local_direction(
    grid: list[list[Player]],
    r: int,
    c: int,
    player: Player,
    dr: int,
    dc: int,
) -> int:
    score = _contiguous_point_score(grid, r, c, player, dr, dc)
    line = _build_local_line(grid, r, c, player, dr, dc, radius=4)
    center = 4

    for pattern, pattern_score in _LOCAL_PATTERN_SCORES.items():
        start = 0
        while True:
            idx = line.find(pattern, start)
            if idx == -1:
                break
            if idx <= center < idx + len(pattern):
                score = max(score, pattern_score)
            start = idx + 1

    return score


def _contiguous_point_score(
    grid: list[list[Player]],
    r: int,
    c: int,
    player: Player,
    dr: int,
    dc: int,
) -> int:
    count = 1
    blocks = 0

    nr, nc = r + dr, c + dc
    while 0 <= nr < BOARD_SIZE and 0 <= nc < BOARD_SIZE and grid[nr][nc] == player:
        count += 1
        nr += dr
        nc += dc
    if nr < 0 or nr >= BOARD_SIZE or nc < 0 or nc >= BOARD_SIZE or grid[nr][nc] != Player.NONE:
        blocks += 1

    nr, nc = r - dr, c - dc
    while 0 <= nr < BOARD_SIZE and 0 <= nc < BOARD_SIZE and grid[nr][nc] == player:
        count += 1
        nr -= dr
        nc -= dc
    if nr < 0 or nr >= BOARD_SIZE or nc < 0 or nc >= BOARD_SIZE or grid[nr][nc] != Player.NONE:
        blocks += 1

    return get_score(count, blocks)


def _build_local_line(
    grid: list[list[Player]],
    r: int,
    c: int,
    player: Player,
    dr: int,
    dc: int,
    radius: int,
) -> str:
    chars: list[str] = []
    for step in range(-radius, radius + 1):
        nr = r + dr * step
        nc = c + dc * step
        if step == 0:
            chars.append("1")
        elif nr < 0 or nr >= BOARD_SIZE or nc < 0 or nc >= BOARD_SIZE:
            chars.append("2")
        else:
            chars.append(_encode_cell(grid[nr][nc], player))
    return "".join(chars)


def _iter_line_strings(
    grid: list[list[Player]],
    player: Player,
) -> Iterator[str]:
    for r in range(BOARD_SIZE):
        yield _encode_line((grid[r][c] for c in range(BOARD_SIZE)), player)

    for c in range(BOARD_SIZE):
        yield _encode_line((grid[r][c] for r in range(BOARD_SIZE)), player)

    for start_c in range(BOARD_SIZE):
        yield _encode_line(_walk_line(grid, 0, start_c, 1, 1), player)
    for start_r in range(1, BOARD_SIZE):
        yield _encode_line(_walk_line(grid, start_r, 0, 1, 1), player)

    for start_c in range(BOARD_SIZE):
        yield _encode_line(_walk_line(grid, 0, start_c, 1, -1), player)
    for start_r in range(1, BOARD_SIZE):
        yield _encode_line(_walk_line(grid, start_r, BOARD_SIZE - 1, 1, -1), player)


def _walk_line(
    grid: list[list[Player]],
    r: int,
    c: int,
    dr: int,
    dc: int,
) -> Iterator[Player]:
    while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE:
        yield grid[r][c]
        r += dr
        c += dc


def _encode_line(cells: Iterable[Player], player: Player) -> str:
    encoded = "".join(_encode_cell(cell, player) for cell in cells)
    return f"2{encoded}2"


def _encode_cell(cell: Player, player: Player) -> str:
    if cell == player:
        return "1"
    if cell == Player.NONE:
        return "0"
    return "2"


def _count_pattern(line: str, pattern: str) -> int:
    count = 0
    start = 0
    while True:
        idx = line.find(pattern, start)
        if idx == -1:
            return count
        count += 1
        start = idx + 1
