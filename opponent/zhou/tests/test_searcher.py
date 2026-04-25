"""Tests for AISearcher."""

from gomoku.ai.evaluator import DEFENSE_WEIGHT, evaluate_local
from gomoku.ai.searcher import _order_search_moves_native
from gomoku.ai.searcher import _order_moves_by_hotness_native
from gomoku.ai.searcher import AISearcher
from gomoku.board import Board
from gomoku.config import AI_MAX_CANDIDATES, Player


def _make_searcher(ai_player: Player = Player.WHITE, depth: int = 2) -> AISearcher:
    return AISearcher(depth=depth, ai_player=ai_player)


# ---------------------------------------------------------------------------
# test_ai_blocks_four
# ---------------------------------------------------------------------------


def test_ai_blocks_four():
    """对手形成冲四（一端被边界封堵）时，AI 应该堵住唯一的开放端。"""
    board = Board()
    # 人类(BLACK)在第0行连了4子: (0,0),(0,1),(0,2),(0,3)
    # 左端 col=-1 越界（天然封堵），右端 (0,4) 是唯一开口，AI 必须堵这里
    for col in range(4):
        board.place(0, col, Player.BLACK)
    searcher = _make_searcher(ai_player=Player.WHITE, depth=2)
    move = searcher.find_best_move(board)
    assert move is not None
    assert move == (0, 4), f"Expected blocking move (0,4), got {move}"


# ---------------------------------------------------------------------------
# test_ai_wins_when_possible
# ---------------------------------------------------------------------------


def test_ai_wins_when_possible():
    """AI 自身已有四连时，应该直接补全五连获胜。"""
    board = Board()
    # AI(WHITE) 在第5行已有4子: (5,0),(5,1),(5,2),(5,3)，右端 (5,4) 为空
    for col in range(4):
        board.place(5, col, Player.WHITE)
    # 人类随便放一子，避免棋盘过于空旷影响候选点生成
    board.place(0, 14, Player.BLACK)

    searcher = _make_searcher(ai_player=Player.WHITE, depth=2)
    move = searcher.find_best_move(board)
    assert move is not None
    # AI 应该选择 (5,4) 形成五连
    assert move == (5, 4), f"Expected winning move (5,4), got {move}"


# ---------------------------------------------------------------------------
# test_find_best_move_returns_valid_position
# ---------------------------------------------------------------------------


def test_find_best_move_on_empty_board():
    """空棋盘时 AI 应该落子在天元（中心）。"""
    board = Board()
    searcher = _make_searcher(ai_player=Player.WHITE, depth=1)
    move = searcher.find_best_move(board)
    assert move == (7, 7)


def test_white_first_move_avoids_diagonal_contact_from_center_opening():
    """白棋首手不应再默认贴对角，需保留更多稳定后续。"""
    board = Board()
    board.place(7, 7, Player.BLACK)

    searcher = _make_searcher(ai_player=Player.WHITE, depth=5)
    move = searcher.find_best_move(board)
    assert move is not None
    assert not (abs(move[0] - 7) == 1 and abs(move[1] - 7) == 1), (
        f"Expected non-diagonal contact reply, got {move}"
    )


def test_white_first_move_avoids_diagonal_contact_from_corner_opening():
    """角部固定开局下，白棋首手也应避免贴对角。"""
    board = Board()
    board.place(4, 4, Player.BLACK)

    searcher = _make_searcher(ai_player=Player.WHITE, depth=5)
    move = searcher.find_best_move(board)
    assert move is not None
    assert not (abs(move[0] - 4) == 1 and abs(move[1] - 4) == 1), (
        f"Expected non-diagonal contact reply, got {move}"
    )


def test_find_best_move_does_not_modify_board():
    """find_best_move 不应改变传入棋盘的状态。"""
    board = Board()
    board.place(7, 7, Player.BLACK)
    history_before = board.move_history.copy()

    searcher = _make_searcher()
    searcher.find_best_move(board)

    assert board.move_history == history_before
    assert board.grid[7][7] == Player.BLACK


