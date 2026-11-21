# Performance Log

## Current Performance Decision Rules

Do not repeat these directions unless new evidence invalidates the decision:

- `recompute_all` reuse: deprioritized. It is a root-entry fixed cost and has not appeared in slow-case root profiles.
- TT best-move trust or generation ordering: deprioritized. Old-generation hints exist, but slow moves had lower old-generation percentage than average.
- Shape reader LUT, indexed neighbor reader, and batch-lines: rejected as default paths after semantic-equivalent tests showed unstable or worse performance.
- Lazy SMP, root YBWC, root split, and aspiration window: rejected as default paths after poor speed/strength or maintenance tradeoffs.
- Early stop and broad dynamic pruning: only revisit with a concrete tactical safety proof and validation against the current mainline.
- VCF memo-key allocation and VCT/VCF as a per-node hotspot: confirmed non-bottleneck by the 2026-07-02 VCT-path profile (`or_node` 0.27% cumulative, the `(Side, Vec<Move>, Vec<Move>)` memo alloc + sort below the 0.8% floor). Do not pursue memo-key micro-optimization.
- Root LMR clamp (`net <= 1` for `self_attack >= 3` at root): rejected for default `base` after 100-game real Renju center-opening validation (`48-48-4` overall, `36-37-3` on 38 unseen holdout openings, `+6.3%` avg and `61.7s -> 144.3s` max regression).
- Restricting `extend_ratio` (`single_forcing || hostile_threat || case_count <= 8` and `ply < root_depth + 4`): rejected despite cutting `p95 -14.2%` and `max 57.4s -> 32.6s`, because 100-game real Renju center-opening strength dropped to `38-61-1` (`38.5%`, `-81.4 Elo`). Do not cut `extend_ratio` in midgame forcing chains.
- `ThreatBoardView` incremental `win_moves` bitset or `line_counts[88]` filter maintained on `play()`: rejected (`c25 p68` VCT6 regressed `+52.7%` / `+15.0%` / `+4.6%`) because trial plays dominate `first_counter_win` and dense midgame boards already fill most lines.

Current active target:

- Final-depth late root candidates. Future analysis should explain why they are expensive, not re-confirm that they are expensive.
- **TODO (VCT speed-up in dense late-game positions, 70+ plies, untimed Master `d8/w40 + VCT6`)**:
  - Serial root `VCT6` remains single-threaded and uncapped in untimed mode, dominating dense 70+-ply moves even with 4T ABDADA enabled (timed mode already caps root VCT to 30% of remaining move time).
  - Candidate directions to evaluate later (check `AGENTS.md` rules before modifying base):
    1. **Route A (low risk)**: evaluate an explicit node/time cap for untimed root VCT (or compare `root_vct_depth = 4` vs `6` strength/latency tradeoff).
    2. **Route B (behavior-equivalent speedup — DONE on `feat/threat-pattern4-bitboard`)**: replaced `ThreatBoardView` board scans and quiet trial plays with a Rapfi-style incremental 4-direction `Pattern4` bitboard/LUT (`6.65x–9.33x` VCT6 speedup across dense Renju cases; see 2026-11-21 entry below).
    3. **Route C (experimental branch)**: upgrade `VCTSearcher` from IDDFS to `df-pn` (proof-number search with TT), validated for zero Renju false positives.

## 2026-11-19 (3) — Incremental `dgn`, sparse `bucket_dot`, deferred rest-sort, and 4-line direct `value_wide_compute`

Context:

- Post-`215a896` `perf` profiles showed two remaining behavior-equivalent Alpha-Beta hotspots:
  - Dense late game (`c21 black p145`, 145 stones): `evaluate_board_main_cached` was **`47.4%`** of total CPU (`evaluate_dgn_from_occupied` scanned all 145 stones × 8 neighbors per leaf + `bucket_dot` visited 92 buckets per side).
  - Midgame forcing (`c43 white p32`, 32 stones): `value_wide_compute_for_rule` + `compute_bucket_attack_and_counts` was **`33.5%`**, `generate_candidates` + `sort` + `trunc` was **`21.2%`**, and `evaluate_board_main_cached` was **`16.2%`**.

Kept optimizations (100% behavior-equivalent):

- `src/eval/caches.rs`, `src/eval/local.rs`, `src/eval/global_eval.rs`:
  - Maintain per-stone `dgn_cc` and exact integer `dgn_sum: [i32; 2]` incrementally on center transitions and `shape_cache` zero/non-zero label flips (with `debug_assert_eq!` against `evaluate_dgn_from_occupied`), turning leaf `dgn` into O(1).
  - Maintain `[u64; 3]` non-zero bucket bitmasks per side so `bucket_dot_sparse` visits only non-zero buckets in identical ascending order.
  - Eliminate `comp[15][15]` / `dirty[225]` / division-modulo coordinate decoding in `value_wide_compute_for_rule` by traversing the 4 lines directly, updating `empty_bucket_counts` only when `changed_player` changes bucket, and computing triangular bucket indices inline without `% 13`.
- `src/search/movegen.rs`, `src/search/ordering.rs`, `src/search/alphabeta.rs`:
  - Swap `tt_best_move` to index 0 and defer sorting `ordered[1..]` until candidate #0 fails to beta-cutoff; inline `fast_trunc` to avoid libc `trunc()`; reuse `MoveAnalysis` scratch on `AlphaBetaSearcher` with compact `[i8; BOARD_AREA]` attack maps.

Rejected experiment (reverted):

- `ThreatBoardView` `line_counts[88]` + `lines_ge3/ge4` filter for `first_counter_win`: regressed `c25 p68` VCT6 `+15.0%` (line-scan) / `+4.6%` (flat-scan).

Verification & A/B timing (5 slow cases at d7, `compute_vct=0`, single-threaded release):

- `c43 white p32`: `7812.4ms -> 6893.4ms` (`-11.8%` vs `215a896`, **`-44.3%`** vs `23b72d6`), exact move `6,2`, score `-33`, nodes `5,295,482`.
- `c25 white p68`: `5332.5ms -> 4199.2ms` (`-21.3%` vs `215a896`, **`-46.4%`** vs `23b72d6`), exact move `10,4`, score `141`, nodes `3,115,273`.
- `c21 black p145`: `5644.1ms -> 3113.0ms` (`-44.8%` vs `215a896`, **`-48.6%`** vs `23b72d6`), exact move `2,14`, score `-10`, nodes `3,299,678`.
- `c34 white p154`: `5691.6ms -> 3161.4ms` (`-44.5%` vs `215a896`, **`-47.5%`** vs `23b72d6`), exact move `14,11`, score `-26`, nodes `3,091,334`.
- `c34 black p83`: `3816.7ms -> 2778.4ms` (`-27.2%` vs `215a896`, **`-50.1%`** vs `23b72d6`), exact move `6,13`, score `53`, nodes `2,496,885`.
- Total across 5 slow cases: `28297.3ms -> 20145.4ms` (`-28.8%` vs `215a896`, **`-46.8%`** vs `23b72d6`).
- Correctness: `cargo fmt --check`, full `cargo test` (with leaf `dgn` debug assertion), release builds, and `scripts/run_diff.py --profile all --check-baselines --jobs 8` (`13/13` passed) all green.

## 2026-11-19 (2) — Long-tail profiling & behavior-equivalent Alpha-Beta hot-path speedup

Context:

- Source: top 5 slowest moves from the 100-game Renju `center_50_prefixes.jsonl` match (`c43 p32` 55.6s / 26.7M nodes, `c25 p68` 53.9s = 16.0s VCT6 + 37.8s AB / 18.8M nodes, `c34 p154` 31.8s / 22.9M nodes, `c34 p83` 29.9s / 16.2M nodes, `c21 p145` 29.3s / 19.6M nodes).
- `perf` CPU profile on `c43 p32` (12K samples, pure Alpha-Beta):
  - `value_wide_compute_for_rule` (`32.0%`) + `compute_bucket_attack_and_counts` (`11.8%`) = **`43.8%`**.
  - `generate_candidates_with_coverage` (`9.7%`) + `getmi` (`7.6%`) + `sort` (`6.5%`) = **`23.8%`**.
