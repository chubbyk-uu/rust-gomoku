#!/usr/bin/env python3
"""Benchmark base and fast on identical match-case positions."""

from __future__ import annotations

import argparse
import json
import shlex
import subprocess
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path
from statistics import median
from typing import Any


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def load_cases(path: Path, tags: list[str], limit: int | None) -> list[dict[str, Any]]:
    text = path.read_text(encoding="utf-8")
    stripped = text.lstrip()
    if not stripped:
        return []
    if stripped.startswith("["):
        cases = json.loads(text)
    else:
        cases = [json.loads(line) for line in text.splitlines() if line.strip()]
    if tags:
        required = set(tags)
        cases = [case for case in cases if required.issubset(set(case.get("tags", [])))]
    if limit is not None:
        cases = cases[: max(0, limit)]
    return cases


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * pct)))
    return ordered[index]


def probe_command(
    *,
    binary: str,
    case: dict[str, Any],
    profile: str,
    depth: int | None,
    width: int | None,
    tt_bits: int | None,
    root_profile: bool,
    extra_args: list[str],
) -> list[str]:
    cmd = [
        binary,
        "--case-json",
        json.dumps(case, separators=(",", ":")),
        "--profile",
        profile,
    ]
    if depth is not None:
        cmd += ["--depth", str(depth)]
    if width is not None:
        cmd += ["--width", str(width)]
    if tt_bits is not None:
        cmd += ["--tt-bits", str(tt_bits)]
    if root_profile:
        cmd.append("--root-profile")
    cmd.extend(extra_args)
    return cmd


