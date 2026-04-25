#!/usr/bin/env python3
"""Run fixed-opening Gomocup matches between Rust and Python reference engines."""

from __future__ import annotations

import argparse
import json
import os
import select
import shlex
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from typing import Any

BOARD_SIZE = 15
BLACK = 1
WHITE = -1
DEFAULT_MAX_MOVES = BOARD_SIZE * BOARD_SIZE
DEFAULT_REF_ROOT = Path.home() / "python_ws" / "pygomoku"
DEFAULT_MOVE_TIMEOUT_SEC = 120.0
DEFAULT_GAME_TIMEOUT_SEC = 900.0

FIXED_OPENINGS_5: list[tuple[int, int]] = [
    (7, 7),
    (4, 4),
    (4, 10),
    (10, 4),
    (10, 10),
]

FIXED_OPENINGS_9: list[tuple[int, int]] = [
    (2, 2),
    (2, 12),
    (12, 2),
    (12, 12),
    (4, 4),
    (10, 4),
    (4, 10),
    (10, 10),
    (7, 7),
]

OPENING_SETS = {
    "5": FIXED_OPENINGS_5,
    "9": FIXED_OPENINGS_9,
}


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def opposite(side: int) -> int:
    return -side


def side_name(side: int) -> str:
    return "black" if side == BLACK else "white"


def in_bounds(x: int, y: int) -> bool:
    return 0 <= x < BOARD_SIZE and 0 <= y < BOARD_SIZE


def winner_after(moves: list[dict[str, Any]], x: int, y: int, side: int) -> bool:
    occupied = {(move["x"], move["y"]): move["side"] for move in moves}
    for dx, dy in [(1, 0), (0, 1), (1, 1), (1, -1)]:
        count = 1
        for sign in [1, -1]:
            cx = x + dx * sign
            cy = y + dy * sign
            while in_bounds(cx, cy) and occupied.get((cx, cy)) == side:
                count += 1
                cx += dx * sign
                cy += dy * sign
        if count >= 5:
            return True
    return False