- `perf` CPU profile on `c25 p68` (15K samples, VCT6):
  - `Board::is_winning_move_for_rule` (`28.0%`) + `first_counter_win` (`20.0%`) = **`48.0%`**, `legal_win_completions` (`18.2%`).

Rejected experiments (reverted):

- TT two-slot same-key dedup + full-bucket probe: mixed on the 5 slow cases (`c21 p145` `-44%`, `c34 p154/p83` `+5%` to `+15%`).
- `extend_ratio` tactical gate + `ply < root_depth + 4` cap: cut 5 slow cases up to `16x` and 100-game `p95 -14.2%` / `max -43.1%`, but lost `38-61-1` (`38.5%`, `-81.4 Elo`, `WW=3 / LL=14`) on `center_50_prefixes.jsonl`.
- `ThreatBoardView` incremental `win_moves` bitset on `play()`: regressed `c25 p68` VCT6 `+52.7%` (`15.4s -> 23.6s`).

Kept optimization (100% behavior-equivalent):

- `src/search/alphabeta.rs`: defer `caches.snapshot()` + `value_wide_compute_for_rule()` until after child `terminal_score` and `tt.probe` miss (reusing the materialized snapshot across PVS / `net > 0` re-searches of the same candidate). Eliminates incremental eval updates and snapshot/restore on every TT-cutoff and terminal child.
- `src/search/ordering.rs` + `src/search/alphabeta.rs`: for non-root, non-`fast_history_ordering`, non-`nonroot_vcf` nodes, run primary sort by `(tt_best_move, order_score)` only and resolve tie-group `getmi` sorting lazily when the candidate loop reaches each tie group.

Verification & A/B timing (5 slow cases at d7, `compute_vct=0`, single-threaded release):

- `c43 white p32`: `12381.2ms -> 7812.4ms` (**`-36.9%`**), identical move `6,2`, score `-33`, nodes `5,295,482`.
- `c25 white p68`: `7838.6ms -> 5332.5ms` (**`-32.0%`**), identical move `10,4`, score `141`, nodes `3,115,273`.
- `c34 black p83`: `5570.2ms -> 3816.7ms` (**`-31.5%`**), identical move `6,13`, score `53`, nodes `2,496,885`.
- `c21 black p145`: `6060.6ms -> 5644.1ms` (**`-6.9%`**), identical move `2,14`, score `-10`, nodes `3,299,678`.
- `c34 white p154`: `6022.2ms -> 5691.6ms` (**`-5.5%`**), identical move `14,11`, score `-26`, nodes `3,091,334`.
- Total across 5 slow cases: `37872.8ms -> 28297.3ms` (**`-25.3%`**).
- Correctness: `cargo fmt --check`, full `cargo test`, release builds, and `scripts/run_diff.py --profile all --check-baselines --jobs 8` (`13/13` passed) all green with zero move/score/node changes.

## 2026-11-19 — Renju center-opening case-10 LMR clamp experiment & 100-game baseline check

Context:

- Trigger: `23b72d6` vs `v0.1.13` (`edd8973`) on 12 Renjunet center openings (24 games, Renju, depth 8 / width 40, untimed, `--reuse-engine-state`) scored `11-13` (`11` splits + `1` double loss on case 10, `renjunet_center_g64692_ply3`).
- Replay analysis (`target/match-results/center24-v013.dDFkOo/review-case10/`):
  - Cold depth-8 search misses `(5,3)` on both `v0.1.13` and `23b72d6` (both pick `(5,5)` at d8/d9; `(5,3)` is found as `+19985` only at cold d10).
  - In the warm match, `v0.1.13` only found `(5,3)` because its pre-`de7ec6e` restricted-root TT cutoff skipped ply-13 d1–d6, leaving `(5,3)` as the ply-15 d7 TT hint and warming deep child TT entries under `(5,3)`. At ply-15 d8, both versions had `(5,5)` at `#0` and `(5,3)` at `#20` (0-based).
  - At `#20`, `alphabeta.rs`'s carry-accumulator `running_downf = (running_downf + index) % 15` hit `10 + 20 = 30 -> net = 2` (`depthdown = 3.0`, `attempt_depth = 5.0`), the first 3-ply reduction in the list, causing `(5,3)` (`self_attack = 3`, `atdown = 3`) to fail low without re-search.

Experiment tested (reverted):

- Minimal change in `src/search/alphabeta.rs`: at root only (`options.root && candidate.self_attack >= 3 && net > 1`), clamp `net = 1` without altering `running_downf` carry propagation.
- `center24` vs `v0.1.13` (24 games, `--reuse-engine-state`, 12 jobs): `12-12` (`WW=0, WL=12, LL=0`), case 10 recovered, `avg=1249.0ms`, `p95=5098.1ms` (`v0.1.13`: `1549.1ms / 6223.5ms`).
- Full 50 Renjunet center openings (`center_50_prefixes.jsonl`, 100 games, `cand` vs `23b72d6`, `--reuse-engine-state`, 12 jobs):
  - Overall (50 pairs): `48-48-4` (`50.00% ± 3.17%`, `WW=1` on case 10, `WD=0`, `WL/DD=47`, `LD=2` on cases 3 and 34, `LL=0`), `0` errors/timeouts/capped pairs.
  - Strictly unseen 38 holdout openings (76 games): `36-37-3` (`49.34%`, `WW=0, LD=1`), timing flat (`avg 2727.5ms vs 2763.4ms`, `p95 10607.9ms vs 10654.6ms`).
  - Only 7/50 pairs diverged, but case 3 (black 225-ply draw) produced five 51s–144s moves, pushing overall `avg_ms` from `2732.7ms` to `2905.2ms` (`+6.3%`) and `max_ms` from `61.7s` to `144.3s`. Both stop-loss thresholds fired.

100-game baseline verification (`23b72d6` vs `v0.1.13`):

- Full 50 Renjunet center openings (`center_50_prefixes.jsonl`, 100 games, Renju d8/w40, `--reuse-engine-state`, 20 jobs):
  - Overall: `48-47-5` (`50.50% ± 3.82%`, `+3.47 Elo`, CI `[-23.1, +30.1]`, `WW=1, WD=2, WL/DD=45, LD=1, LL=1`), `0` errors/timeouts/capped pairs.
  - Unseen 38 holdout openings (76 games): `37-34-5` (`51.97%`, `WW=1, WD=2, WL/DD=34, LD=1, LL=0`).
  - Timing (20 concurrent jobs): `23b72d6` `avg 3226.3ms / med 1365.3ms / p95 12322.5ms / max 65251.5ms` vs `v0.1.13` `avg 3262.3ms / med 1519.9ms / p95 12668.5ms / max 46140.2ms`.

Decision:

- Revert the root `self_attack >= 3` LMR clamp; keep `23b72d6` clean.
- Confirmed `23b72d6` has no strength regression against `v0.1.13` on 100 real Renju center-opening games while preserving faster avg/median/p95 and all `de7ec6e`–`23b72d6` correctness fixes. Future ordering/LMR experiments should target `fast` profile and gate on real center openings rather than tuning to single-case replays.

## 2026-07-03 — VCT6 slow-case legality-cache cleanup

Context:

- Target case: `target/match-results/vct_slow_probe_case.json`
  (`vct_slow_case_47_ply42`, Renju, depth 8 / width 40, VCT enabled,
  `root_vct_depth = 6`).
- Baseline was `9449d3e` after the Renju threat-legality cache. The historical
  release run was about 28.5s, but same-machine reruns varied widely, so the
  final decision used paired A/B runs against a temporary `9449d3e` worktree.

Findings:

- `ThreatBoardView::threat_moves(side)` already returns rule-legal candidates.
  Three downstream paths repeated legality checks on those same moves:
  `collect_attack_moves`, `collect_counter_wins`, and
  `collect_counter_forcing_defenses`.
- `search_defense_stage` also rechecked defenses that are only produced by
  those already-filtered attack/counter collectors.
- Removing those duplicate checks preserves the VCT tree exactly: move, score,
  alpha-beta nodes, VCT OR/AND nodes, generated attacks, generated defenses, and
  VCT memo hits matched the baseline on the slow case.
