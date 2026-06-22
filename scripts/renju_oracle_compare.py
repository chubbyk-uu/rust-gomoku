#!/usr/bin/env python3
"""Compare Renju forbidden-move fixtures against local and external oracles.

The harness validates fixture shape, prints stable summaries, and compares
fixture expectations against available external oracles. Local Rust detector
calls will be added after the pure forbidden detector exists.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Any

BOARD_SIZE = 15
VALID_EXPECTED = {"none", "double_three", "double_four", "overline"}
RENJU_FORBID_BY_CODE = {
    0: "none",
    1: "double_three",
    2: "double_four",
    3: "overline",
}
DEFAULT_RAPFI_BIN = Path(
    "/home/jerry/downloads/oracle_ws/rapfi/Rapfi/build/gcc-oracle/pbrain-rapfi"
)
DEFAULT_RENJU_FORBID_ROOT = Path("/home/jerry/downloads/oracle_ws/renju_forbid")
DEFAULT_KNOWN_MISMATCHES = (
    Path(__file__).resolve().parents[1] / "cases" / "renju" / "oracle_mismatches.jsonl"
)


@dataclass(frozen=True)
class Point:
    x: int
    y: int


@dataclass(frozen=True)
class Stone:
    x: int
    y: int
    side: int


@dataclass(frozen=True)
class Fixture:
    name: str
    board_size: int
    moves: list[Stone]
    candidate: Point
    expected: str
    expected_win: bool
    notes: str


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def in_bounds(point: Point, board_size: int) -> bool:
    return 0 <= point.x < board_size and 0 <= point.y < board_size


def parse_stone(raw: Any, *, case_name: str, index: int, board_size: int) -> Stone:
    if not isinstance(raw, dict):
        raise ValueError(f"{case_name}: move {index} must be an object")
    try:
        stone = Stone(x=int(raw["x"]), y=int(raw["y"]), side=int(raw["side"]))
    except KeyError as exc:
        raise ValueError(f"{case_name}: move {index} missing {exc.args[0]!r}") from exc
    if stone.side not in (-1, 1):
        raise ValueError(f"{case_name}: move {index} side must be 1 or -1")
    if not in_bounds(Point(stone.x, stone.y), board_size):
        raise ValueError(f"{case_name}: move {index} is out of bounds")
    return stone


def parse_point(raw: Any, *, case_name: str, board_size: int) -> Point:
    if not isinstance(raw, dict):
        raise ValueError(f"{case_name}: candidate must be an object")
    try:
        point = Point(x=int(raw["x"]), y=int(raw["y"]))
    except KeyError as exc:
        raise ValueError(f"{case_name}: candidate missing {exc.args[0]!r}") from exc
    if not in_bounds(point, board_size):
        raise ValueError(f"{case_name}: candidate is out of bounds")
    return point


def parse_fixture(raw: Any, line_number: int) -> Fixture:
    if not isinstance(raw, dict):
        raise ValueError(f"line {line_number}: fixture must be an object")
    name = str(raw.get("name") or f"line_{line_number}")
    board_size = int(raw.get("board_size", BOARD_SIZE))
    if board_size != BOARD_SIZE:
        raise ValueError(f"{name}: only 15x15 fixtures are currently supported")
    expected = str(raw.get("expected", ""))
    if expected not in VALID_EXPECTED:
        raise ValueError(f"{name}: expected must be one of {sorted(VALID_EXPECTED)}")
    moves = [
        parse_stone(item, case_name=name, index=index, board_size=board_size)
        for index, item in enumerate(raw.get("moves", []))
    ]
    candidate = parse_point(raw.get("candidate"), case_name=name, board_size=board_size)
    occupied = {(stone.x, stone.y) for stone in moves}
    if (candidate.x, candidate.y) in occupied:
        raise ValueError(f"{name}: candidate is occupied")
    if len(occupied) != len(moves):
        raise ValueError(f"{name}: duplicate stones in moves")
    return Fixture(
        name=name,
        board_size=board_size,
        moves=moves,
        candidate=candidate,
        expected=expected,
        expected_win=bool(raw.get("expected_win", False)),
        notes=str(raw.get("notes", "")),
    )


def load_fixtures(path: Path) -> list[Fixture]:
    fixtures: list[Fixture] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        fixtures.append(parse_fixture(json.loads(stripped), line_number))
    return fixtures


def load_known_kind_differences(path: Path) -> set[str]:
    if not path.exists():
        return set()

    out = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        raw = json.loads(stripped)
        if not isinstance(raw, dict):
            raise ValueError(f"{path}:{line_number}: known mismatch must be an object")
        name = str(raw.get("name") or "")
        kind = str(raw.get("kind") or "")
        if name and "classification convention difference" in kind:
            out.add(name)
    return out


def sgf_coord(point: Point) -> str:
    return f"{chr(ord('a') + point.x)}{chr(ord('a') + point.y)}"


def fixture_to_sgf(fixture: Fixture, *, include_candidate: bool = True) -> str:
    parts = ["(;GM[1]FF[4]SZ[15]"]
    for stone in fixture.moves:
        color = "B" if stone.side == 1 else "W"
        parts.append(f";{color}[{sgf_coord(Point(stone.x, stone.y))}]")
    if include_candidate:
        parts.append(f";B[{sgf_coord(fixture.candidate)}]")
    parts.append(")")
    return "".join(parts)


def fixture_to_setup_sgf(fixture: Fixture, *, include_candidate: bool = True) -> str:
    black = []
    white = []
    for stone in fixture.moves:
        coord = sgf_coord(Point(stone.x, stone.y))
        if stone.side == 1:
            black.append(coord)
        else:
            white.append(coord)

    parts = ["(;GM[1]FF[4]SZ[15]"]
    if black:
        parts.append("AB" + "".join(f"[{coord}]" for coord in black))
    if white:
        parts.append("AW" + "".join(f"[{coord}]" for coord in white))
    if include_candidate:
        parts.append(f";B[{sgf_coord(fixture.candidate)}]")
    parts.append(")")
    return "".join(parts)


def expected_forbidden(fixture: Fixture) -> bool:
    return fixture.expected != "none"


def resolve_rapfi_bin(cli_value: Path | None) -> Path | None:
    if cli_value:
        return cli_value
    env_value = os.environ.get("RAPFI_BIN")
    if env_value:
        return Path(env_value)
    if DEFAULT_RAPFI_BIN.exists():
        return DEFAULT_RAPFI_BIN
    found = shutil.which("rapfi")
    return Path(found) if found else None


def rapfi_available(path: Path | None) -> tuple[bool, str]:
    if path:
        return path.exists(), str(path)
    return False, f"RAPFI_BIN not set, default missing at {DEFAULT_RAPFI_BIN}, and rapfi not on PATH"


def go_available() -> tuple[bool, str]:
    found = shutil.which("go")
    return found is not None, found or "go not on PATH"


def resolve_renju_forbid_root(cli_value: Path | None) -> Path:
    if cli_value:
        return cli_value
    env_value = os.environ.get("RENJU_FORBID_ROOT")
    if env_value:
        return Path(env_value)
    return DEFAULT_RENJU_FORBID_ROOT


def renju_forbid_available(root: Path) -> tuple[bool, str]:
    go_ok, go_detail = go_available()
    if not go_ok:
        return False, go_detail
    go_mod = root / "go.mod"
    if not go_mod.exists():
        return False, f"go.mod not found at {go_mod}"
    return True, str(root)


def rapfi_board_lines(fixture: Fixture) -> list[str]:
    lines = []
    for stone in fixture.moves:
        color = 1 if stone.side == 1 else 2
        lines.append(f"{stone.x},{stone.y},{color}")

    # YXSHOWFORBID only prints forbidden points when Rapfi's side-to-move is
    # black. If the final explicit stone is black, append a white pass.
    if fixture.moves and fixture.moves[-1].side == 1:
        lines.append("-1,-1,2")
    return lines


def parse_rapfi_forbid_line(line: str) -> set[tuple[int, int]]:
    if not line.startswith("FORBID ") or not line.endswith("."):
        raise ValueError(f"unexpected Rapfi forbid line: {line!r}")
    payload = line[len("FORBID ") : -1].strip()
    if not payload:
        return set()
    if len(payload) % 4 != 0 or not payload.isdigit():
        raise ValueError(f"unexpected Rapfi forbid payload: {payload!r}")
    points = set()
    for index in range(0, len(payload), 4):
        points.add((int(payload[index : index + 2]), int(payload[index + 2 : index + 4])))
    return points


def rapfi_forbidden_points(
    fixture: Fixture, *, rapfi_bin: Path, timeout_seconds: float
) -> set[tuple[int, int]]:
    command_lines = ["INFO rule 4", f"START {fixture.board_size}", "YXBOARD"]
    command_lines.extend(rapfi_board_lines(fixture))
    command_lines.extend(["DONE", "YXSHOWFORBID", "END"])
    proc = subprocess.run(
        [str(rapfi_bin)],
        input="\n".join(command_lines) + "\n",
        text=True,
        capture_output=True,
        timeout=timeout_seconds,
        check=False,
    )
    if proc.returncode != 0:
        raise subprocess.CalledProcessError(
            proc.returncode, [str(rapfi_bin)], output=proc.stdout, stderr=proc.stderr
        )
    forbid_lines = [line.strip() for line in proc.stdout.splitlines() if line.startswith("FORBID ")]
    if len(forbid_lines) != 1:
        raise ValueError(
            f"{fixture.name}: expected one Rapfi FORBID line, got {len(forbid_lines)}; "
            f"stdout={proc.stdout!r} stderr={proc.stderr!r}"
        )
    return parse_rapfi_forbid_line(forbid_lines[0])


def compare_rapfi(
    fixtures: list[Fixture], *, rapfi_bin: Path, timeout_seconds: float, quiet: bool, jobs: int
) -> tuple[int, dict[str, bool]]:
    mismatches = 0
    forbidden_by_name = {}
    print("rapfi_compare=begin")

    def probe(fixture: Fixture) -> tuple[Fixture, set[tuple[int, int]]]:
        points = rapfi_forbidden_points(
            fixture, rapfi_bin=rapfi_bin, timeout_seconds=timeout_seconds
        )
        return fixture, points

    # Each fixture is an independent Rapfi subprocess, so a thread pool overlaps
    # the per-process startup/search latency. map() preserves fixture order, so
    # output stays deterministic regardless of completion order.
    if jobs > 1 and len(fixtures) > 1:
        with ThreadPoolExecutor(max_workers=jobs) as executor:
            results = list(executor.map(probe, fixtures))
    else:
        results = [probe(fixture) for fixture in fixtures]

    for fixture, points in results:
        actual = (fixture.candidate.x, fixture.candidate.y) in points
        forbidden_by_name[fixture.name] = actual
        expected = expected_forbidden(fixture)
        status = "ok" if actual == expected else "mismatch"
        if status == "mismatch":
            mismatches += 1
        actual_text = "forbidden" if actual else "none"
        expected_text = "forbidden" if expected else "none"
        if not quiet or status == "mismatch":
            print(
                f"rapfi {status} {fixture.name}: expected={expected_text} actual={actual_text} "
                f"forbid_count={len(points)}"
            )
    print(f"rapfi_compare=end mismatches={mismatches}")
    return mismatches, forbidden_by_name


def run_local_detector(case_file: Path, *, timeout_seconds: float) -> dict[str, str]:
    proc = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "--bin",
            "renju_rule_probe",
            "--",
            "--case-file",
            str(case_file),
        ],
        cwd=repo_root(),
        text=True,
        capture_output=True,
        timeout=timeout_seconds,
        check=False,
    )
    if proc.returncode != 0:
        raise subprocess.CalledProcessError(
            proc.returncode, ["cargo", "run", "--bin", "renju_rule_probe"], proc.stdout, proc.stderr
        )
    results = {}
    for line in proc.stdout.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        name, actual = stripped.split(maxsplit=1)
        results[name] = actual
    return results


def compare_local_detector(
    fixtures: list[Fixture], *, case_file: Path, timeout_seconds: float, quiet: bool
) -> int:
    mismatches = 0
    print("local_detector_compare=begin")
    actual_by_name = run_local_detector(case_file, timeout_seconds=timeout_seconds)
    for fixture in fixtures:
        actual = actual_by_name.get(fixture.name)
        if actual is None:
            raise ValueError(f"local detector did not return fixture {fixture.name}")
        status = "ok" if actual == fixture.expected else "mismatch"
        if status == "mismatch":
            mismatches += 1
        if not quiet or status == "mismatch":
            print(
                f"local_detector {status} {fixture.name}: "
                f"expected={fixture.expected} actual={actual}"
            )
    print(f"local_detector_compare=end mismatches={mismatches}")
    return mismatches


def renju_forbid_go_source() -> str:
    return r'''package main

import (
	"bufio"
	"fmt"
	"os"

	ren "github.com/realjustice/renju_forbid"
)

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		fmt.Println(ren.CheckForbid(scanner.Text()))
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
'''


def run_renju_forbid(
    fixtures: list[Fixture], *, root: Path, timeout_seconds: float
) -> list[str]:
    with tempfile.TemporaryDirectory(prefix="renju_forbid_cli_") as tmpdir:
        main_go = Path(tmpdir) / "main.go"
        main_go.write_text(renju_forbid_go_source(), encoding="utf-8")
        input_text = "\n".join(fixture_to_setup_sgf(fixture) for fixture in fixtures) + "\n"
        proc = subprocess.run(
            ["go", "run", str(main_go)],
            input=input_text,
            text=True,
            capture_output=True,
            timeout=timeout_seconds,
            cwd=root,
            check=False,
        )
    if proc.returncode != 0:
        raise subprocess.CalledProcessError(
            proc.returncode, ["go", "run", "renju_forbid_cli"], proc.stdout, proc.stderr
        )
    lines = [line.strip() for line in proc.stdout.splitlines() if line.strip()]
    if len(lines) != len(fixtures):
        raise ValueError(
            f"renju_forbid returned {len(lines)} lines for {len(fixtures)} fixtures; "
            f"stdout={proc.stdout!r} stderr={proc.stderr!r}"
        )
    results = []
    for fixture, line in zip(fixtures, lines):
        try:
            code = int(line)
        except ValueError as exc:
            raise ValueError(f"{fixture.name}: invalid renju_forbid result {line!r}") from exc
        if code not in RENJU_FORBID_BY_CODE:
            raise ValueError(f"{fixture.name}: unknown renju_forbid result code {code}")
        results.append(RENJU_FORBID_BY_CODE[code])
    return results


def compare_renju_forbid(
    fixtures: list[Fixture],
    *,
    root: Path,
    timeout_seconds: float,
    quiet: bool,
    known_kind_differences: set[str],
    rapfi_forbidden_by_name: dict[str, bool] | None,
) -> int:
    mismatches = 0
    accepted_known = 0
    accepted_coexisting_overline_double_three = 0
    print("renju_forbid_compare=begin")
    actual_results = run_renju_forbid(fixtures, root=root, timeout_seconds=timeout_seconds)
    for fixture, actual in zip(fixtures, actual_results):
        status = "ok" if actual == fixture.expected else "mismatch"
        rapfi_forbidden = (
            None if rapfi_forbidden_by_name is None else rapfi_forbidden_by_name.get(fixture.name)
        )
        if (
            status == "mismatch"
            and fixture.expected == "overline"
            and actual == "double_three"
            and rapfi_forbidden is True
        ):
            status = "accepted_coexisting_overline_double_three"
            accepted_coexisting_overline_double_three += 1
        if (
            status == "mismatch"
            and fixture.name in known_kind_differences
            and expected_forbidden(fixture)
            and actual != "none"
        ):
            status = "accepted_known_kind_difference"
            accepted_known += 1
        if status == "mismatch":
            mismatches += 1
        if not quiet or status == "mismatch":
            print(f"renju_forbid {status} {fixture.name}: expected={fixture.expected} actual={actual}")
    print(
        "renju_forbid_compare=end "
        f"mismatches={mismatches} "
        f"accepted_coexisting_overline_double_three={accepted_coexisting_overline_double_three} "
        f"accepted_known_kind_differences={accepted_known}"
    )
    return mismatches


def print_summary(fixtures: list[Fixture], *, show_sgf: bool, quiet: bool) -> None:
    print(f"fixtures={len(fixtures)}")
    if quiet and not show_sgf:
        counts = {expected: 0 for expected in sorted(VALID_EXPECTED)}
        for fixture in fixtures:
            counts[fixture.expected] += 1
        print(
            "expected_counts="
            + ",".join(f"{expected}:{counts[expected]}" for expected in sorted(counts))
        )
        return
    for fixture in fixtures:
        win_suffix = " expected_win=true" if fixture.expected_win else ""
        print(
            f"{fixture.name}: expected={fixture.expected}{win_suffix} "
            f"stones={len(fixture.moves)} candidate=({fixture.candidate.x},{fixture.candidate.y})"
        )
        if show_sgf:
            print(f"  sgf={fixture_to_sgf(fixture)}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case-file",
        type=Path,
        default=repo_root() / "cases" / "renju" / "forbidden_hand_cases.jsonl",
    )
    parser.add_argument("--show-sgf", action="store_true")
    parser.add_argument("--quiet", action="store_true", help="only print summaries and mismatches")
    parser.add_argument("--skip-local", action="store_true")
    parser.add_argument("--local-timeout", type=float, default=30.0)
    parser.add_argument(
        "--rapfi-bin",
        type=Path,
        default=None,
        help="path to pbrain-rapfi; defaults to RAPFI_BIN, the local oracle_ws build, or rapfi on PATH",
    )
    parser.add_argument("--skip-rapfi", action="store_true")
    parser.add_argument(
        "--jobs",
        type=int,
        default=min(8, os.cpu_count() or 1),
        help="parallel Rapfi subprocesses (default: min(8, cpu count))",
    )
    parser.add_argument(
        "--renju-forbid-root",
        type=Path,
        default=None,
        help="path to local github.com/realjustice/renju_forbid checkout",
    )
    parser.add_argument("--skip-renju-forbid", action="store_true")
    parser.add_argument("--oracle-timeout", type=float, default=10.0)
    parser.add_argument(
        "--require-oracles",
        action="store_true",
        help="fail if Rapfi or Go/renju_forbid prerequisites are not configured",
    )
    parser.add_argument(
        "--known-mismatches",
        type=Path,
        default=DEFAULT_KNOWN_MISMATCHES,
        help="JSONL file for accepted oracle reporting-convention differences",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    fixtures = load_fixtures(args.case_file)
    print_summary(fixtures, show_sgf=args.show_sgf, quiet=args.quiet)

    rapfi_bin = resolve_rapfi_bin(args.rapfi_bin)
    renju_forbid_root = resolve_renju_forbid_root(args.renju_forbid_root)
    known_kind_differences = load_known_kind_differences(args.known_mismatches)
    rapfi_ok, rapfi_detail = rapfi_available(rapfi_bin)
    renju_forbid_ok, renju_forbid_detail = renju_forbid_available(renju_forbid_root)
    print(f"rapfi_available={rapfi_ok} detail={rapfi_detail}")
    print(f"renju_forbid_available={renju_forbid_ok} detail={renju_forbid_detail}")
    print(f"known_kind_differences={len(known_kind_differences)} detail={args.known_mismatches}")

    if args.require_oracles and not (rapfi_ok and renju_forbid_ok):
        return 2
    mismatches = 0
    rapfi_forbidden_by_name = None
    if not args.skip_local:
        mismatches += compare_local_detector(
            fixtures, case_file=args.case_file, timeout_seconds=args.local_timeout, quiet=args.quiet
        )
    else:
        print("local_detector_compare=skipped")
    if rapfi_ok and not args.skip_rapfi:
        assert rapfi_bin is not None
        rapfi_mismatches, rapfi_forbidden_by_name = compare_rapfi(
            fixtures, rapfi_bin=rapfi_bin, timeout_seconds=args.oracle_timeout,
            quiet=args.quiet, jobs=args.jobs,
        )
        mismatches += rapfi_mismatches
    else:
        print("rapfi_compare=skipped")
    if renju_forbid_ok and not args.skip_renju_forbid:
        mismatches += compare_renju_forbid(
            fixtures,
            root=renju_forbid_root,
            timeout_seconds=args.oracle_timeout,
            quiet=args.quiet,
            known_kind_differences=known_kind_differences,
            rapfi_forbidden_by_name=rapfi_forbidden_by_name,
        )
    else:
        print("renju_forbid_compare=skipped")
    if mismatches:
        return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as exc:
        print(f"error: command failed with exit code {exc.returncode}: {exc.cmd}", file=sys.stderr)
        if exc.stdout:
            print(f"stdout:\n{exc.stdout}", file=sys.stderr)
        if exc.stderr:
            print(f"stderr:\n{exc.stderr}", file=sys.stderr)
        raise SystemExit(1)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
