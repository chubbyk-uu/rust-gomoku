# Performance and playing-strength log

Results identify the tested binary, rules, clock, threads, and opening set. Experimental builds are labelled with their tested state. See [testing](testing.md) for the procedure and the [archive](archive/README.md#performance) for older measurements and reverted experiments.

## 2026-11-28 — Route A correctness fixes and 600 games

Route A reused native NNUE evaluators in Gomocup and changed VNNI kernels, board pattern caching, move records, and some search extensions and history updates. The audited build restored policy-buffer initialization and synchronized the Renju forbidden-loss board state with referee adjudication. A 30-position fixed-depth regression matched Route A's moves, scores, depths, nodes, and PVs after the two fixes. The 13 pinned classical diffs also passed.

The benchmark used the development worktree at `ff221e6` plus the Route A changes and the two correctness fixes. The candidate `gomocup_engine` SHA-256 was `f1e1f0c5c861fb604b1c9b7c70e9a15558283d8f116211f4daac75b9da902715`. The Rapfi binary SHA-256 was `b3b0824a653953cb9cca5fab467710df4972cdb8e04cc705e50e795be1cdc885`.

Both sides used the same `Mix9svq` weights and the `model210901.bin` model, `timeout_turn=1000`, `timeout_match=1000000`, `time_left=1000000`, and `max_memory=33554432`. Rapfi's database was disabled. Each opening was played in both colors with `--reuse-engine-state`, a 225-ply cap, and the Renju referee for Renju. The historical comparison used the same opening coordinates, colors, and settings.

| Rules and threads | Games / jobs | W–L–D | Score and 95% interval | Elo | Previous score | Change and paired 95% interval (pp) |
|---|---:|---:|---|---:|---:|---|
| Freestyle 1T | 100 / 10 | 39–40–21 | 49.50% [45.02%, 53.98%] | −3.47 | 49.00% | +0.50 [−6.50, +7.50] |
| Freestyle 4T | 100 / 5 | 43–34–23 | 54.50% [49.58%, 59.42%] | +31.35 | 51.50% | +3.00 [−2.50, +8.50] |
| Renju 1T | 200 / 8 | 85–84–31 | 50.25% [47.27%, 53.23%] | +1.74 | 51.75% | −1.50 [−5.50, +2.25] |
| Renju 4T | 200 / 5 | 82–80–38 | 50.50% [47.99%, 53.01%] | +3.47 | 51.00% | −0.50 [−4.00, +2.75] |
| Total | 600 | 249–238–113 | 50.92% [49.25%, 52.58%] | +6.37 | 51.00% | −0.08 [−2.50, +2.33] |

Intervals for totals account for reuse of the same openings across thread counts. All 600 games finished with zero protocol errors and zero watchdog timeouts. All measured Rust replies stayed below one second. Elapsed wall time was 96.7 minutes.

| Rules and threads | Rust mean / median / P95 / max (ms) | Rapfi mean / median / P95 / max (ms) | Rust score, black / white |
|---|---|---|---:|
| Freestyle 1T | 746.0 / 970.0 / 976.2 / 987.0 | 758.5 / 970.7 / 975.5 / 1017.3 | 62.0% / 37.0% |
| Freestyle 4T | 736.5 / 970.3 / 979.7 / 990.2 | 768.1 / 971.6 / 978.9 / 1035.6 | 69.0% / 40.0% |
| Renju 1T | 717.0 / 969.8 / 975.9 / 984.6 | 730.7 / 970.5 / 975.0 / 1040.9 | 64.0% / 36.5% |
| Renju 4T | 718.3 / 970.2 / 980.4 / 991.8 | 733.9 / 971.4 / 978.9 / 1076.0 | 63.0% / 38.0% |

Freestyle 4T had the largest positive point estimate. The interval for its gain still crosses zero. The Freestyle 1T pre-fix Route A comparison scored 50.5% under the same match settings.

### Renju 1T repeat

The second run used the exact same binaries, opening prefixes, clocks, and eight concurrent games. It completed in 24.1 minutes and scored **90–78–32 (53.00%)**, up 2.75 percentage points from the first run. The paired 95% interval for that change was −1.25 to +7.25 points.

| Opening subset | Historical baseline | First run | Repeat |
|---|---:|---:|---:|
| 50 Renjunet pairs | 50.50% | 50.00% | 53.00% |
| 50 balanced pairs | 53.00% | 50.50% | 53.00% |
| All 100 pairs | 51.75% | 50.25% | 53.00% |

Combining both runs gives **51.63%** (95% interval grouped by opening 49.88%–53.37%). The change against the historical 51.75% is −0.13 points, with a paired interval of about −3.13 to +3.00. One repeat game ended in an adjudicated black overline at the sole remaining intersection after 224 legal plies. There were zero protocol failures and zero watchdog timeouts.

The paired transcripts diverged in 185 of 200 games, with a median first divergence at ply 13. Rapfi moved differently first in 99 cases, and Rust in 86. Of the 49 changed outcomes, 48 first diverged on a move that took at least 900 ms in both runs. In the `bal_renju_p4_09` position before move 7, fixed-depth probes selected `(8,9)` at d16 and `(6,8)` at d17, the same two moves that diverged in the timed games. The timed logs did not capture the completed search depths.

### Local artifacts and earlier speed checks

The run archives are in the ignored `target/match-results/` directory.

- `routeA-fixed-600.UiX69vJC/` contains `manifest.json`, `worktree.patch`, the four raw match outputs, and `summary.json`.
- `routeA-fixed-renju-1t-repeat.Gqiw0Wee/` contains the repeat command, raw Renju 1T output, and paired summary.
- `center-openings/freestyle_50pairs.jsonl` and `center-openings/renju_100pairs.jsonl` contain the benchmark openings. Their SHA-256 values are `e420c3fb73c7b48f7b3c0b1fbd6f15d371a1a29e72b992e107ee321ffd329034` and `8f46e85c69e07f444d66aefd86f5e06a4eb12126d253480ad4a7584962d0ec0e`.

A separate 1T fixed-depth ablation kept the throughput changes, restored buffer initialization, and used the earlier search policy. It matched all 30 fixed-depth positions against `ff221e6`. On 12 equal-node Freestyle d13 positions the median elapsed time fell about 4%. The complete Route A search policy changes affected some fixed-depth results. The tested VNNI batch4 binary still spilled a register in its inner loop.