- The Renju-black legality cache is keyed by `(zobrist, move)`. On this
  VCT-heavy case, the HashMap lookup/insert path itself became hot after the
  first cache optimization. A Renju-only 64K direct-mapped cache with full-key
  validation removes hashing overhead; collisions only cause cache misses, never
  incorrect verdicts. Freestyle views do not allocate this cache.

Timed A/B:

- Clean CPU, six sequential release runs:
  base -> redundant-only -> direct-cache -> direct-cache -> redundant-only ->
  base.
- Baseline: 28.70s, 27.95s, average 28.32s.
- Redundant-only: 26.60s, 26.21s, average 26.41s (`-6.8%`).
- Redundant + direct-cache: 21.39s, 21.43s, average 21.41s (`-24.4%`).
- Final current-worktree rerun after Renju-only lazy cache initialization:
  20.60s (`-28.2%` against the first clean baseline run).
- All compared runs had identical move, score, alpha-beta nodes, VCT OR/AND
  nodes, generated attacks, generated defenses, and VCT memo hit counts.

Decision:

- Keep both duplicate-legality-check removal and direct-mapped legality cache.
  They are behavior-equivalent on the slow case and materially reduce VCT6
  wall time.

Correctness follow-up:

- The later 100-game Renju match against `v0.1.4` exposed a VCT6 root false
  positive. Repro case:
  `cases/renju/vct6_false_positive_case17_after_ply40.json`.
- The paired-match continuation is recorded in
  `cases/renju/vct6_false_positive_case17_loss_line.json`.
- On that exact board with Renju, depth 8 / width 40, VCF/VCT enabled:
  `root_vct_depth=4` and `root_vct_depth=5` do not find VCT and choose `(9,9)`
  through alpha-beta; `root_vct_depth=6` accepts VCT move `(8,8)` with score
  `20000`. In the paired match line, the VCT6 branch later lost while the
  VCT4/VCT5 lines won.
- The VCT6 branch lost at ply 114 when white played `(8,14)`, completing the
  diagonal five `(8,14)-(9,13)-(10,12)-(11,11)-(12,10)`.
- Root cause: A3 attack classification used only gain squares as defender
  replies. In this case, black `(8,8)` had gain `(6,6)`, but white `(9,9)` was
  a quiet endpoint reply: it did not occupy the gain, but it spoiled the open
  four that `(6,6)` would otherwise create. VCT therefore never searched the
  only refutation.
- Fix: A3 defenses now include each real gain plus the rule-legal winning
  completions of the open four created by that gain. Regression coverage:
  `a3_defenses_include_future_open_four_endpoints`,
  `renju_vct6_rejects_a3_quiet_endpoint_false_positive`, and
  `renju_vct6_fails_after_quiet_endpoint_refutation` in `tests/vct_alignment.rs`.
- After the fix, the same case no longer accepts `(8,8)`. VCT6 finds `(9,9)`
  instead; that matches the prior alpha-beta/VCT5 winning branch for this
  paired match. Keep the default `root_vct_depth=4` until VCT6 receives broader
  strength/performance validation.

Performance impact of the correctness fix:

- On `target/match-results/vct_slow_probe_case.json`, the same-machine
  release probe changed from about 20.60s after the legality-cache optimization
  to about 23.09s after the expanded A3 defense set. The move, score, and
  alpha-beta nodes remained the same on this probe.
- This is an expected VCT6 cost increase: the search now considers quiet
  endpoint refutations that were previously unsoundly pruned. It remains below
  the earlier 28.32s paired baseline from before the legality-cache cleanup.

100-game VCT6 match validation:

- Command family: `scripts/run_gomocup_match.py` on
  `cases/renju/strength_100_prefixes.jsonl`, Renju, depth 8 / width 40,
  VCF/VCT enabled, both sides `root_vct_depth=6`, 12 parallel jobs.
- Output:
  `target/match-results/current_a3fix_vct6_vs_v014_vct6_renju_d8w40_strength100.json`.
- Result after the A3 defense fix: current 51 wins, `v0.1.4` 46 wins, 3 draws,
  0 timeouts, 0 errors.
- Previous current-vs-`v0.1.4` VCT6 run before the fix was 49 wins, 48 losses,
  3 draws. Only two paired outcomes changed, both from loss to win; one was the
  recorded case 17 false-positive line.
- Current-side timing changed from avg 1526.735ms, median 513.303ms,
  p95 5529.782ms, max 29450.221ms before the fix to avg 1514.905ms,
  median 544.475ms, p95 5193.077ms, max 31878.314ms after the fix.

Second A3 correctness follow-up:

- The expanded A3 defense helper initially used the final legal-defense list to
  decide whether an A3 existed. That conflated two cases: no real gain exists,
  versus a real gain exists but every direct black defense is forbidden under
  Renju.
- Fix: A3 classification now returns separate state for "has a real gain" and
  "rule-legal defender replies". A white A3 whose black gain/endpoint defenses
  are all forbidden remains an A3 with an empty defense list, so VCT can still
  evaluate black counter-wins and counter-threats instead of silently dropping
  the attack.
- Regression coverage:
  `renju_white_a3_with_all_black_defenses_forbidden_stays_a3` constructs four
  black overline-forbidden defense points around a white A3 and verifies the
  attack remains `A3` with no legal direct defenses.

## 2026-06-23 Renju Forbidden Movegen Cache Gate

Commands:

- `cargo test --release --test renju_perf -- --ignored --nocapture`
- Correctness gate also included `cargo test --quiet`,
  `cargo build --bin gomoku_gui --bin gomocup_engine --quiet`,
  `python3 scripts/renju_oracle_compare.py --quiet`, and a 2000-case dense
  stress oracle compare at seed 32.

Sample:

- Dense contested midgame from `tests/renju_perf.rs`.
- `perf_legality_gate` measures direct `Board::is_legal_move_for_rule`.
- `perf_movegen_node` measures `generate_candidates` with a rule-matched eval
  cache.

Findings:

- Direct Renju legality remained expensive by design: about 736 ns/call versus
  about 2 ns/call for freestyle. External legality and protocol/GUI gates still
  use the full detector.
- Renju movegen with the SlowRenju-style shape-cache gate measured about
  1463 ns/node versus about 1267 ns/node for freestyle on this forcing sample.
- Earlier measurements on the same probe were about 1900 ns/direct legality and
  about 228 us/Renju movegen node before forbidden optimizations; after the
  one-dimensional detector scan it was about 732 ns/direct legality and
  about 77 us/Renju movegen node. The cache gate removes almost all remaining
  movegen-only forbidden overhead for non-suspicious candidates.

Decision:

- Keep the rule-aware eval cache plus SlowRenju-style movegen prefilter.
- Do not route external move validation through this cache gate; keep the full
  detector there.
- If future Rapfi-style line tables are explored, compare against this cache
  gate baseline rather than the original full-detector-per-candidate baseline.

Follow-up:

- Phase 8 added SlowRenju-style Renju black eval suppression for forbidden
  quiet points.
- On the same release probe after Phase 8, direct full detector measured about
  726 ns/call, freestyle movegen about 1244 ns/node, and Renju movegen about
  1503 ns/node.
- The suppression did not materially change the Phase 7 movegen baseline.

## 2026-04-28 Root Profile Slow-Case Sample

Commands:

- Source match JSON: `/tmp/tt_generation_reuse_smoke_quick.json`
- Generated temporary prefix cases: `/tmp/root_profile_slow_cases/*.json`
- Root profile outputs: `/tmp/root_profile_outputs_gomocup2/*.json`
- Analysis outputs: `/tmp/root_profile_slow_cases_gomocup_analysis.json` and `/tmp/root_profile_slow_cases_gomocup_analysis.md`

Sample:

- 6 representative slow moves from `smoke_quick`.
- Profiles were run with `gomocup_engine --profile base|fast` and `INFO root_profile 1`.
- This is one-shot prefix replay with empty TT, not the exact reused-TT game state.

Findings:

- 5/6 cases are `late-heavy`; final-depth late candidates consumed about 66% average time.
- 1/6 case is `PV-heavy`, where the first candidate consumed about 64% time.
- Average final-depth time was about 6.64s; max was about 9.68s in this sample.
- Simple root-width truncation looks weak: simulated width 30 saved about 157ms average and 443ms max, with no improved candidates after width 30.
- Width 20 saved more, about 505ms average and 1.44s max, but is more likely to affect strength and still does not solve 10s-class long tails.
- Idealized unordered 4-way late-candidate parallelism shows high theoretical speedup, but ordered tail parallelism is modest, about 1.17x average. This warns against repeating root-split/YBWC style work without a stronger design.
- TT old-generation hints exist in real reused-TT matches, but slow moves had lower old-generation percentage than average, so TT generation is not the current max-time root cause.

