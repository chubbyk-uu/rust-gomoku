#!/usr/bin/env python3
"""Run fixed-opening matches between two Gomocup engines."""

from __future__ import annotations

import argparse
import json
import os
import select
import shlex
import shutil
import subprocess
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from statistics import median
from typing import Any

BOARD_SIZE = 15
BLACK = 1
WHITE = -1
DEFAULT_MAX_MOVES = BOARD_SIZE * BOARD_SIZE
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

RESERVED_WINNERS = {"draw", "timeout"}


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def opposite(side: int) -> int:
    return -side


def side_name(side: int) -> str:
    return "black" if side == BLACK else "white"


def parse_side_name(value: str) -> int | None:
    if value == "black":
        return BLACK
    if value == "white":
        return WHITE
    return None


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


def parse_info(values: list[str]) -> list[tuple[str, str]]:
    result: list[tuple[str, str]] = []
    for raw in values:
        if "=" not in raw:
            raise ValueError(f"INFO override must be KEY=VALUE: {raw}")
        key, value = raw.split("=", 1)
        result.append((key, value))
    return result


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * pct)))
    return ordered[index]


def check_command(command: str, cwd: Path, label: str) -> None:
    argv = shlex.split(command)
    if not argv:
        raise SystemExit(f"{label}: empty command")
    executable = Path(argv[0])
    if executable.is_absolute():
        exists = executable.exists()
    elif "/" in argv[0]:
        exists = (cwd / executable).exists()
    else:
        exists = shutil.which(argv[0]) is not None
    if not exists:
        raise SystemExit(f"{label}: executable not found: {argv[0]}")


