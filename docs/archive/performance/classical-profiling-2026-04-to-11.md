# Classical search and early profiling

> Snapshot from the former [perf-log.md](../../perf-log.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
