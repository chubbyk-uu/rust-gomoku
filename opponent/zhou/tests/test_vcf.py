"""Tests for standalone VCF backends and Swift-facing protocol."""

from pathlib import Path

import pytest

from gomoku.ai.vcf import (
    PythonVCFBackend,
    VCFQuery,
    _immediate_win_moves_native,
    _order_moves_by_hotness_native,
    _vcf_attack_moves_native,
    create_vcf_backend,
    init_zobrist,
)
from gomoku.board import Board
from gomoku.config import Player


def _vcf_white_winning_board() -> Board:
    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.WHITE)
    board.place(14, 14, Player.BLACK)
    return board


def _vcf_black_winning_board() -> Board:
    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.BLACK)
    board.place(14, 14, Player.WHITE)
    return board


def test_vcf_query_payload_is_swift_friendly():
    board = Board()
    board.place(7, 7, Player.BLACK)
    board.place(7, 8, Player.WHITE)

    query = VCFQuery.from_board(board, "find_win", Player.BLACK, max_depth=8)
    payload = query.to_payload()

    assert payload["apiVersion"] == 1
    assert payload["mode"] == "find_win"
    assert payload["attacker"] == int(Player.BLACK)
    assert payload["defender"] == int(Player.WHITE)
    assert payload["boardSize"] == 15
    assert len(payload["flatGrid"]) == 15 * 15
    assert payload["flatGrid"][7 * 15 + 7] == int(Player.BLACK)
    assert payload["flatGrid"][7 * 15 + 8] == int(Player.WHITE)


def test_python_vcf_backend_finds_winning_move_without_mutating_board():
    board = _vcf_white_winning_board()
    history_before = board.move_history.copy()

    backend = PythonVCFBackend(init_zobrist())
    move = backend.find_winning_move(board, Player.WHITE, max_depth=8)

    assert move == (7, 3)
    assert board.move_history == history_before


def test_python_vcf_backend_finds_blocking_move():
    board = _vcf_black_winning_board()

    backend = PythonVCFBackend(init_zobrist())
    move = backend.find_blocking_move(board, Player.WHITE, max_depth=8)

    assert move == (7, 3)


def test_find_immediate_wins_native_matches_python_and_order():
    board = Board()
    for col in (4, 5, 6, 8):
        board.place(7, col, Player.WHITE)
    board.place(6, 6, Player.BLACK)

    backend = PythonVCFBackend(init_zobrist())
    native = backend._find_immediate_wins(board, Player.WHITE)
    python = backend._find_immediate_wins_python(board, Player.WHITE)

    assert native == python
    assert native == [(7, 7)]


def test_immediate_win_native_helper_matches_python_backend():
    if _immediate_win_moves_native is None:
        pytest.skip("native eval kernels not built")

    board = Board()
    for col in (4, 5, 6, 8):
        board.place(7, col, Player.WHITE)
    for row, col in ((6, 6), (8, 6), (6, 8)):
        board.place(row, col, Player.BLACK)

    backend = PythonVCFBackend(init_zobrist())
    ordered = backend._order_moves(board, board.get_candidate_moves(), Player.WHITE, None)

    assert list(_immediate_win_moves_native(board.grid, ordered, Player.WHITE, None)) == (
        backend._find_immediate_wins_python(board, Player.WHITE, ordered_moves=ordered)
    )
    assert list(_immediate_win_moves_native(board.grid, ordered, Player.WHITE, 1)) == (
        backend._find_immediate_wins_python(board, Player.WHITE, 1, ordered_moves=ordered)
    )


def test_generate_vcf_attacks_native_matches_python_and_order():
    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.WHITE)
    board.place(14, 14, Player.BLACK)

    backend = PythonVCFBackend(init_zobrist())
    native = backend._generate_vcf_attacks(board, Player.WHITE)
    python = backend._generate_vcf_attacks_python(board, Player.WHITE)

    assert native == python
    assert native[0] == (7, 3)


def test_vcf_attack_native_helper_matches_python_backend():
    if _vcf_attack_moves_native is None:
        pytest.skip("native eval kernels not built")

    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.WHITE)
    board.place(14, 14, Player.BLACK)

    backend = PythonVCFBackend(init_zobrist())
    ordered = backend._order_moves(board, board.get_candidate_moves(), Player.WHITE, None)

    assert list(_vcf_attack_moves_native(board.grid, ordered, Player.WHITE, 12)) == (
        backend._generate_vcf_attacks_python(board, Player.WHITE, ordered_moves=ordered)
    )


