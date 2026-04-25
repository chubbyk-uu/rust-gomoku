"""VCF backends for Gomoku.

This module keeps the current Python VCF solver as the default backend and
defines a JSON-based interface for a future Swift implementation.
"""

from __future__ import annotations

import json
import os
import select
import shlex
import shutil
import subprocess
import threading
from dataclasses import dataclass
from typing import Optional, Protocol

from gomoku.ai.evaluator import DEFENSE_WEIGHT, evaluate_local
from gomoku.board import Board
from gomoku.config import (
    AI_VCF_BACKEND,
    AI_VCF_CANDIDATES,
    AI_VCF_SWIFT_COMMAND,
    AI_VCF_SWIFT_TIMEOUT_MS,
    BOARD_SIZE,
    Player,
)

Move = tuple[int, int]
ZobristTable = list[list[dict[int, int]]]

try:
    from gomoku.ai._eval_kernels import immediate_win_moves_native as _immediate_win_moves_native
    from gomoku.ai._eval_kernels import order_moves_by_hotness_native as _order_moves_by_hotness_native
    from gomoku.ai._eval_kernels import vcf_attack_moves_native as _vcf_attack_moves_native
except ImportError:  # pragma: no cover - optional native acceleration
    _immediate_win_moves_native = None
    _order_moves_by_hotness_native = None
    _vcf_attack_moves_native = None


@dataclass(frozen=True)
class VCFQuery:
    """Serializable VCF request shared by Python and Swift backends."""

    api_version: int
    mode: str
    attacker: int
    defender: int
    max_depth: int
    max_candidates: int
    board_size: int
    flat_grid: tuple[int, ...]

    @classmethod
    def from_board(
        cls,
        board: Board,
        mode: str,
        attacker: Player,
        max_depth: int,
        max_candidates: int = AI_VCF_CANDIDATES,
    ) -> "VCFQuery":
        defender = _opponent_of(attacker)
        grid = tuple(tuple(int(cell) for cell in row) for row in board.grid)
        return cls(
            api_version=1,
            mode=mode,
            attacker=int(attacker),
            defender=int(defender),
            max_depth=max_depth,
            max_candidates=max_candidates,
            board_size=BOARD_SIZE,
            flat_grid=tuple(cell for row in grid for cell in row),
        )

    def to_payload(self) -> dict[str, object]:
        """Convert request to a Swift-friendly JSON payload."""
        return {
            "apiVersion": self.api_version,
            "mode": self.mode,
            "attacker": self.attacker,
            "defender": self.defender,
            "maxDepth": self.max_depth,
            "maxCandidates": self.max_candidates,
            "boardSize": self.board_size,
            "flatGrid": list(self.flat_grid),
        }


@dataclass(frozen=True)
class VCFResult:
    """Backend-agnostic VCF result contract."""

    found: bool
    move: Optional[Move]
    backend: str
    depth_reached: int = 0
    nodes: int = 0
    error: Optional[str] = None

    @classmethod
    def from_payload(cls, payload: object, backend: str) -> "VCFResult":
        if not isinstance(payload, dict):
            raise ValueError("VCF backend response must be a JSON object.")

        move = payload.get("move")
        if move is None:
            return cls(
                found=bool(payload.get("found", False)),
                move=None,
                backend=backend,
                depth_reached=int(payload.get("depthReached", 0)),
                nodes=int(payload.get("nodes", 0)),
                error=payload.get("error"),
            )
        if not isinstance(move, list | tuple) or len(move) != 2:
            raise ValueError("VCF backend move must be a [row, col] pair.")
        row, col = move
        if not isinstance(row, int) or not isinstance(col, int):
            raise ValueError("VCF backend move coordinates must be integers.")
        return cls(
            found=True,
            move=(row, col),
            backend=backend,
            depth_reached=int(payload.get("depthReached", 0)),
            nodes=int(payload.get("nodes", 0)),
            error=payload.get("error"),
        )