Decision:

- Do not prioritize `recompute_all` reuse; it is not visible in slow-case root profiles and is only a root-entry fixed cost.
- Do not implement TT generation ordering policy yet; keep generation as instrumentation only.
- Dynamic width may help p95 slightly, but width 30 is too weak and width 20 needs fast-vs-base strength validation.
- Next promising direction is to inspect why late candidates are expensive: fail-high/research behavior, best-move instability, and candidate ordering quality inside final depth.

## 2026-04-28 Root PVS Re-search Sample

Commands:

- Root profile outputs: `/tmp/root_profile_outputs_pvs/*.json`
- Analysis outputs: `/tmp/root_profile_pvs_analysis.json` and `/tmp/root_profile_pvs_analysis.md`

Sample:

- Same 6 representative slow moves as the previous root-profile sample.
- Profiles were run with `gomocup_engine --profile base|fast` and `INFO root_profile 1`.
- Added profile-only fields for zero-window time/nodes, full-window time/nodes, and PVS re-search flag.

Findings:

- 5/6 cases remain `late-heavy`; final-depth late candidates consumed about 66% average time.
- PVS re-search is material: 24 re-searches across 6 final-depth profiles.
- Re-search full-window time averaged about 1.62s per case, about 24% of final-depth time.
- Worst re-search full-window share was about 37.5%.
- Zero-window searches are also expensive: about 3.84s average per case. This means the long tail is not only "fail-high then re-search"; many fail-low/zero-window candidates are still costly.
- Width 30 remains weak: about 156ms average simulated saving and 444ms max saving in this sample.

Decision:

- Do not pursue a pure "avoid PVS re-search" fix; it can only address part of the tail.
- Candidate ordering and zero-window hit quality are now the main suspects.
- Next experiment should be fast-only and targeted: improve root ordering at depth 8 using prior-depth exact candidate scores or stronger same-group history, then validate fast-vs-base. Avoid changing base.

## 2026-04-28 Root Tail Cutoff Offline Simulation

Commands:

- Analysis outputs: `/tmp/root_profile_tail_cutoff_analysis.json` and `/tmp/root_profile_tail_cutoff_analysis.md`

Simulation rule:

- Final-depth profiles only by default.
- `min_depth = 8`
- `min_candidates = 20`
- `window = 10`
- `min_elapsed_since_last_improve = 800ms`
- no recent `improved`, `root_win`, `beta_cutoff`, or `pvs_research`
- `abs(best_score) < 19000`

Findings:

- Triggered in 3/6 slow-case final-depth profiles.
- All 3 simulated cutoffs were safe on this sample: no missed `improved`, `root_win`, or `beta_cutoff`.
- Saved time was modest: about 397ms average among triggered cases, 955ms max, about 4.1% average final-depth saving among triggered cases.
- It did not trigger in fast cases that were either still unstable or PV-heavy.

Decision:

- Root tail cutoff is plausible as a small fast-only experiment, but it is not a primary max-time solution.
- If implemented, keep it conservative and default off until fast-vs-base validates strength.
- Expected benefit is p95/max trimming in select late-heavy cases, not broad speedup.
- Candidate ordering remains the higher-value direction because it can reduce both expensive zero-window searches and PVS re-searches.

Follow-up:

- A fast-only final-depth cutoff prototype was tried and then removed.
- Conservative parameters changed no move/score in 6 representative cases, but only triggered 1/6 and saved about 0.6s there.
- Medium parameters triggered more often but changed one representative fast case move.
- `smoke_quick` showed only small avg/p95 improvement and worse max, so the implementation was not worth keeping.

## 2026-06-24 — Single-process search baseline and first CPU profile

Setup:

- `[profile.release]` now uses `lto = "fat"` + `codegen-units = 1`. Measured
  LTO gain on the fixed-position bench was modest (~4-5% ns/node), because the
  hot code is already inside one crate; kept because it is free. A separate
  `[profile.profiling]` (`inherits = "release"`, `debug = true`, `lto = false`)
  exists only for symbol-resolving CPU profiles.
- New `tests/renju_search_bench.rs` (`#[ignore]`d) runs a full fixed depth/width
  root search per preset prefix and reports nodes, ms, and ns/node, so per-node
  cost is separated from node count.

Baseline (depth 8 / width 40, first 4 `strength_100_prefixes`, 3 searched; one
returns immediately):

- Freestyle: 404,959 nodes, 1293 ms, **3193 ns/node**.
- Renju: 480,808 nodes, 3125 ms, **6499 ns/node**.
- So the Renju gap is ~1.19x node count but ~2.04x per-node cost: the slowdown
  is per-node work, not extra nodes. The worst single tactical prefix hit 2.9x
  per node.

CPU profile (WSL2 has no hardware PMU; sampled with software `task-clock` at
997 Hz, `--call-graph dwarf`, 4493 samples, Freestyle+Renju mixed):

| % self | function | bucket |
| --- | --- | --- |
| 18.4 | `patterns::line::shape_raw_from_board_point_hypothetical` | eval, rule-independent |
| 18.3 | `eval::local::compute_bucket_attack_and_counts` | eval, rule-independent |
| 11.6 | `eval::local::value_wide_compute_for_rule` | eval driver, rule-independent |
| 7.4 | `rules::forbidden::DirectionalLine::from_grid` | Renju forbidden only |
| 5.6 | `search::ordering::getmi` | ordering |
| 4.5 | `eval::global_eval::evaluate_board_main_cached` | leaf eval |
| 4.5 | `search::alphabeta::...::search_with_coverage` | node |
| 3.8 | `rules::forbidden::classify_placed_black` | Renju forbidden only |
| ~3.7 | slice sorts (smallsort/insertion/quicksort) | ordering |
| 1.5 | `rules::forbidden::count_four_shapes_through` | Renju forbidden only |

Interpretation:

- ~48% of total time is the rule-independent eval shape path (top 3). This is
  the freestyle headroom, not allocation or the opponent-VCF probe — both of
  which were hypothesized first and are **not** bottlenecks (`restore_snapshot`
  ~1.6%, the VCF probe does not rank).
- Renju-only forbidden detection (`DirectionalLine::from_grid` +
  `classify_placed_black` + `count_four_shapes_through`) is ~13%, matching the
  measured 2x per-node Renju gap. `DirectionalLine::from_grid` rebuilds 1-D
  line arrays per call.
- The incremental updater is already line-scoped: `value_wide_compute_for_rule`
  marks dirty cells only along the 4 lines through the move and recomputes only
  the flagged direction per dirty point (no all-4-direction redundancy). The
  cost is inherent recompute volume times per-call cost, where each call
  re-walks 10 board cells (with per-cell bounds checks) in
  `shape_raw_from_hypothetical_offsets` and re-derives buckets, rather than
  maintaining a SlowRenju-style packed 1-D line that updates incrementally.

Candidate directions (not yet implemented; measure each against the bench):

1. Micro-opt the hypothetical line walk: precompute clamped step counts once to
   drop per-cell bounds branches; avoid rebuilding the mask arrays per call.
2. Renju: cache/incrementally maintain `DirectionalLine` instead of rebuilding
   it per `classify_*` call (same family as the deferred covered-point scan).
3. Larger/riskier: a packed 1-D incremental line cache to avoid re-walking 10
   cells per point per update; biggest payoff, biggest rewrite.

### #1 result — hypothetical line-walk micro-opt (low yield, kept)

`shape_raw_from_hypothetical_offsets` was rewritten to precompute the on-board
step count (`ray_steps`) and walk only the on-board prefix, dropping the
per-cell bounds branch and the rebuilt mask arrays (edge is treated as the
blocking cell, equivalent to the old off-board `SENTINEL`). All tests pass.

A/B on the bench (release+LTO, deterministic node counts, 3 runs each):

- Freestyle 3193 -> ~3177 ns/node (~0.5-1%, within run-to-run noise).
- Renju 6499 -> ~6370 ns/node (~2.0%, stable across runs).