def test_order_moves_native_helper_matches_python_backend():
    if _order_moves_by_hotness_native is None:
        pytest.skip("native eval kernels not built")

    board = Board()
    for row, col in [(7, 0), (7, 1), (7, 2), (5, 3), (6, 3), (8, 3)]:
        board.place(row, col, Player.WHITE)
    board.place(14, 14, Player.BLACK)

    backend = PythonVCFBackend(init_zobrist())
    moves = board.get_candidate_moves()

    assert list(
        _order_moves_by_hotness_native(board.grid, moves, Player.WHITE, Player.BLACK, 1.5, None)
    ) == backend._order_moves(board, moves, Player.WHITE, None)


def test_swift_backend_falls_back_to_python_when_binary_missing():
    board = _vcf_white_winning_board()

    backend = create_vcf_backend(
        init_zobrist(),
        backend="swift",
        swift_command="/definitely/missing/gomoku-vcf",
    )
    move = backend.find_winning_move(board, Player.WHITE, max_depth=8)

    assert move == (7, 3)


def test_auto_backend_uses_built_swift_binary_when_available():
    binary = Path(__file__).resolve().parents[1] / ".build" / "debug" / "gomoku-vcf"
    if not binary.exists():
        pytest.skip("swift binary not built")

    board = _vcf_white_winning_board()

    backend = create_vcf_backend(init_zobrist(), backend="auto")
    try:
        move = backend.find_winning_move(board, Player.WHITE, max_depth=8)
    finally:
        backend.close()

    assert move == (7, 3)


def test_backend_auto_prefers_release_binary(monkeypatch, tmp_path: Path):
    release = tmp_path / ".build" / "release" / "gomoku-vcf"
    debug = tmp_path / ".build" / "debug" / "gomoku-vcf"
    release.parent.mkdir(parents=True)
    debug.parent.mkdir(parents=True)
    release.write_text("")
    debug.write_text("")
    release.chmod(0o755)
    debug.chmod(0o755)

    import gomoku.ai.vcf as vcf_module

    fake_module = tmp_path / "src" / "gomoku" / "ai" / "vcf.py"
    fake_module.parent.mkdir(parents=True)
    fake_module.write_text("")

    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(vcf_module, "__file__", str(fake_module))
    monkeypatch.setattr(vcf_module, "AI_VCF_SWIFT_COMMAND", "")
    monkeypatch.setattr(vcf_module.shutil, "which", lambda _name: None)
    monkeypatch.delenv("GOMOKU_VCF_SWIFT_BIN", raising=False)

    backend = create_vcf_backend(init_zobrist(), backend="auto")
    try:
        assert backend._command == [str(release)]
    finally:
        backend.close()


def test_swift_server_reuses_single_process_for_multiple_queries():
    binary = Path(__file__).resolve().parents[1] / ".build" / "debug" / "gomoku-vcf"
    if not binary.exists():
        pytest.skip("swift binary not built")

    backend = create_vcf_backend(
        init_zobrist(),
        backend="swift",
        swift_command=str(binary),
    )
    try:
        board = _vcf_white_winning_board()
        first = backend.find_winning_move(board, Player.WHITE, max_depth=8)
        first_pid = backend._process.pid
        second = backend.find_winning_move(board, Player.WHITE, max_depth=8)
        second_pid = backend._process.pid
    finally:
        backend.close()

    assert first == (7, 3)
    assert second == (7, 3)
    assert first_pid == second_pid


def test_swift_server_keeps_requests_stateless():
    binary = Path(__file__).resolve().parents[1] / ".build" / "debug" / "gomoku-vcf"
    if not binary.exists():
        pytest.skip("swift binary not built")

    backend = create_vcf_backend(
        init_zobrist(),
        backend="swift",
        swift_command=str(binary),
    )
    try:
        miss = backend.find_winning_move(Board(), Player.WHITE, max_depth=8)
        hit = backend.find_winning_move(_vcf_white_winning_board(), Player.WHITE, max_depth=8)
    finally:
        backend.close()

    assert miss is None
    assert hit == (7, 3)


def test_swift_server_recovers_after_process_killed():
    binary = Path(__file__).resolve().parents[1] / ".build" / "debug" / "gomoku-vcf"
    if not binary.exists():
        pytest.skip("swift binary not built")

    backend = create_vcf_backend(
        init_zobrist(),
        backend="swift",
        swift_command=str(binary),
    )
    try:
        first = backend.find_winning_move(_vcf_white_winning_board(), Player.WHITE, max_depth=8)
        first_pid = backend._process.pid
        backend._process.kill()
        backend._process.wait(timeout=1)
        second = backend.find_winning_move(_vcf_white_winning_board(), Player.WHITE, max_depth=8)
        second_pid = backend._process.pid
    finally:
        backend.close()

    assert first == (7, 3)
    assert second == (7, 3)
    assert first_pid != second_pid