class VCFBackend(Protocol):
    """Minimal backend interface for VCF solvers."""

    def reset(self) -> None:
        """Reset per-search backend state."""

    def close(self) -> None:
        """Release backend resources."""

    def find_winning_move(
        self,
        board: Board,
        attacker: Player,
        max_depth: int,
        *,
        current_hash: Optional[int] = None,
    ) -> Optional[Move]:
        """Return the first winning VCF move if any."""

    def find_blocking_move(
        self,
        board: Board,
        defender: Player,
        max_depth: int,
        *,
        current_hash: Optional[int] = None,
    ) -> Optional[Move]:
        """Return one move that breaks the opponent's VCF if any."""


class PythonVCFBackend:
    """Native Python VCF backend with Zobrist-based recursion cache."""

    def __init__(self, zobrist: Optional[ZobristTable] = None) -> None:
        self._zobrist = zobrist if zobrist is not None else init_zobrist()
        self._hash = 0
        self._tt: dict[tuple[int, int, int], bool] = {}
        self._candidate_moves_cache: dict[int, tuple[Move, ...]] = {}
        self._ordered_moves_cache: dict[tuple[int, int, Optional[int]], tuple[Move, ...]] = {}
        self._vcf_attacks_cache: dict[tuple[int, int], tuple[Move, ...]] = {}
        self._immediate_wins_cache: dict[tuple[int, int], tuple[Move, ...]] = {}

    def reset(self) -> None:
        self._tt.clear()
        self._candidate_moves_cache.clear()
        self._ordered_moves_cache.clear()
        self._vcf_attacks_cache.clear()
        self._immediate_wins_cache.clear()
        self._hash = 0

    def close(self) -> None:
        """Python backend keeps no external resources."""

    def find_winning_move(
        self,
        board: Board,
        attacker: Player,
        max_depth: int,
        *,
        current_hash: Optional[int] = None,
    ) -> Optional[Move]:
        self._hash = self._compute_hash(board) if current_hash is None else current_hash
        return self._find_vcf_move(board, attacker, max_depth)

    def find_blocking_move(
        self,
        board: Board,
        defender: Player,
        max_depth: int,
        *,
        current_hash: Optional[int] = None,
    ) -> Optional[Move]:
        self._hash = self._compute_hash(board) if current_hash is None else current_hash
        return self._find_blocking_move_against_vcf(board, defender, max_depth)

    def _compute_hash(self, board: Board) -> int:
        value = 0
        for row in range(BOARD_SIZE):
            for col in range(BOARD_SIZE):
                player = board.grid[row][col]
                if player != Player.NONE:
                    value ^= self._zobrist[row][col][player]
        return value

    def _candidate_moves(self, board: Board) -> list[Move]:
        cached = self._candidate_moves_cache.get(self._hash)
        if cached is not None:
            return list(cached)

        moves = tuple(board.get_candidate_moves())
        self._candidate_moves_cache[self._hash] = moves
        return list(moves)

    def _ordered_moves(
        self,
        board: Board,
        current_player: Player,
        max_candidates: Optional[int],
    ) -> list[Move]:
        key = (self._hash, int(current_player), max_candidates)
        cached = self._ordered_moves_cache.get(key)
        if cached is not None:
            return list(cached)

        ordered = tuple(
            self._order_moves(
                board,
                self._candidate_moves(board),
                current_player,
                max_candidates,
            )
        )
        self._ordered_moves_cache[key] = ordered
        return list(ordered)

    def _order_moves(
        self,
        board: Board,
        moves: list[Move],
        current_player: Player,
        max_candidates: Optional[int],
    ) -> list[Move]:
        if _order_moves_by_hotness_native is not None:
            return list(
                _order_moves_by_hotness_native(
                    board.grid,
                    moves,
                    current_player,
                    _opponent_of(current_player),
                    DEFENSE_WEIGHT,
                    max_candidates,
                )
            )

        scored: list[tuple[int, int, float]] = []
        grid = board.grid
        opponent = _opponent_of(current_player)

        for row, col in moves:
            attack_score = evaluate_local(grid, row, col, current_player)
            defend_score = evaluate_local(grid, row, col, opponent)
            hotness = attack_score + defend_score * DEFENSE_WEIGHT
            scored.append((row, col, hotness))

        scored.sort(key=lambda item: item[2], reverse=True)
        ordered = [(row, col) for row, col, _ in scored]
        if max_candidates is None:
            return ordered
        return ordered[:max_candidates]

    def _find_vcf_move(
        self,
        board: Board,
        attacker: Player,
        depth: int,
    ) -> Optional[Move]:
        if depth <= 0:
            return None

        for move in self._generate_vcf_attacks(board, attacker):
            if self._vcf_move_wins(board, attacker, move, depth):
                return move
        return None

    def _find_blocking_move_against_vcf(
        self,
        board: Board,
        defender: Player,
        depth: int,
    ) -> Optional[Move]:
        if depth <= 0:
            return None

        attacker = _opponent_of(defender)
        if not self._has_vcf(board, attacker, depth):
            return None

        moves = self._ordered_moves(board, defender, max_candidates=None)
        for row, col in moves:
            board.place(row, col, defender)
            self._hash ^= self._zobrist[row][col][defender]

            if board.check_win(row, col):
                self._hash ^= self._zobrist[row][col][defender]
                board.undo()
                return (row, col)

            if not self._has_vcf(board, attacker, max(depth - 1, 0)):
                self._hash ^= self._zobrist[row][col][defender]
                board.undo()
                return (row, col)

            self._hash ^= self._zobrist[row][col][defender]
            board.undo()

        return None

    def _has_vcf(self, board: Board, attacker: Player, depth: int) -> bool:
        if depth <= 0:
            return False

        key = (self._hash, int(attacker), depth)
        cached = self._tt.get(key)
        if cached is not None:
            return cached

        result = False
        for move in self._generate_vcf_attacks(board, attacker):
            if self._vcf_move_wins(board, attacker, move, depth):
                result = True
                break

        self._tt[key] = result
        return result

    def _vcf_move_wins(
        self,
        board: Board,
        attacker: Player,
        move: Move,
        depth: int,
    ) -> bool:
        if depth <= 0:
            return False

        defender = _opponent_of(attacker)
        row, col = move

        board.place(row, col, attacker)
        self._hash ^= self._zobrist[row][col][attacker]

        if board.check_win(row, col):
            self._hash ^= self._zobrist[row][col][attacker]
            board.undo()
            return True

        if self._find_immediate_wins(board, defender, limit=1):
            self._hash ^= self._zobrist[row][col][attacker]
            board.undo()
            return False

        defenses = self._find_immediate_wins(board, attacker)
        if not defenses:
            self._hash ^= self._zobrist[row][col][attacker]
            board.undo()
            return False

        for defend_row, defend_col in defenses:
            board.place(defend_row, defend_col, defender)
            self._hash ^= self._zobrist[defend_row][defend_col][defender]

            if board.check_win(defend_row, defend_col):
                self._hash ^= self._zobrist[defend_row][defend_col][defender]
                board.undo()
                self._hash ^= self._zobrist[row][col][attacker]
                board.undo()
                return False

            if not self._has_vcf(board, attacker, depth - 1):
                self._hash ^= self._zobrist[defend_row][defend_col][defender]
                board.undo()
                self._hash ^= self._zobrist[row][col][attacker]
                board.undo()
                return False

            self._hash ^= self._zobrist[defend_row][defend_col][defender]
            board.undo()

        self._hash ^= self._zobrist[row][col][attacker]
        board.undo()
        return True

    def _generate_vcf_attacks(self, board: Board, attacker: Player) -> list[Move]:
        key = (self._hash, int(attacker))
        cached = self._vcf_attacks_cache.get(key)
        if cached is not None:
            return list(cached)

        moves = self._ordered_moves(board, attacker, max_candidates=None)
        if _vcf_attack_moves_native is not None:
            forcing = tuple(_vcf_attack_moves_native(board.grid, moves, attacker, AI_VCF_CANDIDATES))
            self._vcf_attacks_cache[key] = forcing
            return list(forcing)

        forcing = tuple(self._generate_vcf_attacks_python(board, attacker, ordered_moves=moves))
        self._vcf_attacks_cache[key] = forcing
        return list(forcing)

    def _generate_vcf_attacks_python(
        self,
        board: Board,
        attacker: Player,
        *,
        ordered_moves: Optional[list[Move]] = None,
    ) -> list[Move]:
        moves = (
            ordered_moves
            if ordered_moves is not None
            else self._ordered_moves(board, attacker, max_candidates=None)
        )
        forcing: list[Move] = []
        for row, col in moves:
            board.place(row, col, attacker)
            self._hash ^= self._zobrist[row][col][attacker]

            if board.check_win(row, col) or self._find_immediate_wins(board, attacker, limit=1):
                forcing.append((row, col))

            self._hash ^= self._zobrist[row][col][attacker]
            board.undo()

            if len(forcing) >= AI_VCF_CANDIDATES:
                break

        return forcing

    def _find_immediate_wins(
        self,
        board: Board,
        player: Player,
        limit: Optional[int] = None,
    ) -> list[Move]:
        key = (self._hash, int(player))
        cached = self._immediate_wins_cache.get(key)
        if cached is None:
            moves = self._ordered_moves(board, player, max_candidates=None)
            if _immediate_win_moves_native is not None:
                cached = tuple(_immediate_win_moves_native(board.grid, moves, player, None))
            else:
                cached = tuple(
                    self._find_immediate_wins_python(board, player, None, ordered_moves=moves)
                )
            self._immediate_wins_cache[key] = cached

        if limit is None:
            return list(cached)
        return list(cached[:limit])

    def _find_immediate_wins_python(
        self,
        board: Board,
        player: Player,
        limit: Optional[int] = None,
        *,
        ordered_moves: Optional[list[Move]] = None,
    ) -> list[Move]:
        wins: list[Move] = []
        moves = (
            ordered_moves
            if ordered_moves is not None
            else self._ordered_moves(board, player, max_candidates=None)
        )

        for row, col in moves:
            board.place(row, col, player)
            self._hash ^= self._zobrist[row][col][player]

            if board.check_win(row, col):
                wins.append((row, col))
                if limit is not None and len(wins) >= limit:
                    self._hash ^= self._zobrist[row][col][player]
                    board.undo()
                    break

            self._hash ^= self._zobrist[row][col][player]
            board.undo()

        return wins


