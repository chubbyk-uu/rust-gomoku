#!/usr/bin/env python3
"""Analyze diff_probe --root-profile JSON outputs.

This script is read-only with respect to engine behavior. It consumes one or
more probe JSON files and reports where root search time is spent, plus simple
counterfactuals for root-width truncation and idealized late-candidate
parallelism.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path
from statistics import median
from typing import Any


def pct(part: float, total: float) -> float:
    if total <= 0.0:
        return 0.0
    return 100.0 * part / total


def percentile(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * q)))
    return ordered[index]


def stats(values: list[float]) -> dict[str, float]:
    if not values:
        return {"avg": 0.0, "median": 0.0, "p95": 0.0, "max": 0.0}
    return {
        "avg": round(sum(values) / len(values), 3),
        "median": round(median(values), 3),
        "p95": round(percentile(values, 0.95), 3),
        "max": round(max(values), 3),
    }


def move_key(move: Any) -> str:
    if move is None:
        return "-"
    if isinstance(move, list) and len(move) == 2:
        return f"{move[0]},{move[1]}"
    return str(move)


def candidate_float(candidate: dict[str, Any], *keys: str) -> float:
    for key in keys:
        value = candidate.get(key)
        if value is not None:
            return float(value)
    return 0.0


def candidate_bool(candidate: dict[str, Any], key: str) -> bool:
    value = candidate.get(key)
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.lower() == "true"
    return bool(value)


@dataclass
class TailCutoffConfig:
    min_depth: int
    min_candidates: int
    window: int
    min_elapsed_ms: float
    score_guard: int


@dataclass
class TailCutoffSimulation:
    triggered: bool
    evaluated: int
    cutoff_after_index: int | None
    saved_ms: float
    saved_pct: float
    missed_improvements: int
    missed_cutoffs: int
    safe: bool


@dataclass
class DepthAnalysis:
    depth: int
    elapsed_ms: float
    nodes: int
    score: int
    best_move: Any
    candidates: int
    first_ms: float
    top3_ms: float
    late_ms: float
    max_candidate_ms: float
    max_candidate_index: int | None
    fail_low: int
    improved: int
    first_pct: float
    late_pct: float
    max_candidate_pct: float
    last_improved_index: int | None
    post_last_improved_ms: float
    post_last_improved_pct: float
    speculative_fail_high_vs_first: int
    speculative_fail_high_ms_vs_first: float
    speculative_fail_high_pct_vs_first: float
    zero_window_ms: float
    full_window_ms: float
    pvs_researches: int
    pvs_research_full_ms: float
    pvs_research_full_pct: float
    tail_cutoff: dict[str, Any]
    improved_after_width: dict[str, int]
    saved_ms_by_width: dict[str, float]
    late_parallel_ms: dict[str, float]
    late_parallel_speedup: dict[str, float]
    ordered_tail_parallel_ms: dict[str, float]
    ordered_tail_parallel_speedup: dict[str, float]


def simulate_tail_cutoff(
    *,
    depth: int,
    elapsed_ms: float,
    candidates: list[dict[str, Any]],
    config: TailCutoffConfig,
) -> TailCutoffSimulation:
    if depth < config.min_depth or len(candidates) < config.min_candidates:
        return TailCutoffSimulation(False, 0, None, 0.0, 0.0, 0, 0, True)

    current_best_score: int | None = None
    last_improved_index: int | None = None

    for index, candidate in enumerate(candidates):
        reason = candidate.get("reason")
        if reason in ("improved", "root_win"):
            current_best_score = int(candidate.get("score") or 0)
            last_improved_index = index

        evaluated = index + 1
        if evaluated < config.min_candidates or evaluated < config.window:
            continue
        if current_best_score is None or abs(current_best_score) >= config.score_guard:
            continue

        recent = candidates[evaluated - config.window : evaluated]
        if any(
            candidate.get("reason") in ("improved", "root_win", "beta_cutoff")
            for candidate in recent
        ):
            continue
        if any(candidate_bool(candidate, "pvs_research") for candidate in recent):
            continue

        since_index = 0 if last_improved_index is None else last_improved_index + 1
        elapsed_since_improve = sum(
            float(candidate["elapsed_ms"]) for candidate in candidates[since_index:evaluated]
        )
        if elapsed_since_improve < config.min_elapsed_ms:
            continue

        tail = candidates[evaluated:]
        saved_ms = sum(float(candidate["elapsed_ms"]) for candidate in tail)
        missed_improvements = sum(1 for candidate in tail if candidate.get("reason") == "improved")
        missed_cutoffs = sum(
            1 for candidate in tail if candidate.get("reason") in ("root_win", "beta_cutoff")
        )
        return TailCutoffSimulation(
            triggered=True,
            evaluated=evaluated,
            cutoff_after_index=index,
            saved_ms=round(saved_ms, 3),
            saved_pct=round(pct(saved_ms, elapsed_ms), 1),
            missed_improvements=missed_improvements,
            missed_cutoffs=missed_cutoffs,
            safe=missed_improvements == 0 and missed_cutoffs == 0,
        )

    return TailCutoffSimulation(False, 0, None, 0.0, 0.0, 0, 0, True)


def analyze_depth(
    profile: dict[str, Any],
    widths: list[int],
    workers: list[int],
    tail_config: TailCutoffConfig,
) -> DepthAnalysis:
    candidates = profile.get("candidates") or []
    elapsed_ms = float(profile.get("elapsed_ms") or 0.0)
    first_ms = float(candidates[0]["elapsed_ms"]) if candidates else 0.0
    top3_ms = sum(float(candidate["elapsed_ms"]) for candidate in candidates[:3])
    late = candidates[3:]
    late_ms = sum(float(candidate["elapsed_ms"]) for candidate in late)
    max_candidate = max(candidates, key=lambda item: float(item["elapsed_ms"]), default=None)
    fail_low = sum(1 for candidate in candidates if candidate.get("reason") == "fail_low")
    improved = sum(1 for candidate in candidates if candidate.get("reason") == "improved")
    improved_indices = [
        int(candidate["index"]) for candidate in candidates if candidate.get("reason") == "improved"
    ]
    last_improved_index = max(improved_indices, default=None)
    post_last_improved = (
        candidates[last_improved_index + 1 :] if last_improved_index is not None else candidates
    )
    post_last_improved_ms = sum(float(candidate["elapsed_ms"]) for candidate in post_last_improved)
    first_alpha_after = candidates[0].get("alpha_after") if candidates else None
    speculative_fail_high_candidates = (
        [
            candidate
            for candidate in candidates[1:]
            if first_alpha_after is not None
            and int(candidate.get("score") or 0) > int(first_alpha_after)
        ]
        if candidates
        else []
    )
    speculative_fail_high_ms = sum(
        float(candidate["elapsed_ms"]) for candidate in speculative_fail_high_candidates
    )
    zero_window_ms = sum(
        candidate_float(candidate, "zero_window_elapsed_ms", "zero_window_ms")
        for candidate in candidates
    )
    full_window_ms = sum(
        candidate_float(candidate, "full_window_elapsed_ms", "full_window_ms")
        for candidate in candidates
    )
    pvs_research_candidates = [
        candidate for candidate in candidates if candidate_bool(candidate, "pvs_research")
    ]
    pvs_research_full_ms = sum(
        candidate_float(candidate, "full_window_elapsed_ms", "full_window_ms")
        for candidate in pvs_research_candidates
    )
    tail_cutoff = simulate_tail_cutoff(
        depth=int(profile["depth"]),
        elapsed_ms=elapsed_ms,
        candidates=candidates,
        config=tail_config,
    )

    saved_ms_by_width: dict[str, float] = {}
    improved_after_width: dict[str, int] = {}
    for width in widths:
        tail = candidates[width:]
        saved_ms_by_width[str(width)] = round(sum(float(c["elapsed_ms"]) for c in tail), 3)
        improved_after_width[str(width)] = sum(1 for c in tail if c.get("reason") == "improved")

    late_times = [float(candidate["elapsed_ms"]) for candidate in candidates[1:]]
    late_parallel_ms: dict[str, float] = {}
    late_parallel_speedup: dict[str, float] = {}
    ordered_tail_parallel_ms: dict[str, float] = {}
    ordered_tail_parallel_speedup: dict[str, float] = {}
    serial_total = sum(float(candidate["elapsed_ms"]) for candidate in candidates)
    first_serial = first_ms
    for worker_count in workers:
        loads = [0.0 for _ in range(max(1, worker_count))]
        for elapsed in sorted(late_times, reverse=True):
            index = min(range(len(loads)), key=lambda i: loads[i])
            loads[index] += elapsed
        ideal = first_serial + (max(loads) if loads else 0.0)
        late_parallel_ms[str(worker_count)] = round(ideal, 3)
        late_parallel_speedup[str(worker_count)] = (
            0.0 if ideal <= 0.0 else round(serial_total / ideal, 3)
        )

        if last_improved_index is None:
            ordered_prefix_ms = 0.0
            ordered_tail_times = [float(candidate["elapsed_ms"]) for candidate in candidates]
        else:
            ordered_prefix_ms = sum(
                float(candidate["elapsed_ms"]) for candidate in candidates[: last_improved_index + 1]
            )
            ordered_tail_times = [
                float(candidate["elapsed_ms"]) for candidate in candidates[last_improved_index + 1 :]
            ]
        ordered_loads = [0.0 for _ in range(max(1, worker_count))]
        for elapsed in sorted(ordered_tail_times, reverse=True):
            index = min(range(len(ordered_loads)), key=lambda i: ordered_loads[i])
            ordered_loads[index] += elapsed
        ordered_ideal = ordered_prefix_ms + (max(ordered_loads) if ordered_loads else 0.0)
        ordered_tail_parallel_ms[str(worker_count)] = round(ordered_ideal, 3)
        ordered_tail_parallel_speedup[str(worker_count)] = (
            0.0 if ordered_ideal <= 0.0 else round(serial_total / ordered_ideal, 3)
        )

    return DepthAnalysis(
        depth=int(profile["depth"]),
        elapsed_ms=elapsed_ms,
        nodes=int(profile.get("nodes") or 0),
        score=int(profile.get("score") or 0),
        best_move=profile.get("best_move"),
        candidates=len(candidates),
        first_ms=round(first_ms, 3),
        top3_ms=round(top3_ms, 3),
        late_ms=round(late_ms, 3),
        max_candidate_ms=round(float(max_candidate["elapsed_ms"]), 3) if max_candidate else 0.0,
        max_candidate_index=int(max_candidate["index"]) if max_candidate else None,
        fail_low=fail_low,
        improved=improved,
        first_pct=round(pct(first_ms, elapsed_ms), 1),
        late_pct=round(pct(late_ms, elapsed_ms), 1),
        max_candidate_pct=round(
            pct(float(max_candidate["elapsed_ms"]), elapsed_ms), 1
        )
        if max_candidate
        else 0.0,
        last_improved_index=last_improved_index,
        post_last_improved_ms=round(post_last_improved_ms, 3),
        post_last_improved_pct=round(pct(post_last_improved_ms, elapsed_ms), 1),
        speculative_fail_high_vs_first=len(speculative_fail_high_candidates),
        speculative_fail_high_ms_vs_first=round(speculative_fail_high_ms, 3),
        speculative_fail_high_pct_vs_first=round(pct(speculative_fail_high_ms, elapsed_ms), 1),
        zero_window_ms=round(zero_window_ms, 3),
        full_window_ms=round(full_window_ms, 3),
        pvs_researches=len(pvs_research_candidates),
        pvs_research_full_ms=round(pvs_research_full_ms, 3),
        pvs_research_full_pct=round(pct(pvs_research_full_ms, elapsed_ms), 1),
        tail_cutoff=tail_cutoff.__dict__,
        improved_after_width=improved_after_width,
        saved_ms_by_width=saved_ms_by_width,
        late_parallel_ms=late_parallel_ms,
        late_parallel_speedup=late_parallel_speedup,
        ordered_tail_parallel_ms=ordered_tail_parallel_ms,
        ordered_tail_parallel_speedup=ordered_tail_parallel_speedup,
    )


def classify(depth: DepthAnalysis) -> list[str]:
    labels: list[str] = []
    if depth.first_pct >= 60.0:
        labels.append("PV-heavy")
    if depth.late_pct >= 50.0:
        labels.append("late-heavy")
    if depth.max_candidate_pct >= 25.0 and depth.max_candidate_index not in (None, 0):
        labels.append("single-late-spike")
    if depth.late_pct >= 50.0 and depth.max_candidate_pct < 25.0:
        labels.append("multi-late-tail")
    if depth.improved >= 3:
        labels.append("unstable-PV")
    if not labels:
        labels.append("mixed")
    return labels


def analyze_file(
    path: Path,
    widths: list[int],
    workers: list[int],
    tail_config: TailCutoffConfig,
) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    root = payload["root"]
    profiles = root.get("trace", {}).get("root_profiles") or []
    depths = [analyze_depth(profile, widths, workers, tail_config) for profile in profiles]
    final_depth = max(depths, key=lambda item: item.depth) if depths else None
    return {
        "path": str(path),
        "name": payload.get("name", path.stem),
        "result_move": root.get("move"),
        "result_score": root.get("score"),
        "result_depth": root.get("depth"),
        "result_nodes": root.get("nodes"),
        "classification": classify(final_depth) if final_depth else [],
        "depths": [depth.__dict__ for depth in depths],
    }


def markdown_report(summary: dict[str, Any]) -> str:
    lines = ["# Root Profile Analysis", ""]
    lines.append("## Cases")
    lines.append("")
    lines.append(
        "| case | class | d | ms | nodes | score | first% | late% | post-last% | stale-FH% | last imp | max cand | fail-low | improved | w20 saved | w30 saved | ideal x4 | ordered x4 |"
    )
    lines.append(
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|"
    )
    for case in summary["cases"]:
        depths = case["depths"]
        if not depths:
            continue
        d = depths[-1]
        labels = ",".join(case["classification"])
        max_cand = (
            "-"
            if d["max_candidate_index"] is None
            else f"#{d['max_candidate_index']} {d['max_candidate_ms']:.1f}ms"
        )
        lines.append(
            "| {name} | {labels} | {depth} | {ms:.1f} | {nodes} | {score} | "
            "{first:.1f} | {late:.1f} | {post_last:.1f} | {stale_fh:.1f} | {last_imp} | "
            "{max_cand} | {fail_low} | {improved} | {w20:.1f} | {w30:.1f} | "
            "{x4:.2f} | {ordered_x4:.2f} |".format(
                name=case["name"],
                labels=labels,
                depth=d["depth"],
                ms=d["elapsed_ms"],
                nodes=d["nodes"],
                score=d["score"],
                first=d["first_pct"],
                late=d["late_pct"],
                post_last=d["post_last_improved_pct"],
                stale_fh=d["speculative_fail_high_pct_vs_first"],
                last_imp="-" if d["last_improved_index"] is None else d["last_improved_index"],
                max_cand=max_cand,
                fail_low=d["fail_low"],
                improved=d["improved"],
                w20=d["saved_ms_by_width"].get("20", 0.0),
                w30=d["saved_ms_by_width"].get("30", 0.0),
                x4=d["late_parallel_speedup"].get("4", 0.0),
                ordered_x4=d["ordered_tail_parallel_speedup"].get("4", 0.0),
            )
        )
    lines.append("")
    lines.append("## PVS Re-search")
    lines.append("")
    lines.append(
        "| case | d | pvs researches | re-search full ms | re-search full % | zero-window ms | full-window ms |"
    )
    lines.append("|---|---:|---:|---:|---:|---:|---:|")
    for case in summary["cases"]:
        depths = case["depths"]
        if not depths:
            continue
        d = depths[-1]
        lines.append(
            "| {name} | {depth} | {researches} | {research_ms:.1f} | {research_pct:.1f} | {zero_ms:.1f} | {full_ms:.1f} |".format(
                name=case["name"],
                depth=d["depth"],
                researches=d["pvs_researches"],
                research_ms=d["pvs_research_full_ms"],
                research_pct=d["pvs_research_full_pct"],
                zero_ms=d["zero_window_ms"],
                full_ms=d["full_window_ms"],
            )
        )
    lines.append("")
    lines.append("## Tail Cutoff Simulation")
    lines.append("")
    lines.append(
        "| case | d | triggered | evaluated | saved ms | saved % | missed improved | missed cutoffs | safe |"
    )
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for case in summary["cases"]:
        depths = case["depths"]
        if not depths:
            continue
        d = depths[-1]
        tail = d["tail_cutoff"]
        lines.append(
            "| {name} | {depth} | {triggered} | {evaluated} | {saved_ms:.1f} | {saved_pct:.1f} | {missed_improvements} | {missed_cutoffs} | {safe} |".format(
                name=case["name"],
                depth=d["depth"],
                triggered=tail["triggered"],
                evaluated=tail["evaluated"],
                saved_ms=tail["saved_ms"],
                saved_pct=tail["saved_pct"],
                missed_improvements=tail["missed_improvements"],
                missed_cutoffs=tail["missed_cutoffs"],
                safe=tail["safe"],
            )
        )
    lines.append("")
    lines.append("## Aggregate")
    lines.append("")
    aggregate = summary["aggregate"]
    for key, value in aggregate.items():
        lines.append(f"- `{key}`: `{value}`")
    lines.append("")
    return "\n".join(lines)


def aggregate(cases: list[dict[str, Any]]) -> dict[str, Any]:
    final_depths = [case["depths"][-1] for case in cases if case["depths"]]
    labels: dict[str, int] = {}
    for case in cases:
        for label in case["classification"]:
            labels[label] = labels.get(label, 0) + 1
    return {
        "cases": len(cases),
        "labels": labels,
        "final_elapsed_ms": stats([d["elapsed_ms"] for d in final_depths]),
        "final_nodes": stats([float(d["nodes"]) for d in final_depths]),
        "first_pct": stats([d["first_pct"] for d in final_depths]),
        "late_pct": stats([d["late_pct"] for d in final_depths]),
        "max_candidate_pct": stats([d["max_candidate_pct"] for d in final_depths]),
        "post_last_improved_pct": stats([d["post_last_improved_pct"] for d in final_depths]),
        "speculative_fail_high_pct_vs_first": stats(
            [d["speculative_fail_high_pct_vs_first"] for d in final_depths]
        ),
        "speculative_fail_high_count_vs_first": sum(
            d["speculative_fail_high_vs_first"] for d in final_depths
        ),
        "pvs_research_count": sum(d["pvs_researches"] for d in final_depths),
        "pvs_research_full_ms": stats([d["pvs_research_full_ms"] for d in final_depths]),
        "pvs_research_full_pct": stats([d["pvs_research_full_pct"] for d in final_depths]),
        "zero_window_ms": stats([d["zero_window_ms"] for d in final_depths]),
        "full_window_ms": stats([d["full_window_ms"] for d in final_depths]),
        "tail_cutoff_triggered": sum(1 for d in final_depths if d["tail_cutoff"]["triggered"]),
        "tail_cutoff_safe": sum(
            1
            for d in final_depths
            if d["tail_cutoff"]["triggered"] and d["tail_cutoff"]["safe"]
        ),
        "tail_cutoff_saved_ms": stats(
            [d["tail_cutoff"]["saved_ms"] for d in final_depths if d["tail_cutoff"]["triggered"]]
        ),
        "tail_cutoff_saved_pct": stats(
            [d["tail_cutoff"]["saved_pct"] for d in final_depths if d["tail_cutoff"]["triggered"]]
        ),
        "tail_cutoff_missed_improvements": sum(
            d["tail_cutoff"]["missed_improvements"] for d in final_depths
        ),
        "tail_cutoff_missed_cutoffs": sum(d["tail_cutoff"]["missed_cutoffs"] for d in final_depths),
        "ideal_x4_speedup": stats(
            [d["late_parallel_speedup"].get("4", 0.0) for d in final_depths]
        ),
        "ordered_tail_x4_speedup": stats(
            [d["ordered_tail_parallel_speedup"].get("4", 0.0) for d in final_depths]
        ),
        "w20_saved_ms": stats([d["saved_ms_by_width"].get("20", 0.0) for d in final_depths]),
        "w30_saved_ms": stats([d["saved_ms_by_width"].get("30", 0.0) for d in final_depths]),
        "improved_after_w20": sum(d["improved_after_width"].get("20", 0) for d in final_depths),
        "improved_after_w30": sum(d["improved_after_width"].get("30", 0) for d in final_depths),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profiles", nargs="+", type=Path)
    parser.add_argument("--width", action="append", type=int, default=[10, 20, 30])
    parser.add_argument("--worker", action="append", type=int, default=[2, 4, 8])
    parser.add_argument("--tail-min-depth", type=int, default=8)
    parser.add_argument("--tail-min-candidates", type=int, default=20)
    parser.add_argument("--tail-window", type=int, default=10)
    parser.add_argument("--tail-min-elapsed-ms", type=float, default=800.0)
    parser.add_argument("--tail-score-guard", type=int, default=19_000)
    parser.add_argument("--output-json", type=Path, default=Path("/tmp/root_profile_analysis.json"))
    parser.add_argument("--output-md", type=Path, default=Path("/tmp/root_profile_analysis.md"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    widths = sorted(set(args.width))
    workers = sorted(set(args.worker))
    tail_config = TailCutoffConfig(
        min_depth=args.tail_min_depth,
        min_candidates=args.tail_min_candidates,
        window=args.tail_window,
        min_elapsed_ms=args.tail_min_elapsed_ms,
        score_guard=args.tail_score_guard,
    )
    cases = [analyze_file(path, widths, workers, tail_config) for path in args.profiles]
    summary = {
        "settings": {
            "profiles": [str(path) for path in args.profiles],
            "widths": widths,
            "workers": workers,
            "tail_cutoff": tail_config.__dict__,
        },
        "aggregate": aggregate(cases),
        "cases": cases,
    }
    args.output_json.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    args.output_md.write_text(markdown_report(summary), encoding="utf-8")
    print(json.dumps({"aggregate": summary["aggregate"]}, indent=2))
    print(f"wrote {args.output_json}")
    print(f"wrote {args.output_md}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