class GomocupEngine:
    def __init__(
        self,
        *,
        name: str,
        command: str,
        cwd: Path,
        info: list[tuple[str, str]],
    ) -> None:
        self.name = name
        self.command = command
        self.cwd = cwd
        self.info = info
        self.proc: subprocess.Popen[str] | None = None

    def start(self) -> None:
        if self.proc is not None and self.proc.poll() is None:
            return
        argv = shlex.split(self.command)
        if not argv:
            raise RuntimeError(f"{self.name}: empty command")
        self.proc = subprocess.Popen(
            argv,
            cwd=self.cwd,
            env=dict(os.environ),
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

    def kill(self) -> None:
        proc = self.proc
        if proc is not None and proc.poll() is None:
            proc.kill()
        self.proc = None

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
class EngineSpec:
    name: str
    command: str
    cwd: Path
    info: list[tuple[str, str]]


@dataclass(frozen=True)
class MatchTask:
    opening_index: int
    opening: tuple[int, int]
    engine_a_side: int


def run_match_task(
    task: MatchTask,
    *,
    engine_a: EngineSpec,
    engine_b: EngineSpec,
    max_moves: int,
    move_timeout_sec: float,
    game_timeout_sec: float,
) -> dict[str, Any]:
    game_started = time.perf_counter()
    engines = {
        engine_a.name: GomocupEngine(
            name=engine_a.name,
            command=engine_a.command,
            cwd=engine_a.cwd,
            info=engine_a.info,
        ),
        engine_b.name: GomocupEngine(
            name=engine_b.name,
            command=engine_b.command,
            cwd=engine_b.cwd,
            info=engine_b.info,
        ),
    }
    side_to_engine = {
        task.engine_a_side: engine_a.name,
        opposite(task.engine_a_side): engine_b.name,
    }
    moves: list[dict[str, Any]] = []
    occupied: set[tuple[int, int]] = set()
    times_ms = {engine_a.name: [], engine_b.name: []}
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
        "engine_a_side": side_name(task.engine_a_side),
        "engine_b_side": side_name(opposite(task.engine_a_side)),
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


def summarize(games: list[dict[str, Any]], engine_names: tuple[str, str]) -> dict[str, Any]:
    wins = {engine_names[0]: 0, engine_names[1]: 0, "draw": 0, "timeout": 0}
    for game in games:
        winner = game["winner_engine"]
        if winner in wins:
            wins[winner] += 1
    all_times = {engine_names[0]: [], engine_names[1]: []}
    for game in games:
        for move in game["moves"]:
            if not move.get("opening"):
                all_times[move["engine"]].append(float(move["elapsed_ms"]))
    timing = {}
    for name, values in all_times.items():
        timing[name] = {
            "avg_ms": round(sum(values) / len(values), 3) if values else 0.0,
            "median_ms": round(median(values), 3) if values else 0.0,
            "p95_ms": round(percentile(values, 0.95), 3) if values else 0.0,
            "max_ms": round(max(values), 3) if values else 0.0,
            "moves": len(values),
        }
    return {
        "wins": wins,
        "errors": sum(1 for game in games if game.get("error")),
        "timing": timing,
    }


def print_progress(done: int, total: int, game: dict[str, Any]) -> None:
    print(
        "[{done}/{total}] opening={idx} {opening} a_side={a_side} "
        "winner={winner} plies={plies} elapsed={elapsed:.1f}s error={error}".format(
            done=done,
            total=total,
            idx=game["opening_index"],
            opening=game["opening"],
            a_side=game["engine_a_side"],
            winner=game["winner_engine"],
            plies=game["plies"],
            elapsed=float(game.get("elapsed_s", 0.0)),
            error=game.get("error") or "-",
        ),
        flush=True,
    )


def parse_args() -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--opening-set", choices=sorted(OPENING_SETS), default="9")
    parser.add_argument("--opening-index", type=int)
    parser.add_argument(
        "--engine-a-side",
        choices=["black", "white", "both"],
        default="both",
        help="filter which side engine A plays",
    )
    parser.add_argument("--limit-openings", type=int)
    parser.add_argument("--jobs", type=int, default=18)
    parser.add_argument("--max-moves", type=int, default=DEFAULT_MAX_MOVES)
    parser.add_argument("--move-timeout-sec", type=float, default=DEFAULT_MOVE_TIMEOUT_SEC)
    parser.add_argument("--game-timeout-sec", type=float, default=DEFAULT_GAME_TIMEOUT_SEC)
    parser.add_argument("--output", type=Path, default=Path("/tmp/gomocup_match.json"))
    parser.add_argument("--engine-a-name", default="base")
    parser.add_argument("--engine-b-name", default="fast")
    parser.add_argument(
        "--engine-a-command",
        default=str(root / "target" / "release" / "gomocup_engine") + " --profile base",
    )
    parser.add_argument(
        "--engine-b-command",
        default=str(root / "target" / "release" / "gomocup_engine") + " --profile fast",
    )
    parser.add_argument("--engine-a-cwd", type=Path, default=root)
    parser.add_argument("--engine-b-cwd", type=Path, default=root)
    parser.add_argument("--engine-a-info", action="append", default=[])
    parser.add_argument("--engine-b-info", action="append", default=[])
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.engine_a_name == args.engine_b_name:
        raise SystemExit("engine names must be distinct")
    if args.engine_a_name in RESERVED_WINNERS or args.engine_b_name in RESERVED_WINNERS:
        raise SystemExit(f"engine names cannot be one of: {sorted(RESERVED_WINNERS)}")

    engine_a = EngineSpec(
        name=args.engine_a_name,
        command=args.engine_a_command,
        cwd=args.engine_a_cwd.expanduser().resolve(),
        info=parse_info(args.engine_a_info),
    )
    engine_b = EngineSpec(
        name=args.engine_b_name,
        command=args.engine_b_command,
        cwd=args.engine_b_cwd.expanduser().resolve(),
        info=parse_info(args.engine_b_info),
    )
    check_command(engine_a.command, engine_a.cwd, engine_a.name)
    check_command(engine_b.command, engine_b.cwd, engine_b.name)

    openings = OPENING_SETS[args.opening_set]
    indexed_openings = list(enumerate(openings))
    if args.opening_index is not None:
        indexed_openings = [
            (index, opening)
            for index, opening in indexed_openings
            if index == args.opening_index
        ]
        if not indexed_openings:
            raise SystemExit(
                f"opening index {args.opening_index} not found in set {args.opening_set}"
            )
    if args.limit_openings is not None:
        indexed_openings = indexed_openings[: max(0, args.limit_openings)]

    engine_a_sides = [BLACK, WHITE]
    if args.engine_a_side != "both":
        parsed_side = parse_side_name(args.engine_a_side)
        assert parsed_side is not None
        engine_a_sides = [parsed_side]
    tasks = [
        MatchTask(index, opening, engine_a_side)
        for index, opening in indexed_openings
        for engine_a_side in engine_a_sides
    ]

    started = time.perf_counter()
    games: list[dict[str, Any]] = []
    jobs = max(1, args.jobs)
    if jobs == 1:
        for task in tasks:
            games.append(
                run_match_task(
                    task,
                    engine_a=engine_a,
                    engine_b=engine_b,
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
                    engine_a=engine_a,
                    engine_b=engine_b,
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
        "matchup": f"{engine_a.name}_vs_{engine_b.name}",
        "settings": {
            "opening_set": args.opening_set,
            "jobs": jobs,
            "max_moves": args.max_moves,
            "move_timeout_sec": args.move_timeout_sec,
            "game_timeout_sec": args.game_timeout_sec,
            "engine_a": {
                "name": engine_a.name,
                "command": engine_a.command,
                "cwd": str(engine_a.cwd),
                "info": engine_a.info,
            },
            "engine_b": {
                "name": engine_b.name,
                "command": engine_b.command,
                "cwd": str(engine_b.cwd),
                "info": engine_b.info,
            },
        },
        "summary": summarize(games, (engine_a.name, engine_b.name)),
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
