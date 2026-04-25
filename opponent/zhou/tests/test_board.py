"""Tests for Board class."""

from gomoku.board import _candidate_moves_native
from gomoku.board import Board
from gomoku.config import AI_CANDIDATE_RANGE, BOARD_SIZE, Player

# ---------------------------------------------------------------------------
# place & undo
# ---------------------------------------------------------------------------


def test_place_and_undo():
    board = Board()

    # 落子成功
    assert board.place(7, 7, Player.BLACK) is True
    assert board.grid[7][7] == Player.BLACK
    assert board.last_move == (7, 7)
    assert len(board.move_history) == 1

    # 重复落子失败
    assert board.place(7, 7, Player.WHITE) is False

    # 越界落子失败
    assert board.place(-1, 0, Player.BLACK) is False
    assert board.place(0, BOARD_SIZE, Player.BLACK) is False

    # 悔棋
    result = board.undo()
    assert result == (7, 7, Player.BLACK)
    assert board.grid[7][7] == Player.NONE
    assert board.last_move is None
    assert board.move_history == []

    # 空棋盘悔棋返回 None
    assert board.undo() is None


def test_undo_restores_last_move():
    board = Board()
    board.place(3, 3, Player.BLACK)
    board.place(4, 4, Player.WHITE)

    board.undo()
    assert board.last_move == (3, 3)

    board.undo()
    assert board.last_move is None


# ---------------------------------------------------------------------------
# check_win
# ---------------------------------------------------------------------------


def test_check_win_horizontal():
    board = Board()
    for col in range(5):
        board.place(0, col, Player.BLACK)
    assert board.check_win(0, 4) is True


def test_check_win_vertical():
    board = Board()
    for row in range(5):
        board.place(row, 0, Player.WHITE)
    assert board.check_win(4, 0) is True


def test_check_win_diagonal():
    board = Board()
    # 主对角线 (\)
    for i in range(5):
        board.place(i, i, Player.BLACK)
    assert board.check_win(4, 4) is True


def test_check_win_antidiagonal():
    board = Board()
    # 反对角线 (/)
    for i in range(5):
        board.place(i, 4 - i, Player.WHITE)
    assert board.check_win(4, 0) is True


def test_check_win_four_not_win():
    board = Board()
    for col in range(4):
        board.place(0, col, Player.BLACK)
    assert board.check_win(0, 3) is False


def test_check_win_empty_cell():
    board = Board()
    assert board.check_win(7, 7) is False


# ---------------------------------------------------------------------------
# get_candidate_moves
# ---------------------------------------------------------------------------


def test_candidate_moves_empty_board():
    board = Board()
    moves = board.get_candidate_moves()
    assert moves == [(BOARD_SIZE // 2, BOARD_SIZE // 2)]


def test_candidate_moves_includes_neighbors():
    board = Board()
    board.place(7, 7, Player.BLACK)
    moves = board.get_candidate_moves()

    # (7,7) 本身已占，不应出现
    assert (7, 7) not in moves

    # radius=2 时，周围 5x5 去掉中心后的 24 个点都应在候选中
    for di in range(-AI_CANDIDATE_RANGE, AI_CANDIDATE_RANGE + 1):
        for dj in range(-AI_CANDIDATE_RANGE, AI_CANDIDATE_RANGE + 1):
            if di == 0 and dj == 0:
                continue
            assert (7 + di, 7 + dj) in moves
    assert len(moves) == 24


def test_candidate_moves_no_duplicates():
    board = Board()
    board.place(7, 7, Player.BLACK)
    board.place(7, 8, Player.WHITE)
    moves = board.get_candidate_moves()
    assert len(moves) == len(set(moves))


def test_candidate_moves_native_matches_python_order():
    board = Board()
    moves = [
        (7, 7, Player.BLACK),
        (7, 8, Player.WHITE),
        (8, 8, Player.BLACK),
        (6, 7, Player.WHITE),
        (8, 7, Player.BLACK),
        (6, 8, Player.WHITE),
        (9, 7, Player.BLACK),
        (5, 8, Player.WHITE),
    ]
    for row, col, player in moves:
        board.place(row, col, player)

    assert board.get_candidate_moves() == board._get_candidate_moves_python()


def test_candidate_moves_native_helper_matches_python_order():
    if _candidate_moves_native is None:
        return

    board = Board()
    moves = [
        (7, 7, Player.BLACK),
        (7, 8, Player.WHITE),
        (8, 8, Player.BLACK),
        (6, 7, Player.WHITE),
        (8, 7, Player.BLACK),
        (6, 8, Player.WHITE),
        (9, 7, Player.BLACK),
        (5, 8, Player.WHITE),
    ]
    for row, col, player in moves:
        board.place(row, col, player)

    assert list(_candidate_moves_native(board.grid, AI_CANDIDATE_RANGE)) == (
        board._get_candidate_moves_python()
    )


# ---------------------------------------------------------------------------
# is_full & copy
# ---------------------------------------------------------------------------


def test_is_full_false_on_new_board():
    assert Board().is_full() is False


def test_is_full_true_when_all_placed():
    board = Board()
    player = Player.BLACK
    for r in range(BOARD_SIZE):
        for c in range(BOARD_SIZE):
            board.place(r, c, player)
            player = Player.WHITE if player == Player.BLACK else Player.BLACK
    assert board.is_full() is True


def test_copy_is_independent():
    board = Board()
    board.place(7, 7, Player.BLACK)
    clone = board.copy()

    # clone 与原棋盘状态相同
    assert clone.grid[7][7] == Player.BLACK
    assert clone.last_move == (7, 7)

    # 修改 clone 不影响原棋盘
    clone.place(0, 0, Player.WHITE)
    assert board.grid[0][0] == Player.NONE