class GomocupEngine:
    def __init__(
        self,
        *,
        name: str,
        command: str,
        cwd: Path,
        pythonpath: Path | None,
        info: list[tuple[str, str]],
    ) -> None:
        self.name = name
        self.command = command
        self.cwd = cwd
        self.pythonpath = pythonpath
        self.info = info
        self.proc: subprocess.Popen[str] | None = None

    def start(self) -> None:
        if self.proc is not None and self.proc.poll() is None:
            return
        env = dict(os.environ)
        if self.pythonpath is not None:
            current = env.get("PYTHONPATH")
            env["PYTHONPATH"] = (
                str(self.pythonpath) if not current else f"{self.pythonpath}:{current}"
            )
        argv = shlex.split(self.command)
        if not argv:
            raise RuntimeError(f"{self.name}: empty command")
        self.proc = subprocess.Popen(
            argv,
            cwd=self.cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self.send("START 15")
        response = self.read()
        if response != "OK":
            raise RuntimeError(f"{self.name}: START failed: {response}")
        self.configure()

    def configure(self) -> None:
        for key, value in self.info:
            self.send(f"INFO {key} {value}")

    def send(self, line: str) -> None:
        self.start()
        assert self.proc is not None and self.proc.stdin is not None
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def kill(self) -> None:
        proc = self.proc
        if proc is not None and proc.poll() is None:
            proc.kill()
        self.proc = None

    def read(self, timeout_sec: float | None = None) -> str:
        self.start()
        assert self.proc is not None and self.proc.stdout is not None
        while True:
            if timeout_sec is not None and timeout_sec > 0:
                readable, _, _ = select.select([self.proc.stdout], [], [], timeout_sec)
                if not readable:
                    self.kill()
                    raise TimeoutError(f"{self.name}: no response within {timeout_sec:.1f}s")
            line = self.proc.stdout.readline()
            if not line:
                stderr = ""
                if self.proc.stderr is not None:
                    stderr = self.proc.stderr.read()
                raise RuntimeError(f"{self.name}: process exited: {stderr.strip()}")
            text = line.strip()
            if not text or text.upper().startswith("MESSAGE"):
                continue
            return text

    def restart(self) -> None:
        self.send("RESTART")
        response = self.read()
        if response != "OK":
            raise RuntimeError(f"{self.name}: RESTART failed: {response}")
        self.configure()

    def move(
        self,
        moves: list[dict[str, Any]],
        side_to_move: int,
        move_timeout_sec: float,
    ) -> tuple[tuple[int, int], float]:
        self.restart()
        self.send("BOARD")
        for move in moves:
            # Gomocup BOARD uses side=1 for the engine side, side=2 for opponent.
            rel_side = 1 if move["side"] == side_to_move else 2
            self.send(f"{move['x']},{move['y']},{rel_side}")
        start = time.perf_counter()
        self.send("DONE")
        response = self.read(None if move_timeout_sec <= 0 else move_timeout_sec)
        elapsed_ms = (time.perf_counter() - start) * 1000.0
        if "," not in response:
            raise RuntimeError(f"{self.name}: unexpected move response: {response}")
        x_raw, y_raw = response.split(",", 1)
        return (int(x_raw), int(y_raw)), elapsed_ms

    def close(self) -> None:
        proc = self.proc
        if proc is None:
            return
        if proc.poll() is None:
            try:
                assert proc.stdin is not None
                proc.stdin.write("END\n")
                proc.stdin.flush()
            except Exception:
                pass
            try:
                proc.wait(timeout=1)
            except subprocess.TimeoutExpired:
                proc.kill()
        self.proc = None


@dataclass(frozen=True)
class MatchTask:
    opening_index: int
    opening: tuple[int, int]
    rust_side: int


def build_engine(
    *,
    name: str,
    command: str,
    ref_root: Path,
    info: list[tuple[str, str]],
) -> GomocupEngine:
    is_reference = name == "reference"
    return GomocupEngine(
        name=name,
        command=command,
        cwd=ref_root if is_reference else repo_root(),
        pythonpath=ref_root if is_reference else None,
        info=info,
    )


def run_match_task(
    task: MatchTask,
    *,
    rust_command: str,
    reference_command: str,
    ref_root: Path,
    rust_info: list[tuple[str, str]],
    reference_info: list[tuple[str, str]],
    max_moves: int,
    move_timeout_sec: float,
    game_timeout_sec: float,
) -> dict[str, Any]:
    game_started = time.perf_counter()
    engines = {
        "rust": build_engine(
            name="rust",
            command=rust_command,
            ref_root=ref_root,
            info=rust_info,
        ),
        "reference": build_engine(
            name="reference",
            command=reference_command,
            ref_root=ref_root,
            info=reference_info,
        ),
    }
    side_to_engine = {
        task.rust_side: "rust",
        opposite(task.rust_side): "reference",
    }
    moves: list[dict[str, Any]] = []
    occupied: set[tuple[int, int]] = set()
    times_ms = {"rust": [], "reference": []}
    winner_engine: str | None = None
    winner_side: int | None = None
    error: str | None = None

    try:
        x, y = task.opening
        opening_engine = side_to_engine[BLACK]
        moves.append(
            {
                "ply": 1,
                "x": x,
                "y": y,
                "side": BLACK,
                "engine": opening_engine,
                "opening": True,
                "elapsed_ms": 0.0,
            }
        )
        occupied.add((x, y))
        side = WHITE

        while len(moves) < max_moves:
            engine_name = side_to_engine[side]
            if game_timeout_sec > 0 and time.perf_counter() - game_started > game_timeout_sec:
                error = f"game timeout after {game_timeout_sec:.1f}s"
                winner_engine = "timeout"
                break
            try:
                (mx, my), elapsed_ms = engines[engine_name].move(
                    moves,
                    side,
                    move_timeout_sec,
                )
            except TimeoutError as exc:
                error = str(exc)
                winner_engine = side_to_engine[opposite(side)]
                winner_side = opposite(side)
                break
            if not in_bounds(mx, my):
                error = f"{engine_name} returned out-of-bounds move {(mx, my)}"
                winner_engine = side_to_engine[opposite(side)]
                winner_side = opposite(side)
                break
            if (mx, my) in occupied:
                error = f"{engine_name} returned occupied move {(mx, my)}"
                winner_engine = side_to_engine[opposite(side)]
                winner_side = opposite(side)
                break

            move = {
                "ply": len(moves) + 1,
                "x": mx,
                "y": my,
                "side": side,
                "engine": engine_name,
                "opening": False,
                "elapsed_ms": round(elapsed_ms, 3),
            }
            moves.append(move)
            occupied.add((mx, my))
            times_ms[engine_name].append(elapsed_ms)

            if winner_after(moves, mx, my, side):
                winner_engine = engine_name
                winner_side = side
                break
            side = opposite(side)

        if winner_engine is None and len(moves) >= max_moves:
            winner_engine = "draw"
        elapsed_s = time.perf_counter() - game_started
    finally:
        for engine in engines.values():
            engine.close()

    return {
        "opening_index": task.opening_index,
        "opening": list(task.opening),
        "rust_side": side_name(task.rust_side),
        "reference_side": side_name(opposite(task.rust_side)),
        "winner_engine": winner_engine,
        "winner_side": None if winner_side is None else side_name(winner_side),
        "error": error,
        "plies": len(moves),
        "elapsed_s": round(elapsed_s, 3),
        "avg_ms": {
            name: round(sum(values) / len(values), 3) if values else 0.0
            for name, values in times_ms.items()
        },
        "max_ms": {
            name: round(max(values), 3) if values else 0.0 for name, values in times_ms.items()
        },
        "moves": moves,
    }


def parse_info(values: list[str]) -> list[tuple[str, str]]:
    result: list[tuple[str, str]] = []
    for raw in values:
        if "=" not in raw:
            raise ValueError(f"INFO override must be KEY=VALUE: {raw}")
        key, value = raw.split("=", 1)
        result.append((key, value))
    return result


def summarize(games: list[dict[str, Any]]) -> dict[str, Any]:
    wins = {"rust": 0, "reference": 0, "draw": 0, "timeout": 0}
    for game in games:
        winner = game["winner_engine"]
        if winner in wins:
            wins[winner] += 1
    all_times = {"rust": [], "reference": []}
    for game in games:
        for move in game["moves"]:
            if not move.get("opening"):
                all_times[move["engine"]].append(float(move["elapsed_ms"]))
    return {
        "wins": wins,
        "errors": sum(1 for game in games if game.get("error")),
        "avg_ms": {
            name: round(sum(values) / len(values), 3) if values else 0.0
            for name, values in all_times.items()
        },
        "max_ms": {
            name: round(max(values), 3) if values else 0.0
            for name, values in all_times.items()
        },
    }


def print_progress(done: int, total: int, game: dict[str, Any]) -> None:
    print(
        "[{done}/{total}] opening={idx} {opening} rust={rust_side} "
        "winner={winner} plies={plies} elapsed={elapsed:.1f}s error={error}".format(
            done=done,
            total=total,
            idx=game["opening_index"],
            opening=game["opening"],
            rust_side=game["rust_side"],
            winner=game["winner_engine"],
            plies=game["plies"],
            elapsed=float(game.get("elapsed_s", 0.0)),
            error=game.get("error") or "-",
        ),
        flush=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--opening-set", choices=sorted(OPENING_SETS), default="9")
    parser.add_argument("--limit-openings", type=int)
    parser.add_argument("--jobs", type=int, default=18)
    parser.add_argument("--max-moves", type=int, default=DEFAULT_MAX_MOVES)
    parser.add_argument(
        "--move-timeout-sec",
        type=float,
        default=DEFAULT_MOVE_TIMEOUT_SEC,
        help="per-move engine response timeout; 0 disables it",
    )
    parser.add_argument(
        "--game-timeout-sec",
        type=float,
        default=DEFAULT_GAME_TIMEOUT_SEC,
        help="per-game wall-clock timeout; 0 disables it",
    )
    parser.add_argument("--output", type=Path, default=Path("/tmp/rust_vs_reference_9_openings.json"))
    parser.add_argument("--ref-root", type=Path, default=DEFAULT_REF_ROOT)
    parser.add_argument(
        "--rust-command",
        default=str(repo_root() / "target" / "release" / "gomocup_engine"),
    )
    parser.add_argument(
        "--reference-command",
        default=f"{shlex.quote(sys.executable)} -m pygomoku.gomocup_engine --depth 6 --width 20",
    )
    parser.add_argument(
        "--rust-info",
        action="append",
        default=[],
        help="extra Rust INFO override, e.g. root_vct_depth=8",
    )
    parser.add_argument(
        "--reference-info",
        action="append",
        default=["root_vct_depth=4"],
        help="extra reference INFO override, e.g. root_vct_depth=4",
    )
    args = parser.parse_args()

    ref_root = args.ref_root.expanduser().resolve()
    if not (ref_root / "pygomoku").is_dir():
        raise SystemExit(f"reference root not found: {ref_root}")
    if not Path(shlex.split(args.rust_command)[0]).exists() and args.rust_command.startswith("/"):
        raise SystemExit(f"Rust engine binary not found: {args.rust_command}")

    rust_info = parse_info(args.rust_info)
    reference_info = parse_info(args.reference_info)
    openings = OPENING_SETS[args.opening_set]
    if args.limit_openings is not None:
        openings = openings[: max(0, args.limit_openings)]
    tasks = [
        MatchTask(index, opening, rust_side)
        for index, opening in enumerate(openings)
        for rust_side in [BLACK, WHITE]
    ]

    started = time.perf_counter()
    games: list[dict[str, Any]] = []
    jobs = max(1, args.jobs)
    if jobs == 1:
        for task in tasks:
            games.append(
                run_match_task(
                    task,
                    rust_command=args.rust_command,
                    reference_command=args.reference_command,
                    ref_root=ref_root,
                    rust_info=rust_info,
                    reference_info=reference_info,
                    max_moves=args.max_moves,
                    move_timeout_sec=args.move_timeout_sec,
                    game_timeout_sec=args.game_timeout_sec,
                )
            )
            print_progress(len(games), len(tasks), games[-1])
    else:
        with ProcessPoolExecutor(max_workers=jobs) as pool:
            futures = {
                pool.submit(
                    run_match_task,
                    task,
                    rust_command=args.rust_command,
                    reference_command=args.reference_command,
                    ref_root=ref_root,
                    rust_info=rust_info,
                    reference_info=reference_info,
                    max_moves=args.max_moves,
                    move_timeout_sec=args.move_timeout_sec,
                    game_timeout_sec=args.game_timeout_sec,
                ): index
                for index, task in enumerate(tasks)
            }
            indexed: list[tuple[int, dict[str, Any]]] = []
            done_count = 0
            for future in as_completed(futures):
                result = future.result()
                indexed.append((futures[future], result))
                done_count += 1
                print_progress(done_count, len(tasks), result)
            games = [game for _, game in sorted(indexed, key=lambda item: item[0])]

    result = {
        "matchup": "rust_vs_reference",
        "settings": {
            "opening_set": args.opening_set,
            "jobs": jobs,
            "max_moves": args.max_moves,
            "move_timeout_sec": args.move_timeout_sec,
            "game_timeout_sec": args.game_timeout_sec,
            "rust_command": args.rust_command,
            "rust_info": rust_info,
            "reference_command": args.reference_command,
            "reference_info": reference_info,
            "ref_root": str(ref_root),
        },
        "summary": summarize(games),
        "games": games,
        "elapsed_s": round(time.perf_counter() - started, 3),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")

    print(json.dumps({"summary": result["summary"], "elapsed_s": result["elapsed_s"]}, indent=2))
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