def run_probe(
    *,
    binary: str,
    case: dict[str, Any],
    profile: str,
    depth: int | None,
    width: int | None,
    tt_bits: int | None,
    root_profile: bool,
    extra_args: list[str],
) -> dict[str, Any]:
    result = subprocess.run(
        probe_command(
            binary=binary,
            case=case,
            profile=profile,
            depth=depth,
            width=width,
            tt_bits=tt_bits,
            root_profile=root_profile,
            extra_args=extra_args,
        ),
        cwd=repo_root(),
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def run_one_case(
    index: int,
    case: dict[str, Any],
    *,
    binary: str,
    base_profile: str,
    fast_profile: str,
    base_tt_bits: int | None,
    fast_tt_bits: int | None,
    depth: int | None,
    width: int | None,
    root_profile: bool,
    base_extra_args: list[str],
    fast_extra_args: list[str],
) -> dict[str, Any]:
    base = run_probe(
        binary=binary,
        case=case,
        profile=base_profile,
        depth=depth,
        width=width,
        tt_bits=base_tt_bits,
        root_profile=root_profile,
        extra_args=base_extra_args,
    )
    fast = run_probe(
        binary=binary,
        case=case,
        profile=fast_profile,
        depth=depth,
        width=width,
        tt_bits=fast_tt_bits,
        root_profile=root_profile,
        extra_args=fast_extra_args,
    )
    base_result = base["result"]
    fast_result = fast["result"]
    base_time = float(base_result["elapsed_ms"])
    fast_time = float(fast_result["elapsed_ms"])
    base_nodes = int(base_result["nodes"])
    fast_nodes = int(fast_result["nodes"])
    return {
        "index": index,
        "case_name": base["case_name"],
        "tags": base.get("tags", []),
        "prefix_plies": base["prefix_plies"],
        "side_to_move": base["side_to_move"],
        "base": base_result,
        "fast": fast_result,
        "base_trace": base.get("trace"),
        "fast_trace": fast.get("trace"),
        "changed_move": base_result["move"] != fast_result["move"],
        "changed_score": base_result["score"] != fast_result["score"],
        "changed_depth": base_result["depth"] != fast_result["depth"],
        "node_ratio": None if base_nodes == 0 else fast_nodes / base_nodes,
        "time_ratio": None if base_time == 0.0 else fast_time / base_time,
        "delta_ms": fast_time - base_time,
        "delta_nodes": fast_nodes - base_nodes,
    }


def stats(values: list[float]) -> dict[str, float]:
    if not values:
        return {"avg": 0.0, "median": 0.0, "p95": 0.0, "max": 0.0}
    return {
        "avg": round(sum(values) / len(values), 3),
        "median": round(median(values), 3),
        "p95": round(percentile(values, 0.95), 3),
        "max": round(max(values), 3),
    }


def summarize(results: list[dict[str, Any]]) -> dict[str, Any]:
    base_times = [float(item["base"]["elapsed_ms"]) for item in results]
    fast_times = [float(item["fast"]["elapsed_ms"]) for item in results]
    base_nodes = [float(item["base"]["nodes"]) for item in results]
    fast_nodes = [float(item["fast"]["nodes"]) for item in results]
    time_ratios = [float(item["time_ratio"]) for item in results if item["time_ratio"] is not None]
    node_ratios = [float(item["node_ratio"]) for item in results if item["node_ratio"] is not None]
    return {
        "cases": len(results),
        "changed_moves": sum(1 for item in results if item["changed_move"]),
        "changed_scores": sum(1 for item in results if item["changed_score"]),
        "fast_faster_cases": sum(1 for item in results if item["delta_ms"] < 0.0),
        "fast_slower_cases": sum(1 for item in results if item["delta_ms"] > 0.0),
        "base_time_ms": stats(base_times),
        "fast_time_ms": stats(fast_times),
        "base_nodes": stats(base_nodes),
        "fast_nodes": stats(fast_nodes),
        "time_ratio": stats(time_ratios),
        "node_ratio": stats(node_ratios),
    }


def print_progress(done: int, total: int, result: dict[str, Any]) -> None:
    print(
        "[{done}/{total}] {name} changed_move={changed} "
        "base={base_ms:.1f}ms fast={fast_ms:.1f}ms ratio={ratio}".format(
            done=done,
            total=total,
            name=result["case_name"],
            changed=result["changed_move"],
            base_ms=float(result["base"]["elapsed_ms"]),
            fast_ms=float(result["fast"]["elapsed_ms"]),
            ratio="-" if result["time_ratio"] is None else f"{result['time_ratio']:.3f}",
        ),
        flush=True,
    )


def parse_args() -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case-file", type=Path, default=root / "cases" / "match" / "standard.jsonl")
    parser.add_argument("--tag", action="append", default=[])
    parser.add_argument("--limit-cases", type=int)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--binary", default=str(root / "target" / "release" / "case_probe"))
    parser.add_argument("--base-profile", default="base")
    parser.add_argument("--fast-profile", default="fast")
    parser.add_argument("--base-tt-bits", type=int)
    parser.add_argument("--fast-tt-bits", type=int)
    parser.add_argument("--depth", type=int)
    parser.add_argument("--width", type=int)
    parser.add_argument("--root-profile", action="store_true")
    parser.add_argument("--base-extra-arg", action="append", default=[])
    parser.add_argument("--fast-extra-arg", action="append", default=[])
    parser.add_argument("--base-extra-args", default="")
    parser.add_argument("--fast-extra-args", default="")
    parser.add_argument("--output", type=Path, default=Path("/tmp/base_fast_case_bench.json"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    cases = load_cases(args.case_file.expanduser().resolve(), args.tag, args.limit_cases)
    if not cases:
        raise SystemExit(f"no cases selected from {args.case_file}")
    binary = str(Path(args.binary).expanduser())
    base_extra_args = [*shlex.split(args.base_extra_args), *args.base_extra_arg]
    fast_extra_args = [*shlex.split(args.fast_extra_args), *args.fast_extra_arg]
    started = time.perf_counter()
    results: list[dict[str, Any]] = []
    jobs = max(1, args.jobs)
    if jobs == 1:
        for index, case in enumerate(cases):
            result = run_one_case(
                index,
                case,
                binary=binary,
                base_profile=args.base_profile,
                fast_profile=args.fast_profile,
                base_tt_bits=args.base_tt_bits,
                fast_tt_bits=args.fast_tt_bits,
                depth=args.depth,
                width=args.width,
                root_profile=args.root_profile,
                base_extra_args=base_extra_args,
                fast_extra_args=fast_extra_args,
            )
            results.append(result)
            print_progress(len(results), len(cases), result)
    else:
        with ProcessPoolExecutor(max_workers=jobs) as pool:
            futures = {
                pool.submit(
                    run_one_case,
                    index,
                    case,
                    binary=binary,
                    base_profile=args.base_profile,
                    fast_profile=args.fast_profile,
                    base_tt_bits=args.base_tt_bits,
                    fast_tt_bits=args.fast_tt_bits,
                    depth=args.depth,
                    width=args.width,
                    root_profile=args.root_profile,
                    base_extra_args=base_extra_args,
                    fast_extra_args=fast_extra_args,
                ): index
                for index, case in enumerate(cases)
            }
            indexed = []
            done = 0
            for future in as_completed(futures):
                result = future.result()
                indexed.append((futures[future], result))
                done += 1
                print_progress(done, len(cases), result)
            results = [result for _, result in sorted(indexed, key=lambda item: item[0])]

    payload = {
        "settings": {
            "case_file": str(args.case_file),
            "tags": args.tag,
            "jobs": jobs,
            "binary": binary,
            "base_profile": args.base_profile,
            "fast_profile": args.fast_profile,
            "base_tt_bits": args.base_tt_bits,
            "fast_tt_bits": args.fast_tt_bits,
            "depth": args.depth,
            "width": args.width,
            "root_profile": args.root_profile,
            "base_extra_args": base_extra_args,
            "fast_extra_args": fast_extra_args,
        },
        "summary": summarize(results),
        "results": results,
        "elapsed_s": round(time.perf_counter() - started, 3),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps({"summary": payload["summary"], "elapsed_s": payload["elapsed_s"]}, indent=2))
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
