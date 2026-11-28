# Performance and playing-strength log

Results identify the tested binary, rules, clock, threads, and opening set. Experimental builds are labelled with their tested state. See [testing](testing.md) for the procedure and the [archive](archive/README.md#performance) for older measurements and reverted experiments.

## 2026-11-28 — AVX-VNNI batch-four register scheduling

The twelve-accumulator tile in the Mix9svq value head now uses one register to broadcast the four inputs in sequence. Disassembly of the release build confirms that the inner loop keeps the accumulators in registers. The dot products use the same AVX-VNNI instructions and accumulate each output in the original order.

On the i7-13700F development machine, the pre-change and candidate release builds searched the same 32 positions, evenly split between Freestyle and Renju. Each search used one thread, a cold 32 MiB TT, the same Mix9svq weights, and a fixed depth with no clock. All 128 comparisons matched in best move, score, completed depth, selective depth, node count, and the PV prefix reported by Gomocup.

| Depth | Repeats per position | Positions in timing median | Median elapsed time change | Total elapsed time change |
|---|---:|---:|---:|---:|
| 14 | 3 | 23 / 32 | −2.32% | −1.87% |
| 18 | 1 | 28 / 32 | −1.67% | −1.65% |

The timing median uses paired ratios from baseline searches of at least 25 ms and takes the median of repeats within each position. The total includes every search. The runs used the position manifest in ignored `target/search-efficiency.yaF8pC8l/`. Binaries, the comparison script, and raw data are in ignored `target/nnue-throughput.eDEdZWZE/`.

A 12-position 4T smoke test used a 1-second turn limit. The candidate completed a deeper iteration on four positions, the same depth on two, and a shallower iteration on six. Maximum internal search times were 974.8 ms for the baseline and 974.3 ms for the candidate. Repeating the baseline binary alone changed completed depth on eight positions, best move on two, and node count on all twelve. The timed sample leaves the 4T performance effect unresolved. The 138 Rust library tests passed, including the scalar/SIMD comparison. The Linux and Windows GNU engine builds passed.

## 2026-11-28 — Mix9svq incremental update experiments

The AVX-VNNI build above served as the baseline. The same 32 Freestyle and Renju positions were searched once per variant at fixed d14 with one thread and a cold 32 MiB TT. Best move, score, completed depth, selective depth, nodes, and the reported PV prefix matched in all 192 comparisons. The timing median includes positions with a baseline search of at least 25 ms.

| Variant | Median elapsed time change | Total elapsed time change |
|---|---:|---:|
| Compile-time ray directions | −2.01% | −2.91% |
| Compile-time directions and precomputed ray cells | −3.36% | −2.75% |
| Apply group updates directly to the next version | 0.00% | +2.74% |
| Initialize convolution cells from the previous version | −0.35% | +0.06% |
| Skip convolution updates when both input vectors are equal | +0.56% | +0.41% |
| Build the baseline for `x86-64-v3` | +0.13% | −1.52% |

The precomputed-ray variant also matched all 32 d18 comparisons. Its median elapsed time fell 3.22% and total elapsed time fell 2.31%. Local scripts, binaries, the precomputed-ray patch, and per-position results are in ignored `target/nnue-update.FOohWluL/`.

### Retained build and Renju 1T match

The retained build passed 138 Rust library tests. It matched all 64 fixed-depth comparisons against the preceding AVX-VNNI build at d14 and d18. Median elapsed time fell 2.94% at d14 and 2.95% at d18.

The Renju match used the archived Rapfi binary and the exact same 100 opening pairs, colors, weights, 32 MiB TT, 1-second turn limit, engine state reuse, and eight concurrent games as the Route A benchmark and repeat below. The retained Rust binary SHA-256 was `69ef7efcf610bbfd2c9d78fb70a901a1ac24b22fc02b81a9e1ceccdb1ca0ea88`.

| Openings | Games | W–L–D | Score and paired 95% interval | Route A first | Route A repeat |
|---|---:|---:|---:|---:|---:|
| 50 Renjunet pairs | 100 | 48–42–10 | 53.00% [48.48%, 57.52%] | 50.00% | 53.00% |
| 50 balanced pairs | 100 | 39–42–19 | 48.50% [44.03%, 52.97%] | 50.50% | 53.00% |
| Total | 200 | 87–84–29 | 50.75% [47.69%, 53.81%] | 50.25% | 53.00% |

The overall estimate was +5.21 Elo. The score change relative to the mean of the two Route A runs was −0.88 percentage points with a paired 95% interval of [−3.50, +1.88]. For the balanced subset, the change from the Route A repeat was −4.50 points [−9.50, 0.00]. Rust scored 61.50% as Black and 40.00% as White.

The batch took 23.7 minutes. There were zero protocol failures and zero watchdog timeouts. Two Renjunet games ended in a Rapfi Black overline at the only empty intersection after 224 legal plies. Rust mean, median, P95, and maximum move times were 723.8, 969.9, 975.5, and 984.1 ms. Rapfi's were 741.1, 970.5, 974.9, and 1042.9 ms. The raw games, commands, binary hashes, and paired summary are in ignored `target/match-results/ray-renju-1t.IDOQhcbv/`.

### Balanced Renju repeat

The same two binaries replayed the same 50 balanced opening pairs with the same protocol settings and eight concurrent games. The repeat ran only the balanced subset. Rust scored **45–34–21 (55.50%)** in 16.1 minutes, up 7.00 percentage points from 48.50% in the first run. The paired 95% interval for the change was [+1.00, +13.50] points. Rust scored 62.00% as Black and 49.00% as White. Both runs together scored **84–76–40 (52.00%)**, with a 95% interval of [49.00%, 55.25%] grouped by opening.

The first differing move occurred at a median of ply 14 across 96 divergent games. Rapfi moved differently first in 53 games and Rust in 43. Of the 31 changed results, 29 first diverged on a move that took at least 900 ms in both runs.

There were zero protocol failures and zero watchdog timeouts. In `bal_renju_p5_27`, Rapfi lost with a Black overline at the sole empty intersection after 224 legal plies. Rust mean, median, P95, and maximum move times were 761.9, 970.0, 975.4, and 983.0 ms. Rapfi's were 771.9, 970.6, 974.8, and 1041.8 ms. The logs and paired summary are in ignored `target/match-results/ray-renju-balanced-repeat.GYUqbnBB/`.

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
