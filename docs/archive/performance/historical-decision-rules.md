# Earlier performance decision rules

> Snapshot from the former [perf-log.md](../../perf-log.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
- NNUE P4 experiment records (2026-11-24, branch `exp/p3-search-strength`, audited):
  - Bundled Step 1 (`p4_step1`: removing `abdada_table` deferral inside 4T Lazy SMP + disabling Root/PV TT cutoffs + `threadBias` + `max_depth=64`): lost `42-50-8` (`46.0%`, `-27.85 Elo`) vs `p4_fix3_4t` with `+8.7%` avg move time (`507.2ms -> 551.2ms`). Note: `tt_generation` counts all `best_move_hint` hits (`alphabeta.rs:699`) and cannot by itself separate duplicate work from useful sharing; because 4 changes were bundled, do not re-bundle and isolate any single item if revisited.
  - Bundling 7 speculative search heuristics (`p4_step2ab` / `v2`: `Alpha-Improvement Reduction` without aspiration window, `OPPO5_EXT`, uncalibrated Policy Pruning/LMR, PV LMP, `TTM_EXT`): scored `27.0%`/`28.0%` vs Rapfi 1T with `fast_mate` losses rising from `40.0%` to `52.1%–53.4%`. Never bundle unverified search heuristics.
  - Step 3.1 (`p4_step31`, archived for later single-item split verification; NOT merged as a bundle): median single-core speed improved (`c4_ply20` `118.05ms -> 109.86ms`, `loss4t_c9_ply22` `51.74ms -> 45.29ms`), and under true untimed fixed `d8/w40` (`1T`, `32 MiB TT`) both `p4_fix3` and `p4_step31` matched identically (`c4_ply20` `tt_generation current=750`, `27463` nodes; `loss4t_c9_ply22` `current=1520`, `13943` nodes; plus 10 match prefixes at `d8/d10 w40`). Correction: the earlier `733 -> 1319` reading came from omitting `--depth 8` while sending `INFO timeout_turn 0`, which ran a `200ms` timed search (`max_depth=25`, not `d=8`). Still kept out of `p4_base` because 4T vs Rapfi dropped `32.0% -> 26.0%` (paired bootstrap 95% CI `[-11%, -1.5%]`) and 1T self-play was `41-47-12` (`47.0%`). Fixed-depth gates must use `case_probe` or `--depth 8 --width 40` without clock `INFO` (never `INFO timeout_turn 0`, which parses as `200ms`).
  - `p4_fix2` (`dc23d00`, `self_last`-salted QVCF TT) and `p4_fix3` (`f9e92b0`, SE `skip_move` exclusion): conservatively reverted from `p4_base` (`4065c80` = `45f46bb` + `812a993` `INFO max_memory`) to re-verify each fix cleanly in isolation (`p4_fix2_4t vs p4_fix1_4t` was `53.5% [47.37%, 59.63%]`, `p4_fix1_4t vs 33d1515_4t` `54.0% [48.44%, 59.56%]`, `p4_fix1_4t vs p3_batch3_4t` `53.0% [46.35%, 59.65%]`; 100g self-play CIs cross 50%, so individual 4T regressions were not separately proven). Also note `EvalSnapshot` is `608B` (`empty_bucket_counts` is `368B`, maintaining `nonzero_bucket_mask` read by NNUE L4/L5 checks) and `move_value` feeds the `vbw <= 0.0` filter before NNUE policy ordering.
  - Unsalted QVCF `nonlocal_b4_dropped` TT-skip (`HASHF_EMPTY` on pure key when distant B4 dropped): eliminated cross-move-order leaf TT poisoning, but increased 6-case fixed `d8/w40` nodes by `+9.46%` (`76,727 -> 83,987`; salted `p4_fix2` `+16.59%`), and scored `48.0% [42.05%, 53.95%]` (1T, `44-48-8`) and `49.0% [43.68%, 54.32%]` (4T, `45-47-8`) vs `p4_base`. Rapfi (`movepick.cpp:503-514`, `search.cpp:2236-2263`) likewise stores `RANGE_SQUARE2_LINE4` QVCF bounds directly on pure `zobristKey` for timed speed. Reverted TT-skip; kept only the zero-cost `alphabeta.rs:2328` narrow-window `HASHF_BETA` mate-bound fix.

Current active target:

- Final-depth late root candidates. Future analysis should explain why they are expensive, not re-confirm that they are expensive.
- **TODO (VCT speed-up in dense late-game positions, 70+ plies, untimed Master `d8/w40 + VCT6`)**:
  - Serial root `VCT6` remains single-threaded and uncapped in untimed mode, dominating dense 70+-ply moves even with 4T ABDADA enabled (timed mode already caps root VCT to 30% of remaining move time).
  - Candidate directions to evaluate later (check `AGENTS.md` rules before modifying base):
    1. **Route A (low risk)**: evaluate an explicit node/time cap for untimed root VCT (or compare `root_vct_depth = 4` vs `6` strength/latency tradeoff).
    2. **Route B (behavior-equivalent speedup — DONE on `feat/threat-pattern4-bitboard`)**: replaced `ThreatBoardView` board scans and quiet trial plays with a Rapfi-style incremental 4-direction `Pattern4` bitboard/LUT (`6.65x–9.33x` VCT6 speedup across dense Renju cases; see 2026-11-21 entry below).
    3. **Route C (experimental branch)**: upgrade `VCTSearcher` from IDDFS to `df-pn` (proof-number search with TT), validated for zero Renju false positives.