class SwiftVCFBackend:
    """Swift backend adapter using JSON stdin/stdout with Python fallback.

    Expected Swift binary contract:
    - stdin: one JSON payload matching ``VCFQuery.to_payload()``
    - stdout: one JSON object like ``{"move": [row, col]}`` or ``{"move": null}``
    """

    def __init__(
        self,
        fallback: VCFBackend,
        command: Optional[str] = None,
        timeout_ms: int = AI_VCF_SWIFT_TIMEOUT_MS,
    ) -> None:
        self._fallback = fallback
        self._timeout_ms = timeout_ms
        self._command = self._resolve_command(command)
        self._process: Optional[subprocess.Popen[str]] = None
        self._lock = threading.Lock()

    def reset(self) -> None:
        self._fallback.reset()

    def close(self) -> None:
        with self._lock:
            self._stop_process_locked()
        self._fallback.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def find_winning_move(
        self,
        board: Board,
        attacker: Player,
        max_depth: int,
        *,
        current_hash: Optional[int] = None,
    ) -> Optional[Move]:
        query = VCFQuery.from_board(board, "find_win", attacker, max_depth)
        result = self._run_query(query)
        if result is not None:
            return result.move
        return self._fallback.find_winning_move(
            board,
            attacker,
            max_depth,
            current_hash=current_hash,
        )

    def find_blocking_move(
        self,
        board: Board,
        defender: Player,
        max_depth: int,
        *,
        current_hash: Optional[int] = None,
    ) -> Optional[Move]:
        attacker = _opponent_of(defender)
        query = VCFQuery.from_board(board, "find_block", attacker, max_depth)
        result = self._run_query(query)
        if result is not None:
            return result.move
        return self._fallback.find_blocking_move(
            board,
            defender,
            max_depth,
            current_hash=current_hash,
        )

    def _resolve_command(self, command: Optional[str]) -> Optional[list[str]]:
        raw = command or os.getenv("GOMOKU_VCF_SWIFT_BIN") or AI_VCF_SWIFT_COMMAND
        if raw:
            return shlex.split(raw)

        repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
        for candidate in (
            os.path.join(repo_root, ".build", "release", "gomoku-vcf"),
            os.path.join(repo_root, ".build", "debug", "gomoku-vcf"),
            os.path.join(os.getcwd(), ".build", "release", "gomoku-vcf"),
            os.path.join(os.getcwd(), ".build", "debug", "gomoku-vcf"),
        ):
            if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
                return [candidate]

        auto = shutil.which("gomoku-vcf")
        if auto:
            return [auto]
        return None

    def _run_query(self, query: VCFQuery) -> Optional[VCFResult]:
        if self._command is None:
            return None

        try:
            with self._lock:
                process, started_new = self._ensure_process_locked()
                if process is None or process.stdin is None or process.stdout is None:
                    return None

                process.stdin.write(json.dumps(query.to_payload(), ensure_ascii=True))
                process.stdin.write("\n")
                process.stdin.flush()

                timeout_s = self._timeout_ms / 1000
                if started_new:
                    timeout_s = max(timeout_s, 1.0)
                line = self._read_response_line(process, timeout_s)
        except (BrokenPipeError, OSError, TimeoutError, subprocess.SubprocessError):
            with self._lock:
                self._stop_process_locked()
            return None

        try:
            payload = json.loads(line)
            result = VCFResult.from_payload(payload, backend="swift")
            if result.error:
                return None
            return result
        except (json.JSONDecodeError, ValueError):
            with self._lock:
                self._stop_process_locked()
            return None

    def _ensure_process_locked(self) -> tuple[Optional[subprocess.Popen[str]], bool]:
        if self._process is not None and self._process.poll() is None:
            return self._process, False

        self._stop_process_locked()
        if self._command is None:
            return None, False

        command = self._command.copy()
        if "--server" not in command:
            command.append("--server")

        try:
            process = subprocess.Popen(
                command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                bufsize=1,
            )
        except OSError:
            return None, False

        if process.stdin is None or process.stdout is None:
            process.kill()
            return None, False

        self._process = process
        return process, True

    def _read_response_line(self, process: subprocess.Popen[str], timeout_s: float) -> str:
        assert process.stdout is not None
        ready, _, _ = select.select([process.stdout.fileno()], [], [], timeout_s)
        if not ready:
            raise TimeoutError("swift vcf server timed out")

        line = process.stdout.readline()
        if line == "":
            raise OSError("swift vcf server closed stdout")
        return line

    def _stop_process_locked(self) -> None:
        process = self._process
        self._process = None
        if process is None:
            return

        if process.stdin is not None:
            process.stdin.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=max(self._timeout_ms / 1000, 0.2))
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=1)
        if process.stdout is not None:
            process.stdout.close()


