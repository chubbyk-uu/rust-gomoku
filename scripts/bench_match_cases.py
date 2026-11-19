#!/usr/bin/env python3
"""Compare two case_probe builds or configurations on identical positions."""

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
            depth=depth,
            width=width,
            tt_bits=tt_bits,
            root_profile=root_profile,
            extra_args=extra_args,
        ),
        cwd=repo_root(),
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"{binary} exited with {result.returncode}: {result.stderr.strip()}")
    return json.loads(result.stdout)


def run_one_case(
    index: int,
    case: dict[str, Any],
    *,
    baseline_binary: str,
    candidate_binary: str,
    baseline_tt_bits: int | None,
    candidate_tt_bits: int | None,
    depth: int | None,
    width: int | None,
    root_profile: bool,
    baseline_extra_args: list[str],
    candidate_extra_args: list[str],
) -> dict[str, Any]:
    baseline = run_probe(
        binary=baseline_binary,
        case=case,
        depth=depth,
        width=width,
        tt_bits=baseline_tt_bits,
        root_profile=root_profile,
        extra_args=baseline_extra_args,
    )
    candidate = run_probe(
        binary=candidate_binary,
        case=case,
        depth=depth,
        width=width,
        tt_bits=candidate_tt_bits,
        root_profile=root_profile,
        extra_args=candidate_extra_args,
    )
    if baseline["rule_set"] != candidate["rule_set"]:
        raise ValueError(f"different rules for {baseline['case_name']}")
    baseline_result = baseline["result"]
    candidate_result = candidate["result"]
    baseline_time = float(baseline_result["elapsed_ms"])
    candidate_time = float(candidate_result["elapsed_ms"])
    baseline_nodes = int(baseline_result["nodes"])
    candidate_nodes = int(candidate_result["nodes"])
    return {
        "index": index,
        "case_name": baseline["case_name"],
        "tags": baseline.get("tags", []),
        "prefix_plies": baseline["prefix_plies"],
        "rule_set": baseline["rule_set"],
        "side_to_move": baseline["side_to_move"],
        "baseline": baseline_result,
        "candidate": candidate_result,
        "baseline_trace": baseline.get("trace"),
        "candidate_trace": candidate.get("trace"),
        "changed_move": baseline_result["move"] != candidate_result["move"],
        "changed_score": baseline_result["score"] != candidate_result["score"],
        "changed_depth": baseline_result["depth"] != candidate_result["depth"],
        "node_ratio": None if baseline_nodes == 0 else candidate_nodes / baseline_nodes,
        "time_ratio": None if baseline_time == 0.0 else candidate_time / baseline_time,
        "delta_ms": candidate_time - baseline_time,
        "delta_nodes": candidate_nodes - baseline_nodes,
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
    baseline_times = [float(item["baseline"]["elapsed_ms"]) for item in results]
    candidate_times = [float(item["candidate"]["elapsed_ms"]) for item in results]
    baseline_nodes = [float(item["baseline"]["nodes"]) for item in results]
    candidate_nodes = [float(item["candidate"]["nodes"]) for item in results]
    time_ratios = [float(item["time_ratio"]) for item in results if item["time_ratio"] is not None]
    node_ratios = [float(item["node_ratio"]) for item in results if item["node_ratio"] is not None]
    return {
        "cases": len(results),
        "changed_moves": sum(1 for item in results if item["changed_move"]),
        "changed_scores": sum(1 for item in results if item["changed_score"]),
        "candidate_faster_cases": sum(1 for item in results if item["delta_ms"] < 0.0),
        "candidate_slower_cases": sum(1 for item in results if item["delta_ms"] > 0.0),
        "baseline_time_ms": stats(baseline_times),
        "candidate_time_ms": stats(candidate_times),
        "baseline_nodes": stats(baseline_nodes),
        "candidate_nodes": stats(candidate_nodes),
        "time_ratio": stats(time_ratios),
        "node_ratio": stats(node_ratios),
    }


def print_progress(done: int, total: int, result: dict[str, Any]) -> None:
    print(
        "[{done}/{total}] {name} changed_move={changed} "
        "baseline={baseline_ms:.1f}ms candidate={candidate_ms:.1f}ms ratio={ratio}".format(
            done=done,
            total=total,
            name=result["case_name"],
            changed=result["changed_move"],
            baseline_ms=float(result["baseline"]["elapsed_ms"]),
            candidate_ms=float(result["candidate"]["elapsed_ms"]),
            ratio="-" if result["time_ratio"] is None else f"{result['time_ratio']:.3f}",
        ),
        flush=True,
    )


def parse_args() -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--case-file", type=Path, default=root / "cases" / "match" / "standard.jsonl")
    parser.add_argument("--rule", choices=("renju", "freestyle"), help="rule for unlabelled prefixes")
    parser.add_argument("--tag", action="append", default=[])
    parser.add_argument("--limit-cases", type=int)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--binary", default=str(root / "target" / "release" / "case_probe"))
    parser.add_argument("--baseline-binary", help="baseline case_probe executable")
    parser.add_argument("--candidate-binary", help="candidate case_probe executable")
    parser.add_argument("--baseline-tt-bits", "--base-tt-bits", type=int)
    parser.add_argument("--candidate-tt-bits", "--fast-tt-bits", type=int)
    parser.add_argument("--depth", type=int)
    parser.add_argument("--width", type=int)
    parser.add_argument("--root-profile", action="store_true")
    parser.add_argument("--baseline-extra-arg", "--base-extra-arg", action="append", default=[])
    parser.add_argument("--candidate-extra-arg", "--fast-extra-arg", action="append", default=[])
    parser.add_argument("--baseline-extra-args", "--base-extra-args", default="")
    parser.add_argument("--candidate-extra-args", "--fast-extra-args", default="")
    parser.add_argument("--output", type=Path, default=Path("/tmp/case_bench.json"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    cases = load_cases(args.case_file.expanduser().resolve(), args.tag, args.limit_cases)
    if not cases:
        raise SystemExit(f"no cases selected from {args.case_file}")
    baseline_binary = str(Path(args.baseline_binary or args.binary).expanduser().resolve())
    candidate_binary = str(Path(args.candidate_binary or args.binary).expanduser().resolve())
    baseline_extra_args = [*shlex.split(args.baseline_extra_args), *args.baseline_extra_arg]
    candidate_extra_args = [*shlex.split(args.candidate_extra_args), *args.candidate_extra_arg]
    if args.rule:
        baseline_extra_args.extend(["--rule", args.rule])
        candidate_extra_args.extend(["--rule", args.rule])
    same_configuration = (
        baseline_binary == candidate_binary
        and args.baseline_tt_bits == args.candidate_tt_bits
        and baseline_extra_args == candidate_extra_args
    )
    if same_configuration:
        print("Identical probe configurations. This run measures repeatability.", flush=True)

    probe_options = dict(
        baseline_binary=baseline_binary,
        candidate_binary=candidate_binary,
        baseline_tt_bits=args.baseline_tt_bits,
        candidate_tt_bits=args.candidate_tt_bits,
        depth=args.depth,
        width=args.width,
        root_profile=args.root_profile,
        baseline_extra_args=baseline_extra_args,
        candidate_extra_args=candidate_extra_args,
    )
    started = time.perf_counter()
    results: list[dict[str, Any]] = []
    jobs = max(1, args.jobs)
    if jobs == 1:
        for index, case in enumerate(cases):
            result = run_one_case(index, case, **probe_options)
            results.append(result)
            print_progress(len(results), len(cases), result)
    else:
        with ProcessPoolExecutor(max_workers=jobs) as pool:
            futures = [
                pool.submit(run_one_case, index, case, **probe_options)
                for index, case in enumerate(cases)
            ]
            for future in as_completed(futures):
                result = future.result()
                results.append(result)
                print_progress(len(results), len(cases), result)
        results.sort(key=lambda item: item["index"])

    payload = {
        "settings": {
            "case_file": str(args.case_file),
            "tags": args.tag,
            "jobs": jobs,
            "baseline_binary": baseline_binary,
            "candidate_binary": candidate_binary,
            "same_configuration": same_configuration,
            "baseline_tt_bits": args.baseline_tt_bits,
            "candidate_tt_bits": args.candidate_tt_bits,
            "depth": args.depth,
            "width": args.width,
            "root_profile": args.root_profile,
            "baseline_extra_args": baseline_extra_args,
            "candidate_extra_args": candidate_extra_args,
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