def test_ai_as_black_wins_when_possible():
    """AI 执黑时，有五连机会应该直接赢。"""
    board = Board()
    # AI(BLACK) 已有4子: (3,0)~(3,3)，右端 (3,4) 为空
    for col in range(4):
        board.place(3, col, Player.BLACK)
    board.place(0, 14, Player.WHITE)  # 对手随机一子

    searcher = AISearcher(depth=2, ai_player=Player.BLACK)
    move = searcher.find_best_move(board)
    assert move == (3, 4), f"Expected (3,4), got {move}"


def test_depth_one_ai_blocks_broken_four():
    """深度 1 也应识别并堵住对手的跳四中点。"""
    board = Board()
    for col in (4, 5, 7, 8):
        board.place(7, col, Player.BLACK)
    board.place(0, 0, Player.WHITE)

    searcher = AISearcher(depth=1, ai_player=Player.WHITE)
    move = searcher.find_best_move(board)
    assert move == (7, 6), f"Expected blocking move (7,6), got {move}"


def test_depth_one_ai_finds_vcf_winning_setup():
    """深度 1 时也应优先走出可形成双重胜点的 VCF 先手。"""
    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.WHITE)
    board.place(14, 14, Player.BLACK)

    searcher = AISearcher(depth=1, ai_player=Player.WHITE)
    move = searcher.find_best_move(board)
    assert move == (7, 3), f"Expected VCF setup move (7,3), got {move}"


def test_depth_one_ai_blocks_opponent_vcf_setup():
    """对手存在一步进入 VCF 的先手时，AI 应提前占住关键点。"""
    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.BLACK)
    board.place(14, 14, Player.WHITE)

    searcher = AISearcher(depth=1, ai_player=Player.WHITE)
    move = searcher.find_best_move(board)
    assert move == (7, 3), f"Expected preventive block (7,3), got {move}"


def test_search_order_native_matches_python_hotness_order():
    if _order_moves_by_hotness_native is None:
        return

    board = Board()
    for row, col, player in [
        (7, 7, Player.BLACK),
        (7, 8, Player.WHITE),
        (8, 8, Player.BLACK),
        (6, 7, Player.WHITE),
        (8, 7, Player.BLACK),
        (6, 8, Player.WHITE),
        (9, 7, Player.BLACK),
        (5, 8, Player.WHITE),
    ]:
        board.place(row, col, player)

    searcher = AISearcher(depth=2, ai_player=Player.WHITE)
    moves = board.get_candidate_moves()
    current_player = Player.WHITE
    opponent = Player.BLACK

    expected = []
    for r, c in moves:
        attack_score = evaluate_local(board.grid, r, c, current_player)
        defend_score = evaluate_local(board.grid, r, c, opponent)
        expected.append((r, c, attack_score + defend_score * DEFENSE_WEIGHT))
    expected.sort(key=lambda item: item[2], reverse=True)

    assert list(
        _order_moves_by_hotness_native(board.grid, moves, current_player, opponent, 1.5, None)
    ) == [(r, c) for r, c, _ in expected]


def test_search_order_search_native_matches_python_search_order():
    if _order_search_moves_native is None:
        return

    board = Board()
    for row, col, player in [
        (7, 7, Player.BLACK),
        (7, 8, Player.WHITE),
        (8, 8, Player.BLACK),
        (6, 7, Player.WHITE),
        (8, 7, Player.BLACK),
        (6, 8, Player.WHITE),
        (9, 7, Player.BLACK),
        (5, 8, Player.WHITE),
    ]:
        board.place(row, col, player)

    searcher = AISearcher(depth=2, ai_player=Player.WHITE)
    moves = board.get_candidate_moves()
    tt_move = moves[3]
    killers = [moves[1], moves[5]]

    expected_searcher = AISearcher(depth=2, ai_player=Player.WHITE)
    expected_searcher._killers[2] = killers.copy()
    expected = expected_searcher._order_moves(board, moves, Player.WHITE, 2, tt_move)

    assert list(
        _order_search_moves_native(
            board.grid,
            moves,
            Player.WHITE,
            Player.BLACK,
            tt_move,
            killers,
            1.5,
            AI_MAX_CANDIDATES,
        )
    ) == expected