def create_vcf_backend(
    zobrist: Optional[ZobristTable] = None,
    *,
    backend: Optional[str] = None,
    swift_command: Optional[str] = None,
) -> VCFBackend:
    """Create the configured VCF backend.

    ``backend`` supports:
    - ``python``: always use the in-process Python solver
    - ``swift``: try Swift first, fallback to Python if unavailable
    - ``auto``: same as swift for now, reserved for future heuristics
    """

    python_backend = PythonVCFBackend(zobrist=zobrist)
    mode = (backend or AI_VCF_BACKEND).strip().lower()

    if mode == "python":
        return python_backend
    if mode in {"swift", "auto"}:
        return SwiftVCFBackend(
            fallback=python_backend,
            command=swift_command,
        )
    return python_backend


def init_zobrist() -> ZobristTable:
    """Initialize the shared Zobrist table."""
    import random

    rng = random.Random(42)
    table: ZobristTable = []
    for row in range(BOARD_SIZE):
        zobrist_row = []
        for col in range(BOARD_SIZE):
            cell = {
                Player.BLACK: rng.getrandbits(64),
                Player.WHITE: rng.getrandbits(64),
            }
            zobrist_row.append(cell)
        table.append(zobrist_row)
    return table


def _opponent_of(player: Player) -> Player:
    return Player.WHITE if player == Player.BLACK else Player.BLACK
