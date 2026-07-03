# Performance Log

## Current Performance Decision Rules

Do not repeat these directions unless new evidence invalidates the decision:

- `recompute_all` reuse: deprioritized. It is a root-entry fixed cost and has not appeared in slow-case root profiles.
- TT best-move trust or generation ordering: deprioritized. Old-generation hints exist, but slow moves had lower old-generation percentage than average.
- Shape reader LUT, indexed neighbor reader, and batch-lines: rejected as default paths after semantic-equivalent tests showed unstable or worse performance.
- Lazy SMP, root YBWC, root split, and aspiration window: rejected as default paths after poor speed/strength or maintenance tradeoffs.
- Early stop and broad dynamic pruning: only revisit with a concrete tactical safety proof and fast-vs-base validation.
- VCF memo-key allocation and VCT/VCF as a per-node hotspot: confirmed non-bottleneck by the 2026-07-02 VCT-path profile (`or_node` 0.27% cumulative, the `(Side, Vec<Move>, Vec<Move>)` memo alloc + sort below the 0.8% floor). Do not pursue memo-key micro-optimization.

Current active target:

- Final-depth late root candidates. Future analysis should explain why they are expensive, not re-confirm that they are expensive.

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