Re-profile (profiling build, same conditions) shows `shape_raw` essentially
unchanged at 18.4% -> 18.1% self. Conclusion: the bounds branches were already
neutralized by the optimizer (predictable, in-bounds), so the cost inside
`shape_raw` is memory loads + the shape-table lookup + sheer call volume, not
the boundary arithmetic. The change is kept (real ~2% Renju, cleaner code, no
risk) but it disproves inner-loop micro-opt as the way to close the freestyle
gap. The remaining levers are reducing recompute *volume* (Renju covered-region
scan, direction #2) or the packed 1-D line rewrite (#3); freestyle ~3.2 us/node
is close to where this re-walk-per-point architecture lands without #3.

### #3 Stage 0 — flatten the shape table (no measurable gain, kept)

`SHAPE_TABLE` was changed from `LazyLock<Vec<Vec<i32>>>` to
`LazyLock<Box<[[i32; 3969]; 2]>>` (contiguous, single deref + flat index,
plus load-time dimension asserts). Semantics identical by construction; full
suite green. Bench A/B (release+LTO): Freestyle ~3193 -> ~3170 ns/node, Renju
~6370 -> ~6440 ns/node — both within run-to-run noise, i.e. **no measurable
gain**. So the `Vec<Vec>`/`LazyLock` indirection was not a bottleneck either
(the table is hot and effectively cached). Kept for cleaner code; not a
regression.

### #3 Stage 1 — de-risk the walk (positive: 4.7x ceiling)

WSL2 has no branch-miss PMU, so the EMPTY-skip-branch hypothesis was tested
directly with a throwaway micro-bench (`tests/shape_reader_microbench.rs`, not
committed): the current data-dependent +/-5 walk vs. a branchless bitmask reader
that builds forward/backward blocker+own masks, derives `si`/`sj` via
`trailing_zeros`, and `ssp` via masks + a 5-bit reversal LUT, producing the
**identical** `table_index`.

- Correctness: 0 mismatches over 20,000 random windows (incl. simulated edges).
- Timing: walk 19.19 ns/call vs branchless 4.07 ns/call = **4.72x**.

Unlike #1 and Stage 0, the walk's branches are a real cost, so the gate is
cleared. The de-risk also shows the win comes from the branchless *computation*,
not from incremental line maintenance: the branchless reader reads the same
window. Stage 2 is therefore a localized, lower-risk rewrite of
`shape_raw_from_hypothetical_offsets` (same cells in, bit-identical shape out),
with no new `EvalCaches` state or snapshot/restore changes. Gated by an
exhaustive window-identity test plus the existing eval/movegen/root alignment
and incremental-vs-full suites.

### #3 Stage 2 — branchless bitmask shape reader (real win, kept)

`shape_raw_from_hypothetical_offsets` (src/patterns/line.rs) was rewritten to
build forward/backward blocker+own bitmasks over offsets 1..=5 via a fixed
5-iteration `directional_masks` (off-board = blocker, no data-dependent
early-exit), derive `si`/`sj` with `trailing_zeros`, and `ssp` via masks + a
const `REVERSE5` 5-bit-reversal LUT. This produces the **identical**
`table_index`, so every shape value is unchanged. No `EvalCaches`/snapshot
changes were needed — the win is in the computation, not in maintaining a packed
line.

Equivalence gate: the existing
`point_shape_reader_matches_full_line_extraction_on_varied_boards` already
drives this path against the full-line reference; a new
`hypothetical_reader_matches_full_line_on_random_and_edge_boards` extends it to
120 random/dense/edge boards over every empty point, direction, side, and rule
(0 mismatches). Full eval/movegen/root alignment and incremental-vs-full suites
stay green unchanged.

Bench A/B (release+LTO, deterministic node counts, 3 runs):

- Freestyle ~3172 -> ~2994 ns/node (**~5.6%**).
- Renju ~6440 -> ~6138 ns/node (**~4.7%**).

Re-profile (profiling build) shows `shape_raw` self-cost dropping 18.4% -> 16.6%
and no longer the top function; the larger release wall-clock gain comes from LTO
inlining `directional_masks`. This is the first real win in the thread and lands
on freestyle. Remaining top costs are now `compute_bucket_attack_and_counts`
(~18-19%, pure classifier called per dirty point/side; reduce call count rather
than the function) and, for Renju, `DirectionalLine::from_grid` (~8%, rebuilt per
detector call — direction #2).

### Renju forbidden detector — build the 4 lines once (Renju win, kept)

`classify_placed_black` (src/rules/forbidden.rs) rebuilt a `DirectionalLine` in
each of its checks (`has_exact_five`, `has_overline`, `count_four_directions`,
`count_true_open_three_directions`), up to ~20 `from_grid` calls per
classification of one point. It now builds the four lines once
(`std::array::from_fn`) and reuses them across all checks (~20 -> 4). `from_grid`
is a pure function of (grid, x, y, dir), and the recursive gain probe restores
`grid` before each subsequent check, so this is bit-identical; the 72 forbidden
fixtures plus the Rapfi/`renju_forbid` alignment suite and full test suite stay
green. The test-only free fns `has_exact_five`/`has_overline`/
`count_four_shapes_through` were `#[cfg(test)]`-gated.

Bench A/B (release+LTO, 3 runs): Freestyle ~2994 -> ~2949 ns/node (noise; the
detector is Renju-only) and Renju ~6138 -> ~5689 ns/node (**~7.3%**). Re-profile
shows `DirectionalLine::from_grid` dropping from ~8.4% (was 4th) out of the top
8, with `classify_placed_black` and `count_four_shapes_through` also leaving it.

Session totals on the bench: Freestyle 3193 -> ~2949 ns/node (~7.6%), Renju
6499 -> ~5689 ns/node (~12.5%). Remaining top costs are the eval core
`shape_raw` (~18%) and `compute_bucket_attack_and_counts` (~17%, dominated by the
Renju 225-point apparent-double-three refresh scan — the next candidate, needs an
incrementally maintained candidate set to cut call count without changing
semantics).

### Renju refresh — incremental apparent-double-three set (large Renju win, kept)

`refresh_renju_apparent_double_three_points` (src/eval/local.rs) scanned all
empty points every Renju make, calling `compute_bucket_attack_and_counts` as a
per-point filter (`exact_fives == 0 && open_threes >= 2`) to find the few
apparent double-threes it must re-evaluate for non-local forbidden flips. That
per-make ~225-point scan dominated `compute_bucket_attack_and_counts` (~17%).

Membership is a pure function of a point's own (local) black shapes, so it is now
maintained incrementally: `EvalCaches` gains `dt3_present` (authoritative
membership) and an append-only `dt3_members` list, updated in the make dirty
loop wherever a point's black shapes change (`refresh_dt3_membership`). The
refresh iterates `dt3_members` (skipping stale entries via `dt3_present`) instead
of scanning the board. Snapshot/restore reverts membership via a `dt3_present_log`
journal (transitions) plus `dt3_members` length truncation — chosen over a
by-value `dt3_present` snapshot because the latter cost a 225-byte copy per node
and regressed Freestyle ~2.6%; the journal stays empty in Freestyle (which never
touches the set) for zero overhead.

Correctness: bit-identical — the same point set is re-evaluated the same way.
Gated by the 400-trial `renju_incremental_eval_matches_full_recompute_over_random_sequences`
stress test (value/attack caches vs full recompute) and a new
`renju_snapshot_restore_reverts_caches_including_dt3` test that asserts the whole
cache (dt3 included) reverts after snapshot -> make -> restore. Full suite green.

Bench A/B (release+LTO, 3 runs): Renju ~5689 -> ~4588 ns/node (**~19.4%**);
Freestyle unchanged (~2949 -> ~2993, within noise — the set is Renju-only and the
journal adds no Freestyle cost). Re-profile shows
`compute_bucket_attack_and_counts` halving from ~17% to ~9.7% and leaving the
number-two slot.

Session totals on the bench: Freestyle 3193 -> ~2993 ns/node (~6.3%), Renju
6499 -> ~4588 ns/node (**~29%**). The remaining top cost is the rule-independent
`shape_raw` (~18-19%) reader; further gains there need the bitboard line model,
not call-count reduction.

## 2026-07-02 — VCT-path profile after Renju tactical fixes

Context: after the Renju VCF/VCT correctness fixes (false-positive A4 instant
win, four-vs-open-four refutation, forbidden-block VCF win, `b4p` 0x1D jump
four), a same-position sequential bench over 88 match-derived Renju positions
showed the fixed engine ~16% slower per move than the pre-fix baseline (avg
~601 ms vs ~516 ms, release build). This profile localizes that cost.

Setup:

- Profiling build (`--profile profiling`, no LTO), `renju_search_bench`
  (`BENCH_POSITIONS=6`, depth 8 / width 40, Freestyle+Renju mixed).
- `perf record -F 997 -e task-clock --call-graph dwarf`, 4494 samples.

Findings (flat self%):

| % self | function | bucket |
| --- | --- | --- |
| 16.07 | `patterns::line::shape_raw_from_board_point_hypothetical` | eval, rule-independent |
| 9.32 | `eval::local::value_wide_compute_for_rule` | eval driver |
| 7.70 | `eval::local::compute_bucket_attack_and_counts` | eval classifier |
| 7.10 | `threats::threat_board::ThreatBoardView::threat_moves` | VCF/VCT candidate gen |
| 4.83 | `search::ordering::getmi` | ordering |
| 4.58 | `eval::global_eval::evaluate_board_main_cached` | leaf eval |
| ~8 (sum) | `rules::forbidden::*` (`count_four_shapes_through`, `classify_placed_black`, `black_run_bounds_with_extra`) | Renju forbidden |
| 1.07 | `ThreatBoardView::legal_win_completions` | new rule-aware check |

Interpretation:

- Two hypotheses are ruled out. The VCF memo key's per-node `Vec` alloc + sort
  does not appear at all (below the 0.8% floor), matching the 2026-06-24 finding
  that allocation is not a bottleneck. The new rule-aware tactical checks added
  by the fixes are cheap: `legal_win_completions` 1.07%, `has_a4` 1.13%,
  `has_winning_a4`/`four_has_rule_legal_completion` below the floor.
- VCT itself barely runs on the strength-prefix sample (`or_node` 0.27%
  cumulative), so this sample does not reproduce the +16%, which was measured on
  VCT-heavy match positions. The +16% is therefore attributed to the extra
  search nodes from the four-vs-open-four correctness fix (a real work increase,
  not per-node cost), which cannot be optimized away without reintroducing the
  bug. `threat_moves` (7.1%, re-scans the board per VCF/VCT node) is the largest
  single tactical-path item and would be the target if a VCT-heavy profile later
  justifies it.
- General-search top costs are unchanged from 2026-06-24: the rule-independent
  eval core (`shape_raw` + `value_wide` + `compute_bucket` ~33%) and Renju
  forbidden detection (~8%). Both are known; the eval core needs the bitboard
  line model.

Decisions:

- Drop the VCF memo-key optimization (non-bottleneck; see decision rules above).
- Reverted the A1 cache micro-opt (caching the defender-four existence across
  the OR-node attack loop) back to the simple per-attack
  `a4_attack_wins_immediately`: the profile shows `or_node` is not hot, so the
  cache had no measurable benefit, and the simple form is obviously correct
  without a cross-iteration-invariance argument. Behavior-equivalent: vcf/vct/
  root alignment unchanged, `root_alignment` nodes identical.
- Accept the ~16% Renju single-move overhead as the correctness cost of the
  four-vs-open-four fix.

### 2026-11-19: Long-tail speedup experiments after `eb58de8` (128 MiB TT, `fast` RFP/Futility, `fast` quiet LMR) — all reverted

Context: after the 100% behavior-equivalent single-thread speedups (`215a896`,
`aab2e69`, `eb58de8`, ~1.88x cumulative vs `23b72d6`), fixed-depth d8/w40 Renju
center-opening games (`center_50_prefixes.jsonl`, 100 games, `--reuse-engine-state`)
still had occasional 15s–36s single-move long tails (`base_eb58de8`: avg
`1575.2ms`, median `740.9ms`, p95 `5998.8ms`, max `35990.0ms`). Three
single-thread long-tail levers were tested and all rejected:

1. **128 MiB TT (`TranspositionTable::new(22)` vs default 32 MiB `new(20)`)**:
   - Hypothesis: larger TT reduces mid/late-game collisions on 15M–21M-node d8
     long tails.
   - Measured (`compute_vct=0`, d8/w40, single-thread release):
     - `c43_p32`: `-7.9%`
     - `c25_p68`: `-4.5%`
     - `c34_p154`: **`+28.2%`**
     - `c10_p15` (normal midgame): **`+6.0%`**
   - Decision: reverted. L3 cache thrashing outweighs collision reduction.

2. **`fast`-only Shallow Reverse Futility Pruning (`depth <= 2.0`, margin `35/65`) + Quiet Futility Pruning (`depth <= 1.0`, margin `40`)**:
   - Note on eval scale: `rust_gomoku`'s `drift = 18.88`, `atdown3 = 2.35`,
     `atdown4 = 3.67`, normal midgame eval `[-50, +50]`. Initial `120/220` +
     `140` margins barely triggered (`-1.9%` to `+0.3%`). Rescaled `35/65` + `40`
     (gated on `EngineProfile::Fast`, non-root, zero-window, `!win_priority`,
     `!single_forcing`, `!hostile_threat`, `total.abs() < 32_768.0`).
   - Fixed-position d8/w40 (`compute_vct=0`): `c43_p32 -23.5%`, `c25_p68 -10.7%`,
     `c34_p154 -7.6%`, `c10_p15 -18.9%`, all same best move.
   - 100-game Renju `center_50_prefixes.jsonl` match (`fast_rfp35_65_fut40` vs
     `base_eb58de8`, d8/w40, `--reuse-engine-state`, 0 errors / 0 timeouts):
     - Strength: **`47-52-1` (`47.50%`, `-17.39 Elo`, 95% CI `[-72.2, +36.6]`,
       pentanomial `WW=6, WD=1, WL/DD=34, LD=0, LL=9`)** — failed `>= 50%` gate.
     - Timing: `fast_rfp` was **slower** overall (`avg 1670.5ms`, `median 770.5ms`,
       `p95 6229.3ms`, **`max 82313.2ms`**) vs `base_eb58de8` (`avg 1575.2ms`,
       `median 740.9ms`, `p95 5998.8ms`, `max 35990.0ms`).
   - Decision: reverted. Dropping shallow nodes in Gomoku misses tactical
     counter-attacks and can steer games into worse/slower endgames.

3. **`fast`-only Quiet LMR (`+1` at `index >= 3, depth >= 3.0`; `+2` at `index >= 10, depth >= 5.0`) with existing full-depth re-search (`score > alpha && net > 0`)**:
   - Gated on `EngineProfile::Fast`, `found_pv`, `!options.root`, `!win_priority`,
     `!single_forcing`, `!hostile_threat`, `self_attack == 0 && opp_attack == 0`,
     non-killer.
   - Fixed-position d8/w40 (`compute_vct=0`):
     - `c2_p104`: `65840.9ms -> 77004.4ms` (**`+17.0%`**)
     - `c43_p32`: `34074.8ms -> 35755.4ms` (**`+4.9%`**)
     - `c25_p68`: `23437.2ms -> 22719.7ms` (`-3.1%`)
     - `c10_p45`: `20989.1ms -> 11925.0ms` (`-43.2%`), **but selected `(7,10)`**,
       which exact d8 proves is a forced loss (`-19987`) vs saving move `(5,10)`.
     - `c34_p154`: `-32.1%`, `c10_p15`: `-32.7%`.
   - Decision: reverted. Re-searches + shallower TT pollution slowed the worst
     long tails (`+17.0%`), while hiding tactical refutations (`c10_p45`).

Long-tail breakdown finding (`compute_vct=1` vs `compute_vct=0` + `--root-profile`):

- On the 4 slowest moves from the 100-game match (`c2 p104` 68.3s, `c10 p46`
  20.6s, `c10 p45` 21.0s, `c2 p71` 16.3s), **96%–100% of elapsed time is
  Alpha-Beta** (VCT is only `0%–3.9%`).
- Long tails split into (a) real tactical rescue re-searches (e.g. `c10 p45`:
  d7 `0.47s` → d8 discovers d7 PV `(7,10)` loses `-19987` and PVS-researches
  saving candidates), and (b) wide zero-window fail-low trees (e.g. `c2 p104`:
  89% of time spent on root `index=1..19` fail-lows). Single-thread heuristic
  pruning consistently loses strength or regresses worst-case tails; further
  major long-tail reductions require either shared lock-free parallel Alpha-Beta
  (e.g. ABDADA under an experimental path) or time-budgeted iterative deepening.

### 2026-11-19: `fast` vs `base` re-evaluation on `eb58de8` and removal of `fast` profile / `fast_history_ordering`

Context: after `215a896` + `aab2e69` added lazy tie-group `getmi` ordering and
TT-best-move deferred primary sorting to the default `base` ordering path,
`EngineProfile::Fast` differed from `Base` only by `fast_history_ordering = true`,
which still eagerly computed `getmi` and fully sorted candidates on every node.

100-game Renju `center_50_prefixes.jsonl` match (`fast_eb58de8` vs `base_eb58de8`,
d8/w40, `--reuse-engine-state`, 20 jobs, 0 errors / 0 timeouts, 0 capped pairs):

- **Strength**: `47-49-4` (**`49.00%`**, `-6.95 Elo`, 95% CI `[-49.8, +35.7]`,
  pentanomial `WW=3, WD=3, WL/DD=38, LD=1, LL=5`) — below the `>= 50%` point-estimate gate, with no statistically resolved strength difference.
- **Timing** (2331 moves each):
  - `base_eb58de8`: avg **`1616.1ms`**, median **`802.8ms`**, p95 **`6216.6ms`**, max **`27736.9ms`**
  - `fast_eb58de8`: avg `2093.4ms` (**`+29.5%`**), median `1019.3ms` (**`+27.0%`**), p95 `8221.6ms` (**`+32.3%`**), max `36445.3ms` (**`+31.4%`**)

Decision: removed `EngineProfile` (`fast` profile) and `fast_history_ordering`
(killer/history tables, CLI flags, Gomocup `INFO` toggles, and desktop and
Android selectors). The former Fast profile took 29.5% longer per move in
this match without demonstrating a strength gain.

### 2026-11-20: Multi-threaded ABDADA shared-TT Alpha-Beta (`threads > 1`)

Motivation & design:

- Existing `TranspositionTable` (`src/search/tt.rs`) already uses lock-free
  `AtomicU64` XOR-hashed buckets behind `Arc<TTStorage>`.
- Implemented ABDADA (`src/search/abdada.rs`, `src/search/alphabeta.rs`,
  `src/search/root.rs`):
  - Lock-free 512 KiB `AbdadaTable` (65,536 `AtomicU64` slots with per-depth
    generation salt, zero clearing cost), touched only when `depth >= 3.0`
    (`ABDADA_MIN_DEPTH = 3.0`; `2.0` was tested and slightly slower at `4T`:
    `1.71x` vs `1.82x` due to shallow atomic traffic).
  - Persistent `SearchWorkerPool` on `RootSearcher`, woken only at `depth >= 5`
    and disabled when `node_limit.is_some()` or `threads == 1` (`threads == 1`
    takes the exact original single-threaded path with zero overhead).
  - Shared root best `(alpha, move)` (`SharedRootBest`) publishing only strict
    exact improvements (`score > alpha`); zero-window fail-lows (`score == alpha`)
    never overwrite `best_move` (an intermediate prototype that broke ties on
    `score == alpha` lost `42-56-2` / `-49.0 Elo` and was immediately fixed).

Fixed-position d8/w40 (`compute_vct=0`, 5 slow tails sum = `73.35s` at `1T`):

- `2T`: `49.24s` (`1.49x`)
- `4T`: `38.68s–40.40s` (`1.82x–1.90x`)
- `8T`: `33.12s–34.55s` (`2.12x–2.21x`, up to `3.13x` on `c24_p155` and `2.97x` on `c43_p32`)
- Normal midgame `c10_p15` (`640.5ms`): `4T 347.9ms` (`1.84x`), `8T 303.4ms` (`2.11x`).

100-game Renju `center_50_prefixes.jsonl` matches (`--reuse-engine-state`, 0 errors / 0 timeouts / 0 capped pairs across all 600 games):

| Matchup | Mode | W-L-D | Paired Score | Speedup vs `1T` (`avg / med / p95 / max`) |
| --- | --- | --- | --- | --- |
| `4T + 32 MiB` vs `1T + 32 MiB` (v1) | d8/w40 | `48-50-2` | `49.0%` | `1.99x / 2.21x / 1.94x / 1.38x` |
| `4T + 32 MiB` vs `1T + 32 MiB` (v3) | d8/w40 | `47-52-1` | `47.5%` | `1.93x / 1.93x / 2.02x / 1.33x` |
| `4T + 64 MiB` vs `1T + 32 MiB` | d8/w40 | `49-48-3` | `50.5%` (`+3.5 Elo`) | `1.97x / 2.07x / 2.06x / 2.16x` |
| `8T + 64 MiB` vs `1T + 32 MiB` | d8/w40 | `55-41-4` | `57.0%` (`+49.0 Elo`) | `2.37x / 2.39x / 2.30x / 2.61x` |
| `8T + 128 MiB` vs `1T + 32 MiB` | d8/w40 | `47-53-0` | `47.0%` | `2.16x / 2.00x / 2.16x / 1.53x` |
| **Combined 500 fixed-d8 games** | d8/w40 | **`246-244-10`** | **`50.20%`** | **`~2.0x–2.4x`** |
| `8T + 64 MiB` vs `4T + 64 MiB` | d8/w40 | `47-49-4` | `49.0%` | `8T` faster by `7%` avg, `12%` p95, `36%` max |
| `4T + 32 MiB` vs `1T + 32 MiB` | **1s/move timed** | **`58-41-1`** | **`58.5%` (`+59.6 Elo`, `WW=12, LL=4`)** | median `230ms vs 687ms` (`3.0x` faster) |

Decision:

- Keep library and Gomocup default at `threads = 1` + `32 MiB TT` (`tt_bits = 20`)
  so `base` remains 100% deterministic and passes all 13/13 reference diffs.
- Auto-tier default TT to `64 MiB` (`tt_bits = 21`) when `threads >= 4` and no
  explicit `tt_bits` is set (`128 MiB` slowed `8T` slightly due to L3/TLB pressure).
- Enable interactive default `default_interactive_threads()` (`4` on `>= 4` cores
  with `64 MiB TT`, `min(2, cores)` on `< 4` cores with `32 MiB TT`) in desktop
  GUI (`gomoku_gui`) and Android JNI bridge.

### 2026-11-21: Route B — Incremental 4-direction `Pattern4` bitboard/LUT for `ThreatBoardView` / VCT / VCF (`feat/threat-pattern4-bitboard`)

Hotspot evidence & target:

- Prior VCT/VCF profiles (`2026-07-02` + dense late-game Renju probes) showed
  `ThreatBoardView::threat_moves`, `collect_attack_moves`, `first_counter_win`,
  and `collect_counter_forcing_defenses` running ~100 quiet trial `play/undo`
  cycles per OR/AND node. That trial-play dominance is also why earlier
  `play()`-only incremental bitsets regressed (`+52.7%`).
- Target: achieve a major behavior-equivalent speedup on dense VCT6 positions
  (`cases/renju/vct_dense_ply78.json`, `cases/renju/vct_counter_win_scan_ply75.json`,
  `target/match-results/vct_slow_probe_case.json`) without altering moves,
  scores, or reference diff baselines.

Modification scope:

- Added `src/threats/pattern4.rs`: 88 padded 2-bit-per-cell `u64` lines, a
  59,049-entry (`3^10`) ternary LUT for exact ±5 1D patterns (`Win5`, `Overline`,
  `A4`, `DoubleFour1D`, `B4` + exact reply offset, `A3` + exact gain/completion
  offsets with multi-window ambiguity fallback), column-major `BitBoard225`
  threat masks (`win5_bits`, `a4_bits`, `b4_bits`, `a3_bits`), and O(1)
  `fast_rule_legal_empty` (calling `rule_legal_cached` only on Renju Black
  `maybe_33` candidates).
- Updated `src/threats/threat_board.rs`, `src/threats/vct.rs`, and
  `src/threats/vcf.rs`: eliminated quiet trial plays in `first_counter_win`,
  `broken_four_point_for_side`, `collect_attack_moves` (including zero-`play/undo`
  `try_a3_defense_info_fast` with slow fallback on orthogonal interaction or
  multi-window lines), `winning_threat_moves`, `forcing_threat_moves`,
  `has_vct_trigger_for_rule`, `vct::collect_counter_forcing_defenses`,
  `vct::a4_attack_wins_immediately`, `vct::search_defense_stage` instant
  refutations, and `vcf::search_attacker`; replaced per-AND-node `HashSet<Move>`
  with stack `MoveSet225` and removed unused `adjacency` tracking from
  `ThreatBoardView`.
- Added fixture `cases/renju/vct_dense_ply78.json` (78-ply dense Renju game).

Net-benefit hypothesis & stop-loss:

- Added cost: incremental ±5 neighbor LUT updates on real tree `play/undo`
  (4 axes × ≤10 empty cells, zero-allocation bit shifts + table lookups).
- Saved cost: eliminates ~20x–30x quiet trial `play/undo` calls per node,
  per-node board scans, per-AND-node `HashSet<Move>` allocations, and repeated
  Renju forbidden checks on non-`3-3` squares.
- Stop-loss: revert if net VCT6 speedup < `2x` or if any fixed case / 13-case
  reference diff regressed.

Correctness & equivalence verification:

- `cargo fmt --check`, `git diff --check`: clean.
- `cargo test` + `android/rust_bridge` tests: all passed, including new random
  oracle tests (`incremental_play_undo_matches_fresh_rebuild_for_both_rules`,
  `pattern4_win5_a4_b4_and_b4_reply_match_threat_board_oracle` [2,000 boards],
  `fast_a3_defense_info_matches_slow_oracle_for_both_rules` [1,600 boards], and
  `counter_forcing_defenses_match_legacy_trial_oracle_for_freestyle` [400 boards]).
- `python3 scripts/run_diff.py --profile all --check-baselines --jobs 8`: **13/13
  passed**.

A/B VCT6 timing comparison (`case_probe --rule renju --depth 1 --width 10`,
release build, 3-run median vs `master` `cbd0a7c`):

| Case | Baseline (`cbd0a7c`) | Route B (median) | Speedup | Move / Score / VCT Stats |
| --- | ---: | ---: | ---: | --- |
| `cases/renju/vct_dense_ply78.json` | `1555.0 ms` | **`166.6 ms`** | **`9.33x`** | **100% bit-identical** (`[10,10]`, `68`, `or: 164,384`, `and: 252,506`, `atk: 272,226`, `def: 252,506`, `memo: 37,047 / 0 / 19,720`) |
| `cases/renju/vct_counter_win_scan_ply75.json` | `2659.1 ms` | **`339.6 ms`** | **`7.83x`** | **100% bit-identical** (`[7,1]`, `24`, `or: 385,483`, `and: 511,034`, `atk: 526,615`, `def: 511,802`, `memo: 61,702 / 18 / 2,713`) |
| `target/match-results/vct_slow_probe_case.json` | `3491.9 ms` | **`525.3 ms`** | **`6.65x`** | `move [8,13]`, `score 59`, `and: 648,797`, `memo: 58,537 / 838 / 25,222` identical; `or/def` `-24` (`478,893` vs `478,917`) from excluding 24 fake Renju-Black overline fours previously miscounted as `counter_b4` |
| **3-case total** | **`7,706.0 ms`** | **`1,031.5 ms`** | **`7.47x`** | — |

Note on untimed Master `d8/w40` on `vct_dense_ply78.json`: VCT6 is now `~165–184 ms`
(`0.3%` of total), while uncapped `d8/w40` Alpha-Beta itself still takes `67.6s`
(`1T`) / `48.2s` (`4T`, `208M` nodes). In timed mode (e.g. `2s`/move), full VCT6
now completes inside `~0.17s` instead of hitting the 30% sub-budget cap.

### 2026-11-21 (2): Fix multi-threaded ABDADA fractional-depth TT miss & worker collision (`feat/abdada-multithread-scaling`)

Hotspot evidence & root cause:

- On `cases/renju/vct_dense_ply78.json` (`d7/w40` & `d8/w40`), `4T` ABDADA
  previously suffered **`2.99x` node inflation** (`d7`: `45.9M` vs `15.3M`,
  `d8`: `208.4M` vs `71.5M`), yielding only `1.39x–1.40x` speedup (`67.8s -> 48.5s`).
  Increasing TT from `64 MiB` to `128 MiB` barely moved node count (`45.79M`),
  ruling out TT capacity overflow.
- Root cause: `AlphaBetaSearcher` stored `depth as i32` (floor) and probed
  `depth.ceil() as i32` (ceil). Because `base_depthdown` is fractional whenever
  `case_count < wide`, almost every deep tactical subtree had a non-integer
  `depth` (e.g. `5.4`), storing `5` and probing `6` — making it **impossible**
  for ABDADA workers returning in `pass == 1` to hit the TT entry just written
  by another worker. In addition, `AbdadaTable` ignored depth in its busy key
  and all workers iterated `pass == 1` deferred moves in identical order.

Modification scope:

- `src/search/alphabeta.rs`: for multi-threaded searches (`threads > 1`) only,
  quantize TT store & probe depths consistently in 1/8-ply fixed point
  (`((depth * 8.0).floor() as i32).clamp(1, 200)` for `depth > 0.0`, `0` for
  leaves, `+40` win/loss bonus); mix 1/8-ply depth into `AbdadaTable` busy keys;
  rotate `pass == 1` deferred move order by `worker_id`. Single-threaded (`1T`)
  path is 100% unchanged.
- `src/search/abdada.rs`, `src/search/root.rs`, `src/search/tt.rs`: pass
  `worker_id` into `AbdadaContext`; add `TranspositionTable::clear()` and clear
  TT if a `RootSearcher` switches between `threads == 1` and `threads > 1`
  mid-game so integer and 1/8-ply TT entries never mix.

Correctness & equivalence verification:

- `cargo fmt --check`, `git diff --check`: clean.
- `cargo test` + `android/rust_bridge` tests: all passed.
- `python3 scripts/run_diff.py --profile all --check-baselines --jobs 8`: **13/13
  passed** (`1T` `base` 100% unchanged).

Fixed-position `cases/renju/vct_dense_ply78.json` (`d8/w40`, Renju):

- `1T` baseline: `move [8,1]`, `score 29`, `71,533,289` nodes, `67,822 ms`
- Old `4T` (`031053e`): `move [8,1]`, `score 29`, `208,386,127` nodes, `48,453 ms` (`1.40x`)
- New `4T`: `move [8,1]`, `score 29`, **`9,235,714` nodes**, **`4,034 ms`** (**`16.8x` vs `1T`, `12.0x` vs old `4T`**)
- New `8T`: `move [8,1]`, `score 29`, **`11,400,102` nodes**, **`2,676 ms`** (**`25.3x` vs `1T`**)

100-game Renju `center_50_prefixes.jsonl` match (`4T_subply_abdada` vs `1T_base`,
`d8/w40`, `--reuse-engine-state`, `--jobs 5`, 0 errors / 0 timeouts / 0 capped pairs):

- **Strength**: **`51-46-3`** (**`52.50%`**, **`+17.39 Elo`**, 95% CI `[-37.4, +73.0]`,
  pentanomial `WW=8, WD=3, WL/DD=32, LD=0, LL=7`) — passes `>= 50%` gate.
- **Timing** (`2,289` vs `2,287` moves):
  - `1T_base`: avg `1,283.5 ms`, median `564.8 ms`, p95 `5,010.1 ms`, max `35,390.7 ms`
  - `4T_subply_abdada`: avg **`431.7 ms` (`2.97x`)**, median **`204.0 ms` (`2.77x`)**,
    p95 **`1,677.9 ms` (`2.99x`)**, max **`15,051.6 ms` (`2.35x`)** (previous `4T`
    was `~1.97x–2.16x` across avg/med/p95/max).
