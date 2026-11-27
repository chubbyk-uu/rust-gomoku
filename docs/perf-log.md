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

## 2026-11-27 — `exp/rapfi-60pct-phase1` Phase 1: Bit-exact `Mix9svq` AVX2/AVX-VNNI speedup (`+11%` 1T NPS, `+12%–14%` 4T NPS)

Context:

- Goal: close the raw NPS gap against `pbrain-rapfi` (`Mix9svq`, 32 MiB TT, 1s/move, Renju) before starting Phase 2 search-heuristic upgrades on `exp/rapfi-60pct-phase1`.
- Baseline (`master @ b7d750e`, i7-13700F AVX-VNNI, 32 MiB TT):
  - `mid10` 1T 1s: `407K` NPS (vs Rapfi `460K` NPS); 4T 1s: `1,886K` NPS (vs Rapfi `2,180K` NPS).
  - `open4` 1T 1s: `333K` NPS (vs Rapfi `418K` NPS); 4T 1s: `1,667K` NPS (vs Rapfi `2,516K` NPS).
- Component micro-benchmark (`fast_engine_component_timing`) confirmed `eval_value` (`~1,748 ns`) and `play_move` (`~1,466 ns` post-1.1) dominate; `eval.clone()` (`0.26 ms`) and `FastBoard play+undo` (`184 ns`) are negligible.

Kept optimizations (100% bit-exact vs scalar `Mix9svq`, zero move/score/node change on fixed depth):

- `src/nnue/mix9svq.rs`:
  - Unified `#[target_feature(enable = "avx2")] unsafe fn play_move_avx2` eliminating 214 cross-ABI AVX2 calls / `vzeroupper`s per move, with `MaybeUninit` `OnePointChange` and `AlignedVSum` stack buffers.
  - Precomputed `INNER_TO_OUTER: [u16; 225]` LUT in `evaluate_policy_moves_avx2/avxvnni`; `MaybeUninit` scratch buffers eliminating stack `memset`s in `evaluate_value_logits`, `evaluate_policy_moves`, and `star_block`; index loop + `clear()` replacing `Vec::drain(..)` in `flush_move_cache`.
  - Exact 13x13 `STAR_CONV_MASK` with branchless `trailing_zeros()` bit-scan skipping ~35.5% untouched `map_conv` cells, plus 4-direction telescoping fusion for center cell `(x, y)` (`41` updates instead of `44`).
  - Batched 4-input `star_block_batch4_avxvnni` (`src/nnue/simd.rs::linear_i8_chunk4_batch4_avxvnni`) sharing weight loads across the 4 corners, 4 edges, and 4 quads.
- Rejected micro-experiment (reverted):
  - Port-balanced AVX-VNNI + AVX2 `pmaddubsw/pmaddwd` hybrid on 2/8 lanes: only `-1.2%` (`1,680 -> 1,659 ns`, within noise) because `vpdpbusd` already shares execution ports cleanly on Raptor Cove; reverted to pure `vpdpbusd`.

Verification & timing:

- Bit-exactness: `mix9svq_avx2_matches_scalar_bit_exact_on_renju_and_freestyle` passed; full `cargo test --locked` passed (`0` failures); fixed `d15 1T` node counts 100% identical (`open4`: `36,808` nodes `113.8ms -> 103.4ms`; `mid10`: `10,619` nodes `28.1ms -> 25.5ms`).
- Release component & 1s warm benchmark (`fast_engine_component_timing`):
  - `eval_value (pure)`: `1,748 ns -> 1,676 ns` (at ~99% of Raptor Lake `vpdpbusd` throughput ceiling for 9,096 `vpdpbusd` ops).
  - `play_move (pure)`: `~1,600+ ns -> 1,244–1,278 ns`.
  - `mid10 1T 1s`: `407K -> 451.6K` NPS (`+11.0%`, `d19/29`, `440K` nodes; 98.2% of Rapfi `460K` `d18/37`).
  - `mid10 4T 1s`: `1,886K -> 1,937K–2,156K` NPS (`d21/33–37`, matching Rapfi `2,180K` `d18/49`).
- M1 100-game 1T Renju vs `pbrain-rapfi` (`strength_100_prefixes.jsonl`, 1s/move, 32 MiB TT, `--reuse-engine-state`):
  - `51-49-0` (`51.0% ± 2.86%`, `+6.95 Elo`, `WW=1, WL=49, LL=0`), `avg 551.0ms / med 901.7ms / p95 974.4ms / max 983.3ms` (vs Rapfi `max 1026.2ms`, `0` timeouts; 1 ply-224 full-board Black-only-forbidden terminal error).
  - Note: `strength_100_prefixes.jsonl` produced 49/50 splits because many ply-10 prefixes from classical games are already decisively won/lost for NNUE; all subsequent Phase 2+ single-factor gates use balanced Renjunet pro openings (`center_50_prefixes.jsonl`).
- Rejected Phase 2.1 experiment (reverted):
  - 3-key static-eval `CorrectionHistory` (`black_corr` + `white_corr` + `cont_corr`, `±256 cp` max, applied to `search()` and `vcfsearch()` with raw-eval TT purity):
  - 100-game 1T Renju self-play vs `phase1_1t` (`center_50_prefixes.jsonl`, 500ms/move, 32 MiB TT, `--reuse-engine-state`): `40-45-15` (`47.5% ± 4.23%`, `-17.39 Elo`, `WW=0, WD=4, WL/DD=38, LD=7, LL=1`, `0` errors / `0` timeouts).
  - Root cause: unlike Chess `pawn_hash` (slow endgame skeleton), Gomoku stones are permanent so `color_key(BLACK)` stays identical across all sibling White replies to a Black move, aliasing tactical outliers across siblings and overwhelming tight RFP/Razoring margins (`63 cp/ply`). Immediately reverted per stop-loss rule.
- Rejected Phase 2.2 experiment (reverted):
  - 1-ply + 2-ply `ContinuationHistory` (`2 x 225 x 225` `i16` each, `405 KiB/worker`, fed into `MovePicker` ordering + LMR `stat_score` without altering `static_eval`):
  - 100-game 1T Renju self-play vs `phase1_1t` (`center_50_prefixes.jsonl`, 500ms/move, 32 MiB TT, `--reuse-engine-state`): `42-44-14` (`49.0% ± 4.77%`, `-6.95 Elo`, `WW=1, WD=4, WL/DD=39, LD=4, LL=2`, `0` timeouts; 1 ply-224 full-board Black-only-forbidden terminal error on `phase1_1t`).
  - Root cause: `Mix9svq`'s dynamic pointwise policy head already supplies strong board-conditioned move ordering; allocating/zeroing `405 KiB` per move and indexing 2 extra tables per candidate added overhead without net Elo gain. Reverted per stop-loss rule.
- Rejected Phase 2.3 experiment (reverted):
  - Un-aged persistent `MainHistory` + `CounterMoveHistory` preserved across moves in `FastSearcher`:
  - 100-game 1T Renju self-play vs `phase1_1t` (`center_50_prefixes.jsonl`, 500ms/move, 32 MiB TT, `--reuse-engine-state`): `42-48-10` (`47.0% ± 4.72%`, `-20.87 Elo`, `WW=1, WD=1, WL/DD=42, LD=3, LL=3`, `0` errors / `0` timeouts).
  - Root cause: carrying un-decayed history from earlier board states biases LMR `stat_score` and ordering toward stale squares as tactics shift; fresh per-move history outperformed it cleanly. Reverted per stop-loss rule.

## 2026-11-27 (2) — `exp/rapfi-60pct-phase1` Step 1: 2 MiB Huge Pages + VQ Skip + Policy Batch-4 + Lazy MovePicker (`+21%–28%` NPS vs `master`, surpassing Rapfi)

Context:

- Component breakdown probe on `012ec27` (`AnonHugePages: 0 kB`) showed `play_move` (~44%), `eval_value` (~17%), `FastBoard` (~8%), and `eval_policy` (~6.2%, avg 58–64 candidates) plus `MovePicker` sorting (falling back to full `sort_unstable_by` at `INSERTION_LIMIT = 56 < avg_c`) accounted for total search time, while host THP `/sys/kernel/mm/transparent_hugepage/enabled` was `[madvise]` (meaning `pbrain-rapfi` got 2 MiB huge pages while Rust got 4 KiB pages).

Kept optimizations (100% bit-exact / order-exact, exact fixed `d15` node counts `10,619` and `36,808` unchanged):

- `src/fast_engine/tt.rs` & `src/nnue/mix9svq.rs`:
  - Linux 2 MiB Transparent Huge Pages (`alloc` → `madvise(MADV_HUGEPAGE)` → `write_bytes(0)`) for `FastTT` (`32 MiB`) and `Mix9svqWeight` `codebook`/`mapping_index` (`HugeBox<T>`, verified `AnonHugePages: 67,584 kB`).
  - Zero-delta VQ cluster skip (`if old_cb_idx == new_cb_idx`) and early `codebook` prefetch (`changes[0..4]` + `i + 4`) in `play_move_avx2`.
  - Fused 4-candidate AVX2 pointwise policy conv + output projection (`src/nnue/simd.rs::policy_eval_batch4_avx2`) loading `pw_w` once per 4 candidates (`eval_policy 30mv: 454 ns -> 411 ns`).
- `src/fast_engine/movepick.rs`:
  - Replaced upfront full `sort_unstable_by` on 60+ candidates with lazy top-6 `u32`-key selection sort (`scored_key` packing `(score DESC, pos ASC)`), deferring tail sort to rare all-nodes while keeping 100% identical move order.

Verification & timing (`fast_engine_component_timing`, release+LTO, 32 MiB TT):

- Fixed `d15 mid10` (`10,619` nodes, mv=129): `28.1ms (master)` → `25.04ms (Phase 1)` → **`23.16ms`** (`+21.3%` vs `master`).
- Fixed `d15 open4` (`36,808` nodes, mv=83): `113.8ms (master)` → `100.61ms (Phase 1)` → **`92.71ms`** (`+22.7%` vs `master`).
- `mid10 1T 1s`: `407K (master)` → **`492.7K` NPS** (`+21.1%` vs `master`, `1.07x` Rapfi `460K`).
- `mid10 4T 1s`: `1,886K (master)` → **`2,208K` NPS** (`d23/28`, surpassing Rapfi `2,180K`).
- `open4 1T 1s`: `333K (master)` → **`411.7K` NPS** (`+23.6%` vs `master`).
- `open4 4T 1s`: `1,667K (master)` → **`2,131K` NPS** (`+27.8%` vs `master`).
- Correctness: `mix9svq` bit-exact test + all `fast_engine` tests (`fast_tt_resize_kb_is_panic_safe_on_invalid_size` intact) passed.
- Milestone match vs `pbrain-rapfi` (`rust_step1_1t` vs `rapfi_1t`, 100g Renju `1s/move`, 32 MiB TT, `center_50_prefixes.jsonl`, `--reuse-engine-state`): **`47-44-9` (`51.5% ± 4.47%`, `+10.43 Elo`, CI `[-20.65, +41.67]`, `WW=2, WD=4, WL/DD=39, LD=5, LL=0`)**, vs `master` (`v0.1.20`) `49.5%` (`+14 Elo` from pure bit-exact speedups alone). Timing: `rust_step1_1t` `avg 692.7ms / med 969.6ms / p95 974.7ms / max 980.4ms` (`0` Rust errors, `0` timeouts; 1 Rapfi ply-224 overline loss) vs `rapfi_1t` `694.8 / 970.2 / 974.3 / 1033.3 ms`. Move-time histogram showed `360 / 3238` moves (`11.1%`) stopped early between `873ms` and `960ms` due to `optimum_ms = 0.90 * max_ms`.
- Evaluated & dropped LLVM PGO (`-C profile-generate` / `-C profile-use`, 7 Renju+Freestyle 1T/4T workloads): `mid10_d17` `521.1ms -> 507.4ms` (`-2.6%`), `open4_d16` `176.7ms -> 179.1ms` (`+1.3%`, PGO+native `522.3ms / 178.8ms`). Because fat LTO + explicit AVX2/AVX-VNNI kernels already dominate runtime, PGO yielded < 1–2% net gain and is not worth maintaining a platform-bound build pipeline.
- **Step 3A (kept, `7bf4426`) — Cut-node Negative Singular Extension** (`else if cut_node { extension = -1.5; }` in `src/fast_engine/search.rs`):
  - 200-game 1T Renju screen vs `gomocup_step1` (`renju_100pairs.jsonl`, `300ms/move`, 32 MiB TT, `--reuse-engine-state`): **`89-85-26` (`51.0% ± 3.45%`, `+6.95 Elo`, `WW=4, WD=10, WL/DD=75, LD=8, LL=3`)**, positive on both halves (`old_50`: `51.5%`, `new_50`: `50.5%`), 0 real errors / 0 timeouts; deepened 1s probes by +1–2 plies (`mid10 1T`: `d21/41 @ 517K NPS`, `open4 4T`: `d22/38`).
- **Step 3B (rejected & reverted) — Raising `ample_match_time` `optimum_ms` ratio `0.90 -> 0.99` + resetting `rm.value = VALUE_NONE` per depth**:
  - 200-game 1T Renju screen vs `gomocup_3a` (`renju_100pairs.jsonl`, `300ms/move`, 32 MiB TT): **`82-96-22` (`46.5% ± 3.59%`, `-24.36 Elo`, `WW=2, WD=5, WL/DD=78, LD=7, LL=8`)**. Forcing an extra 11.4% of moves to launch unfinishable `d+1` iterations with <10% time remaining caused partial-iteration root instability and TT churn; reverted immediately per stop-loss rule.
- **Bit-exact `Mix9svq` 64B cache-line alignment + 2-pass `mapping_index` prefetch (`src/nnue/mix9svq.rs`)**:
  - Wrapped `map_sum` (`128B`) and `map_conv` (`64B`) in `#[repr(C, align(64))] struct AlignedRow<N>` (eliminating 3/4-probability cache-line splits across 522 `map_conv` + 41 `map_sum` ops/move and enabling aligned AVX2 loads/stores), plus 2-pass `mapping_index` prefetching in `play_move_avx2`.
  - 100% bit-exact (`mix9svq` test + identical fixed `d15` nodes `28,984` / `32,328`): `fixed d15 open4` `84.16ms -> 79.41ms` (`-5.6%`), `mid10 1T 1s` `517K -> 524.4K` NPS (`+28.8%` vs `master`, `1.14x` Rapfi), `mid10 4T 1s` `2,265K -> 2,360.5K` NPS (`+4.2%`, `1.08x` Rapfi).
- **Step 3D-1 (kept) — Non-PV Cut-node IIR** (`!root_node && !pv_node && cut_node && depth >= 4.0 && depth < IID_DEPTH && tt_move == NONE_MOVE => depth -= 0.75`):
  - 200-game 1T Renju screen vs `gomocup_base3` (`renju_100pairs.jsonl`, `300ms/move`, 32 MiB TT, `--reuse-engine-state`): **`93-84-23` (`52.25% ± 4.08%`, `+15.65 Elo`, `WW=9, WD=7, WL/DD=72, LD=8, LL=4`)**, zero speed overhead (`avg 219.2ms` vs `219.1ms`), 0 real errors / 0 timeouts.

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

### 2026-11-21 (3): Correctness fixes for Pattern4 VCT and sub-ply TT (4 reproduced bugs)

The 1/8-ply TT above was **unsound**: store and probe both used `floor`, so a
`depth 1.00` entry could cut off a `depth 1.10` search. Part of the `4.03s` came
from that unsound cutoff. Four reproduced bugs, all fixed with regressions:

1. **VCT promoted a blockable Renju four to A4** (`threat_board.rs::collect_attack_moves`).
   The black reply legality was checked *before* White's B4 stone. White can
   break a Black double-three and make the only block legal again. Now: if the
   reply had no win5/forbidden/`maybe_33` flag before the move, it stays legal
   (a White stone cannot create Black patterns). Otherwise it is rechecked after
   `play(move)`. Regression: `[[6,9],[0,11],[7,4],[2,9],[5,6],[1,10],[5,8]]`.
   White `[3,8]` is B4 with defense `[4,7]`, and root d1/w10 returns `[4,7]` / `-22`.
2. **Pattern4 `maybe_33` missed full-rule double threes.** `maybe_33` counted only
   straight-VCT `A3` axes. New `P1::RenjuOpen3Candidate` reproduces
   `rules::forbidden::{open_three_gain_indices, has_open_four_through_gain}` on the ±5
   window. Now `maybe_33` is a superset of full-rule DoubleThree, falling back to
   `rule_legal_cached`. Regression: the 17-ply game; White `[7,6]` A3 defenses are
   exactly `{[5,4],[10,9]}`, and `[8,7]` is flagged `maybe_33` and judged illegal.
3. **Mixed TT depth encodings on thread switch or concurrent shared TT** (GUI/Android/library).
   `TranspositionTable` now carries a handle-local `subply: bool` mode selector
   (`set_subply`), while `TTStorage` binds the first mode used to its primary bucket
   array (zero extra memory in single-mode runs) and lazily routes the other mode to
   an isolated secondary bucket array (with per-entry mode-bit tagging inside
   `RawEntry::data` as an OOM fallback). `RootSearcher::with_tt`,
   `AlphaBetaSearcher::new`/`with_tt`, and `search_vct_and_alphabeta_overlap`
   synchronize `subply` before `fork_snapshot`, avoiding any extra TT plane
   allocation during overlapped VCT + Alpha-Beta search
   (`multithread_overlap_vct_alphabeta_does_not_allocate_secondary_tt_plane`).
   Sequential (`4→1`, `1→4`) and concurrent `1T + 4T` searchers sharing the same
   `TranspositionTable` are completely isolated down to exact node counts
   (`concurrent_and_interleaved_1t_and_mt_searches_on_shared_tt_stay_isolated`).
4. **Sub-ply TT floor/floor cutoff & d13+ / d51+ depth-cap saturation.** Now strictly
   sound for arbitrary depths:
   - Widened TT `depth` to 12 bits (`0..=4095`), kept `priority` at full 16 bits
     (`0..=65535`), tagged `subply` in 1 bit, and set `generation` to 9 bits
     (`0..=511`) inside the 64-bit `RawEntry::data` word.
   - Multi-threaded TT uses 1/64-ply `store = floor(depth * 64.0)` (clamped from `0`)
     and `probe = ceil(depth * 64.0)` (clamped from `1` for `depth > 0.0`, while 1T
     keeps `store = floor(depth)` clamped from `0` so `0 < depth < 1` stores `0` and
     never cuts off `depth = 1.0`; `single_thread_sub_one_depth_stores_zero_and_never_cuts_off_depth_one`)
     with `SUBPLY_TT_DEPTH_CAP = 3200` (50 plies,
     well above `timed_max_depth = 25`; `+640` win/loss bonus = `3840 <= 4095`),
     floor-quantized `base_depthdown = floor(raw_depthdown * 64.0) / 64.0`, and full
     sub-ply replacement priority `priority_base * 256 + hash_depth`.
   - When the actual requested depth exceeds the encodable range (`> 50` plies in MT
     mode or `> 4085` plies in 1T mode), `probe_depth` is raised to `DEPTH_MAX + 1`
     (`4096`) instead of being clamped down, so saturated TT entries only supply
     move-ordering hints and never cut off or narrow the window
     (`depth_above_50_uses_tt_move_hint_only_without_saturated_cutoff`).
   - Regressions: `1.00 -> 1.10`, `1.563125 -> 1.625`, and `1.6095 -> 1.625`
     never cut off deeper searches (`subply_tt_shallower_depth_never_cuts_off_deeper_search`),
     and 2-thread `d14/w2` genuinely expands `d13` and `d14` matching `1T`
     (`multithread_deep_search_above_d12_does_not_saturate_tt_depth_cap`).

Verification: `cargo test` and the Android bridge pass, and 13/13 reference diffs
pass (1T base unchanged).

VCT6 (`--depth 1 --width 10`, 3-run median, final build). Node stats are identical
to the Route B commit:

| Case | Baseline `cbd0a7c` | Final | Speedup |
| --- | ---: | ---: | ---: |
| `vct_dense_ply78` | `1555.0 ms` | `181.2 ms` | `8.58x` |
| `vct_counter_win_scan_ply75` | `2659.1 ms` | `368.1 ms` | `7.22x` |
| `vct_slow_probe_case` | `3491.9 ms` | `637.2 ms` | `5.48x` |
| **Total** | **`7706.0 ms`** | **`1186.5 ms`** | **`6.49x`** |

`vct_dense_ply78` d8/w40, 64 MiB TT (`--tt-bits 21`):

| Threads | Move | Score | Nodes | Elapsed |
| --- | --- | ---: | ---: | ---: |
| 1T | `[8,1]` | 28 | 73,267,291 | 70,109 ms |
| 4T | `[8,1]` | 29 | 17,737,980 | 6,456 ms (`10.9x`) |
| 8T | `[8,1]` | 29 | 19,744,653 | 3,865 ms (`18.1x`) |

100-game Renju `center_50_prefixes.jsonl` (`4T_sound64_prio` vs `1T_base`, d8/w40,
`--reuse-engine-state`, 0 errors / 0 timeouts / 0 capped pairs):

- Strength: **`50-47-3` (`51.50%`, `+10.43 Elo`**, 95% CI `[-33.7, +54.9]`;
  pentanomial `WW=5, WD=2, WL/DD=38, LD=1, LL=4`) — clears the `>= 50%` gate.
- Timing: 4T avg **`463.1 ms`** / median **`217.3 ms`** / p95 **`1,721.5 ms`** /
  max **`11,102.6 ms`** vs 1T avg `1,410.9 ms` / `586.5 ms` / `5,709.0 ms` /
  `26,465.7 ms` (**`3.05x` / `2.70x` / `3.32x` / `2.38x`** faster).

### 2026-11-22: P0 (`M0.1`–`M0.3`) — Lean `EvalSnapshot`, `TTTableView` + prefetch, and persistent `WorkerLocal`

Hotspot evidence & target:

- `EvalSnapshot` copied `~1.16 KiB` per snapshot/restore including a dead 450B
  `occupied_moves` array (only read in `#[cfg(debug_assertions)]`) and 225B
  `board_shadow` (only 1 cell changes per move), while undo logs stored wide
  `usize` coordinates (`24B–40B` per entry).
- `TranspositionTable::probe/store` executed an `Acquire` atomic load on
  `primary_mode` per node and issued no software prefetch.
- `SearchWorkerPool` cloned `EngineConfig` (4 `Vec<f64>`s), `EvalCaches`, and
  constructed a fresh `AlphaBetaSearcher` on every `depth >= 5` per worker.

Modification scope (100% behavior-equivalent):

- `src/eval/caches.rs`, `src/eval/local.rs`, `src/eval/global_eval.rs`:
  removed `occupied_moves`/`occupied_len`, journaled `board_shadow` via
  `shadow_log`, and compacted undo log entries to `u8`/`i16`/`i8` tuples
  (`3B–8B` each), shrinking `EvalSnapshot` by ~59%.
- `src/search/tt.rs`, `src/search/alphabeta.rs`: added `TTTableView<'a>` to
  resolve the 1T/MT bucket plane once per root search (eliminating per-node
  `Acquire` loads) and issued x86_64 `_mm_prefetch` after `board.play`.
- `src/search/abdada.rs`, `src/search/root.rs`: wrapped `DepthJob.config` in
  `Arc<EngineConfig>` and kept persistent `WorkerLocal` (`EvalCaches` +
  `AlphaBetaSearcher`) per worker thread across depths.

Verification & A/B timing (release build, clean CPU):

- Correctness: `cargo fmt --check`, `cargo test`, `android/rust_bridge` tests,
  and `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13
  passed**, exact move/score/node parity).
- `cases/renju/vct_dense_ply78.json` (`d7/w40`, Renju, `15,324,461` nodes):
  - `1T`: `alphabeta_ms` `13,833.1 ms -> 13,236.6 ms` (**`-4.3%`**, `903 -> 864 ns/node`)
  - `4T`: `alphabeta_ms` `1,794.2 ms -> 1,726.1 ms` (**`-3.8%`**)
- `renju_search_bench` (`d8/w40`, 6 positions): Freestyle `1654 -> 1644 ns/node`
  (`-0.6%`), Renju `1967 -> 1961 ns/node` (`-0.3%`).

### 2026-11-22 (2): `M1.1` — Persistent 88-line `PackedLine` bitboards in `EvalCaches`, skip occupied neighbor `SHAPE_TABLE` lookups, drop legacy `ThreatBoardView::x1..x4`

Hotspot evidence & target:

- `value_wide_compute_for_rule` rebuilt 4 `PackedLine`s by scanning 60 board
  cells on every move, and eagerly called `hypothetical_shapes()` (2×
  `SHAPE_TABLE` lookups) for every neighbor cell along the 4 lines — even
  already-occupied cells that `update_line_neighbor_point` immediately
  discarded.
- `ThreatBoardView` still heap-allocated 88 `Vec<i32>` (`x1..x4`) on every
  construction and updated 4 2D vectors on every `play/undo` even though
  `Pattern4State` had replaced all hot-path reads.

Modification scope (100% behavior-equivalent):

- `src/patterns/line.rs`, `src/eval/caches.rs`, `src/eval/local.rs`:
  added persistent `packed_lines: [[u32; 2]; 88]` + compile-time
  `PACKED_ON_BOARD_MASKS` inside `EvalCaches` (incrementally updated via
  `set_board_shadow` / `shadow_log` with zero `EvalSnapshot` copy overhead and
  verified bit-identical to `PackedLine::through` under `debug_assert_eq!`),
  and moved `line.hypothetical_shapes()` inside the `cell == EMPTY` branch of
  `update_line_neighbor_point`.
- `src/threats/threat_board.rs`: removed `x1..x4` `Vec<Vec<i32>>` fields and
  per-play/undo writes; `lines_for` fallback builds stack `[i32; 15]` arrays
  from `board.grid_rows()`.

Verification & A/B timing (release build, clean CPU, cumulative vs `master` `7fafe74`):

- Correctness: `cargo fmt --check`, `cargo test` (with `debug_assert_eq!`
  bit-identity check), `android/rust_bridge` tests, and `scripts/run_diff.py
  --profile all --check-baselines --jobs 8` (**13/13 passed**, zero move/score/node
  changes).
- `cases/renju/vct_dense_ply78.json` (`d7/w40`, Renju, `15,324,461` nodes):
  - `1T`: `alphabeta_ms` `13,833.1 ms -> 12,602.1 ms` (**`-8.9%`** cumulative, `903 -> 822 ns/node`)
  - `4T`: `alphabeta_ms` `1,794.2 ms -> 1,659.0 ms` (**`-7.5%`** cumulative)
  - `VCT6`: `173.5 ms -> 165.1 ms` (**`-4.8%`**), `vct_counter_win_scan_ply75` `368.1 ms -> 336.8 ms` (**`-8.5%`**), exact OR/AND nodes unchanged.
- `renju_search_bench` (`d8/w40`, 6 positions):
  - Freestyle: `1252.2 ms (1654 ns/node) -> 1172.5 ms (1549 ns/node)` (**`-6.4%`**)
  - Renju: `2100.4 ms (1967 ns/node) -> 2010.2 ms (1882 ns/node)` (**`-4.3%`**)

### 2026-11-22 (3): Reverted experimental rounds (`M1.2` deep VCF, `M1.3` per-turn soft early-stop/overhead, `M2.1` 4-way + 1T `subply_tt`, `M2.4a` mild LMR, `M2.3a` leaf-VCF/single-A3 trigger)

All experiments below were evaluated against `master` (`7a30f07`) on 100-game Renju matches (`--reuse-engine-state`, 0 errors / 0 timeouts) and **reverted** because they failed the strict dual-mode `> 50%` gate (must win or hold in both untimed `d8/w40` and timed `1s/move`):

1. **`M1.3` per-turn soft-limit early stop (`strength_100_prefixes.jsonl`, 100 games, `1s/move`)**:
   - Result: `44-56-0` (`44.0%`, `-41.9 Elo`), avg `313.4ms` vs `418.3ms`.
   - Root cause: in pure per-turn timed games (`timeout_turn` without match-clock carryover), unused turn time is forfeited and stopping early sacrifices TT warming for the next move.
   - Rule: soft-limit early stop must ONLY be enabled when remaining time carries over across moves (match-clock mode), never in pure per-turn games.
2. **`M1.2` raising default `opponent_vcf_depth` (`4 -> 12`) + `root_vcf_depth` (`5 -> 16`) (`center_50_prefixes.jsonl`, 100 games, untimed `d8/w40`)**:
   - Result: `46-51-3` (`47.5%`, `-17.4 Elo`, `WW=0, WL/DD=47, LD=1, LL=2`), avg `1389ms` vs `1369ms`, max `20.8s` vs `29.2s`.
   - Root cause: cold-TT moves on all 3 diverging cases were identical; the 3 pair flips came from extra root VCF filtering altering warm-TT state without delivering net tactical wins over existing `VCT6 + d8` (`extend_ratio=0.9`).
3. **`M2.1` 64B 4-way TT + unified 1T `1/64`-ply `subply_tt` (`center_50_prefixes.jsonl`, 100 games vs `master` `7a30f07`)**:
   - v1 had a same-key overwrite bug (`entry.flag == HASHF_EXACT` allowed depth-0 leaves to overwrite deep internal entries): untimed `d8/w40` scored `43-50-7` (`46.5%`, avg `1125ms` vs `1696ms`).
   - v2 fixed same-key overwrite protection (never let shallower/leaf entries overwrite deeper ones):
     - **Timed `1s/move`**: **`54-45-1` (`54.5%`, `+31.35 Elo`, `WW=9, WD=1, LL=5`)**, median **`96.7ms` vs `431.0ms` (`4.46x` faster)**, max `994ms` vs `1020ms`.
     - **Untimed `d8/w40`**: `44-55-1` (`44.5%`, `-38.4 Elo`, `WW=5, WD=1, LL=11`), avg **`936ms` vs `1234ms` (`1.32x`)**, p95 **`3162ms` vs `4535ms` (`1.43x`)**, max **`20.3s` vs `32.3s` (`1.59x`)**, TT nodes `45.0M` vs `122.6M` (`2.72x` fewer).
   - Root cause of fixed-depth dip: `running_downf` carries path-dependent remainder `0..14` across plies without encoding it in the Zobrist key. Higher 1T TT hit rates amplify cross-path `downf` cutoffs at fixed `d8`, whereas equal-time play easily profits from the `1.3x–2.4x` speedup.
4. **`M2.4a` stateless mild logarithmic LMR replacing `running_downf` (`center_50_prefixes.jsonl`, 100 games, `1s/move`)**:
   - Result: `39-60-1` (`39.5%`, `-74.1 Elo`), `vct_dense_ply78` d7 nodes exploded `5.1x` (`4.49M -> 22.90M`, `5.3s -> 26.9s`).
   - Root cause: reducing `r` on `self_attack >= 3 || opp_attack >= 4 || hostile_threat` made late-move reductions far weaker than `running_downf` at `wide=40`, costing 1–2 plies in timed play.
5. **`M2.3a` VCT leaf-VCF + single-A3 trigger + 25ms turn overhead**:
   - Nesting a 6-ply recursive VCF at every `depth == 0` VCT OR-leaf (`>100K` leaves on dense boards) blew up `vct_dense_ply78` VCT6 from `166ms` to `>120s` (killed).
   - Replacing it with an `O(1)` leaf `win5/a4` bitboard check kept VCT6 fast (`166ms` / `309ms`), but adding `legal_a3_count >= 2` weak trigger + 25ms per-turn overhead scored `46-53-1` (`46.5%`, `-24.4 Elo`) on `center_50_prefixes.jsonl` `1s/move` (weak VCT triggers consumed timed AB budget + 25ms overhead cut 2.5% per-turn search time).

Decision:

- Reverted all 5 experimental paths to keep `master` (`7a30f07`) clean.
- Next focus for **guaranteed dual-mode (untimed `d8/w40` + timed `1s/move`) gains**:
  1. `M2.2a` (tested & reverted): Stage 0 `TT Move` zero-movegen fast path gated on `!has_high_threat_bucket() && covered_empty >= wide + 10` scored `48-50-2` (`49.0%`, `WW=7, LL=7`, TT nodes `34.38M` vs `34.34M`, avg `524.1ms` vs `526.3ms`) on `center_50_prefixes.jsonl` `1s/move`. Because midgame boards almost always contain at least one `L4`/`L5` cell somewhere on the 15x15 grid, `!has_high_threat_bucket()` blocked Stage 0 on ~90% of real midgame nodes (confirmed by `vct_dense_ply78` changing only `0.3%`). Reverted.
  2. `M2.3`: `df-pn` VCT solver with bounded node budget (deep narrow forcing lines 10–20 plies without wide-branch explosion — genuine tactical strength gain in both untimed `d8/w40` and timed `1s/move`).
  3. `M3.1`: stronger classical evaluation (4-direction + attack/defense cross terms + per-rule integer weights + SPSA/Texel tuning) to break the fixed-depth strength ceiling.

### 2026-11-22 (4): `M2.3` `df-pn` VCT solver (`exp/m23-dfpn-vct`)

Context & modification scope:

- Added `src/threats/dfpn.rs` (`DfpnSearcher`): Depth-First Proof-Number (df-pn) VCT solver backed by `Pattern4State` / `ThreatBoardView` (preserving all verified Renju-safe attack/defense semantics) and a 65,536-bucket 2-way Zobrist TT (`131,072` entries, ~3 MiB, generation-salted, always-replace slot 1 when slot 0 holds a solved entry so progress is never dropped on hash collisions).
- Initial proof-number biasing: `B4` (`pn=1, dn=2`), `Double-A3` (`pn=1, dn=4`), single-`A3` (`pn=9+ply, dn=1`) so narrow `B4` chains and lethal `Double-A3` forks are proved immediately while wide `A3` trees scale shallow-first; depth cap (`max_ply = (root_vct_depth * 3).clamp(12, 22)`, default `18` plies) stores finite `DFPN_DEPTH_CAP_PN = 50_000` (never `dn=0`) so depth truncation is never mis-cached as a mathematical disproof; `DEFAULT_DFPN_NODE_LIMIT = 120_000`.
- Wired `DfpnSearcher` into `RootSearcher` (`src/search/root.rs`), keeping `verify_root_vct_move` and timed 30% sub-budget intact.

Verification & A/B benchmarks (release build vs `master` `7a30f07`):

- Unit/integration tests: `cargo fmt --check`, full `cargo test` (`root_alignment` 33/33, `dfpn` 3/3 including Renju `case17` false-positive check and `dual_a3_diamond`), and `android/rust_bridge` 6/6 all passed with zero warnings.
- VCT case probes (`Renju`, `d6/w40`, `4T`):
  - `cases/renju/vct_dense_ply78.json`: VCT `165.1 ms -> 57.6 ms` (**`2.87x` faster** while searching up to 18 ply).
  - `cases/renju/vct_counter_win_scan_ply75.json`: VCT `336.8 ms -> 146.8 ms` (**`2.29x` faster**).
  - `cases/renju/vct6_false_positive_case17_after_ply40.json`: rejected `(8,8)` false positive AND proved the real winning VCT `(9,9)` in **`7.78 ms`** (previously missed by IDDFS VCT6, requiring `44 ms` alpha-beta).
- 100-game Renju center-opening matches (`center_50_prefixes.jsonl`, `--jobs 16 --reuse-engine-state`, vs `master_7a30f07`, `0` errors / `0` timeouts in both modes):
  - **Timed `1s/move`**: **`52-47-1` (`52.5%`, `+17.39 Elo`, `WW=2, WD=1, WL/DD=47, LD=0, LL=0` — zero lost pairs)**; avg `520.6 ms` vs `525.0 ms`.
  - **Untimed `d8/w40`**: **`48-48-4` (`50.0%`, `0.0 Elo`, `WW=0, WD=0, WL/DD=50, LD=0, LL=0` — zero lost pairs)**; avg `1669.6 ms` vs `1692.9 ms` (`-1.4%`), median `799.9 ms` vs `817.2 ms` (`-2.1%`), p95 `6351.1 ms` vs `6522.9 ms` (`-2.6%`).

Decision:

- `M2.3` (`df-pn` VCT) strictly dominates IDDFS VCT6 across timed strength (`+17.4 Elo`, zero lost pairs), untimed stability (`50.0%`, zero lost pairs), and VCT speed (`2.3x–2.9x` faster on dense Renju cases). Merged to `master` (`7c7e447`).

### 2026-11-22 (5): `M2.3` post-review hardening (`df-pn` termination, local child cache, fallback switch, diff & Freestyle validation)

Context (caught by adversarial code review of `7fafe74..7c7e447`):

- Fixed two termination bugs in `src/threats/dfpn.rs`:
  1. `ply >= max_ply` previously stored finite `(DFPN_DEPTH_CAP_PN = 50_000, 1)`, allowing `mid_and` (`ply = max_ply - 1`) to spin until `node_limit` when `th_pn > 50_000 && th_dn >= 2`. Because Gomoku has no captures/passes, every board state uniquely determines `ply`, so `ply >= max_ply` now stores `(DFPN_INF, 0)` and sets `hit_depth_cap = true` (`solved = dn == 0 && !aborted && !hit_depth_cap`), guaranteeing monotonic termination with zero shallower-ply pollution.
  2. `mid_or` / `mid_and` previously discarded child return values and re-read all children from the 2-way TT every iteration, which could ping-pong if two unsolved siblings collided in a bucket whose slot 0 held a solved entry. Both loops now cache `child_pn` / `child_dn` locally (initialized once from TT) and update `best_idx` directly from recursive return values.
- Additional hardening:
  - `view.board.force_side_to_move(side)` at `DfpnSearcher::search_for_rule` entry to guarantee Zobrist alignment even when `board.side_to_move() != side`; terminal winner early-returns now write to TT; manual `Clone` drops the 3 MiB TT on clone.
  - Added `RuntimeOptions::vct_use_dfpn` (`DEFAULT_VCT_USE_DFPN = true`, `INFO vct_use_dfpn 0|1`) in `src/config.rs`, `src/protocol/gomocup.rs`, and `src/search/root.rs` to preserve a 1-flag fallback to legacy `VCTSearcher`; removed the hard `12`-ply lower clamp (`(root_vct_depth * 3).clamp(2, 22)`); populated `VCTStats` (`depth_limit`, `depth_completed`, `memo_exact_hits`).
  - `rebuild_global_eval_state` (`src/eval/local.rs`) now always refreshes `packed_lines` under `preserve_snapshot` so direct `board_shadow` edits in tests stay self-healing.
  - Added 6 targeted `dfpn` unit tests (`max_ply` termination, `MoveBudget` node limit, Renju Black 3-3 attack exclusion, Renju White win via Black forbidden block, mismatched `side_to_move` + clone/multi-call reuse).

Verification & A/B benchmarks (release build vs `master` `7a30f07`):

- Correctness: `cargo fmt --check`, `git diff --check`, full `cargo test` (`dfpn` 9/9, `root_alignment` 33/33, zero warnings), `android/rust_bridge` 6/6, and `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed**; pinned `root_zhou_black_5_10_4_prefix_14_d6_w20` where `df-pn` finds faster mate-in-3 `[10, 3]` vs legacy `[9, 5]`).
- VCT case probes (`Renju`, `d6/w40`, `4T`):
  - `cases/renju/vct_dense_ply78.json`: VCT `165.1 ms -> 49.8 ms` (**`3.31x` faster** vs IDDFS VCT6, `-13.5%` vs pre-fix `df-pn`).
  - `cases/renju/vct_counter_win_scan_ply75.json`: VCT `336.8 ms -> 128.3 ms` (**`2.63x` faster** vs IDDFS VCT6, `-12.6%` vs pre-fix `df-pn`).
  - `cases/renju/vct6_false_positive_case17_after_ply40.json`: proves `(9,9)` in **`7.69 ms`** (zero false positives).
- 300-game center-opening verification (`center_50_prefixes.jsonl`, `--jobs 16 --reuse-engine-state`, vs `master_7a30f07`, `0` errors / `0` timeouts across all 300 games):
  - **Renju timed `1s/move` (100 games)**: **`52-47-1` (`52.5%`, `+17.39 Elo`, `WW=6, WD=1, WL/DD=39, LD=0, LL=4`)**; `df-pn` avg `516.9 ms` / med `1000.1 ms` / p95 `1009.2 ms` / max `1024.2 ms` vs `master` avg `517.9 ms` / med `1000.1 ms` / p95 `1009.5 ms` / max `1026.6 ms`.
  - **Freestyle timed `1s/move` (100 games)**: **`50-48-2` (`51.0%`, `+6.95 Elo`, `WW=3, WD=1, WL/DD=43, LD=1, LL=2`)**; `df-pn` avg `476.2 ms` / **med `46.1 ms` (`3.16x` faster)** / p95 `1000.6 ms` / max `1035.3 ms` vs `master` avg `483.6 ms` / med `145.8 ms` / p95 `1000.4 ms` / max `1025.8 ms`.
  - **Renju untimed `d8/w40` (100 games)**: **`48-48-4` (`50.0%`, `0.0 Elo`, `WW=0, WD=0, WL/DD=50, LD=0, LL=0`)**; `df-pn` avg `1450.4 ms` (`-1.6%`) / med `682.1 ms` (`-1.1%`) / p95 `5511.5 ms` (`-3.6%`) / max `29304.8 ms` vs `master` avg `1474.2 ms` / med `689.4 ms` / p95 `5715.7 ms` / max `29223.6 ms`.

Decision:

- `M2.3` (`df-pn` VCT) is hardened and verified on `master` across both rulesets and both time modes; next milestone is `M3.1` (stronger integer classical evaluation + SPSA/Texel tuning).

### 2026-11-23: `M3.2` `Mix9svq` incremental NNUE (Value + Policy ordering + AVX2 SIMD) (`exp/m32-nnue-mix9svq`)

Context & modification scope:

- Implemented pure-Rust `Mix9svq` NNUE inference compatible with Rapfi's CC0 `.bin.lz4` weights (`src/nnue/lz4.rs`, `src/nnue/simd.rs`, `src/nnue/mix9svq.rs`, `src/nnue/mod.rs`), preserving 100% classical fallback when `use_nnue = false` (default when no `--nnue-dir` / `INFO nnue_dir` / `RUST_GOMOKU_NNUE_DIR` is supplied).
- Architecture & SIMD optimizations:
  - Pure-Rust standard LZ4 Frame decoder + `StandardHeader` validator (`0xacd8cc6a` + `MIX9SVQ_ARCH_HASH` + rule/board-size masks) + 10-bit `PackedSigned10Stream` codebook decoder + `OnceLock` `Arc<Mix9svqWeight>` read-only weight cache shared across ABDADA threads.
  - Version-stacked incremental accumulator (`undo()` = `current_version -= 1`, O(1)) + lazy `MoveCache` cancelling consecutive opposite moves before any evaluation occurs.
  - Runtime-detected `x86_64` AVX2 kernels with 8-YMM-register unrolled `linear_i8_chunk4_avx2` (hoisting signed-input `msb/low7` splits once per chunk), `commit_vsum_avx2`, `codebook` prefetching, and candidate-only `evaluate_policy_moves_avx2` + `policy_dot16_avx2` (`evaluate_value`: `3.91 us -> 2.83 us`, **`-27.6%` / `1.38x`** speedup).
- Search integration (`src/search/alphabeta.rs`, `src/config.rs`, `src/protocol/gomocup.rs`, `src/bin/gomocup_engine.rs`):
  - Keeps root `VCF + df-pn VCT` and leaf `±WIN` exact tactical win/loss checks untouched; replaces non-mate leaf static evaluation with `Mix9svq` Value (`ScalingFactor = 200`, clamped `[-15_000, +15_000]`), disables classical `atdown/rootbonus` when NNUE is active, and sorts root & `depth >= 2.0` internal non-forcing candidates with `Mix9svq` Policy (keeping `tt_best_move` first).

Verification & A/B benchmarks (release build vs `master` `8563005`):

- Bit-exact C++ oracle verification (against compiled `opponent/rapfi/Rapfi/eval/mix9svqnnue.cpp`):
  - Freestyle 8-ply BLACK: Rust `val = 994` vs Rapfi C++ `994` (exact); top-5 Policy matches to `< 2e-6` (`12.929857` vs `12.929859`).
  - Renju 7-ply WHITE / 8-ply BLACK: Rust `80 / 225` vs Rapfi C++ `80 / 225` (exact); AVX2 matches scalar `layer3i32` bit-identically; `undo()` matches `sync_with_board()`.
- Unit/integration & diff regression: `cargo fmt --check`, `git diff --check`, full `cargo test` (zero warnings), `android/rust_bridge` 6/6, and `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed** with default classical fallback).
- 300-game center-opening verification (`center_50_prefixes.jsonl`, `--jobs 16 --reuse-engine-state`, vs `master_8563005`, **ZERO lost pairs `LD=0, LL=0` across all 300 games**):
  - **Renju timed `1s/move` (100 games)**: **`79-21-0` (`79.0%`, `+230.16 Elo`, 95% CI `[+162.2, +318.5]`, `WW=29, WD=0, WL/DD=21, LD=0, LL=0`)**, `0` errors / `0` timeouts; avg `491.5 ms` / med `120.1 ms` / p95 `1068.6 ms` / max `1107.3 ms` vs `master` avg `463.3 ms` / med `18.5 ms` / p95 `1010.6 ms` / max `1022.3 ms`.
  - **Freestyle timed `1s/move` (100 games)**: **`71-28-1` (`71.5%`, `+159.78 Elo`, 95% CI `[+103.1, +225.8]`, `WW=21, WD=1, WL/DD=28, LD=0, LL=0`)**, `0` errors / `0` timeouts; avg `491.0 ms` / med `122.9 ms` / p95 `1042.3 ms` / max `1072.4 ms` vs `master` avg `446.7 ms` / med `10.6 ms` / p95 `1009.0 ms` / max `1024.1 ms`.
  - **Renju untimed `d8/w40` (100 games, 97 completed + 3 long games hit harness default 900s game-timeout at plies 118–160 under 16-way contention)**: **`79-18-0` (`80.85%`, `+250.22 Elo`, 95% CI `[+176.8, +350.3]`, `WW=29, WD=0, WL/DD=18, LD=0, LL=0`)**; TT nodes **`82.5M` vs `162.3M` (`1.97x` fewer nodes)**; avg `2121.2 ms` / med `730.3 ms` / p95 `8395.9 ms` / max `65837.8 ms` vs `master` avg `1561.2 ms` / med `378.9 ms` / p95 `6584.4 ms` / max `59146.8 ms`.

Decision:

- `M3.2` (`Mix9svq` Value + Policy + AVX2) delivers a massive `+160` to `+250+ Elo` leap across BOTH Renju & Freestyle and BOTH untimed `d8/w40` & timed `1s/move` with zero lost pairs. Ready for merge to `master` upon user approval; next follow-ups are Policy-guided LMR / 4-way TT (to make `d8/w40` even faster than classical `master`) and ARM64 NEON SIMD for Android.

### 2026-11-23 (2): `P0` (`AVX-VNNI` + Lazy Policy + LCP board sync) & `P1` (4-way 64B TT + PV-safe LMP + RFP + Policy-LMR) (`exp/m32-nnue-mix9svq`)

Hotspot evidence & target:

- On 100-game Renju untimed `d8/w40`, `M3.2` (`9f6b899`) beat `master` `79-18-3` (`80.5%`, `LD=0, LL=0`), but dense 100+ ply endgames still exploded (`case22_ply109`: `19.96M` nodes / `47.9s`; `case22_ply114`: `43.48M` nodes / `87.0s` NNUE vs `94.56M` / `58.1s` classical `master`), averaging `2121 ms/move` vs `master` `1561 ms/move`.
- Target: eliminate eager Policy waste & repeated full-board replays (`P0`), accelerate NNUE inference via VEX `AVX-VNNI`, upgrade the NNUE TT to a 64B-aligned 4-way table caching `static_eval`, and add tactically-guarded modern pruning (`LMP + RFP + shift-invariant Policy-LMR`) to both beat `P0` significantly in `1s/move` AND beat classical `master` in `d8/w40` wall time.

Modification scope:

- **`P0` (`e68d198`)**:
  - `src/search/alphabeta.rs` & `src/search/root.rs`: `sync_config` hot-reloads NNUE when `(use_nnue, nnue_dir, rule_set)` changes with explicit load-failure warnings; deferred `evaluate_policy_moves()` until `index == 1` using `rotate_right(1)` (100% search-tree equivalent to eager Policy).
  - `src/protocol/gomocup.rs`: preserve full unquoted paths with spaces in `INFO nnue_dir`.
  - `src/bin/case_probe.rs`: added `--nnue-dir` / `--no-nnue` / `RUST_GOMOKU_NNUE_DIR` + `trace.nnue_active`.
  - `src/nnue/simd.rs` & `src/nnue/mix9svq.rs`: added runtime-detected VEX `AVX-VNNI` (`_mm256_dpbusd_avx_epi32`, avoiding EVEX `SIGILL` on non-AVX512 CPUs) with precomputed `signed_bias` for `star_block.down`; added LCP incremental `sync_with_board`; fixed scalar `evaluate_policy_moves` & AVX2 `policy_dot16_avx2`.
- **`P1` (`P1-1 + P1-2`, `P1b`)**:
  - `src/search/tt.rs`: added `#[repr(C, align(64))]` 4-way `TTBucket4` (4 × 16B, full 64-bit XOR key check + 64-bit payload storing `value`, `static_eval: i16`, `depth`, `best_move`, `flag`, `tt_pv`, `generation`) at identical memory footprint; same-key dedup preserving `best_move`/`static_eval` and deeper bounds; depth-0 entries never evict current-gen `depth > 0` slots; classical `use_nnue = false` keeps the 2-way path 100% untouched.
  - `src/search/alphabeta.rs`: wired `TTTableView4` + leaf `static_eval` reuse + tactically-guarded non-PV RFP (`depth <= 3.0`, `margin = 135 * d`), PV-safe non-PV LMP (`d <= 4.0`: `6/10/15/22`, guarded by `!win_priority && !single_forcing && !hostile_threat && current > -WIN && self_attack < 4 && opp_attack < 4`), and shift-invariant Policy-LMR (`gap = p_max - p_i > 1.5`, full-depth re-search on fail-high).

Verification & A/B benchmarks:

- Microbenchmark (Release): `AVX2 eval = 2.84 us -> AVX-VNNI eval = 1.93 us` (**`1.47x` / `-32.0%`**); `AVX-VNNI == AVX2 == Scalar` bit-identical; `cargo fmt --check`, `git diff --check`, full `cargo test` (zero warnings), `android/rust_bridge` 6/6, and `run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed**).
- Single-move slow probe (`case_probe --nnue-dir`, `d8/w40` Renju):
  - `case22_ply109`: `P0` `19.96M` / `47.90s` → `P1-1` (4-way TT only) `14.88M` / `33.95s` (`-29.1%`) → `P1-1+P1-2` `1.07M` / `3.15s` (**`15.2x` faster**).
  - `case22_ply114`: `P0` `43.48M` / `87.03s` (classical `master`: `94.56M` / `58.09s`) → `P1-1+P1-2` `16.04M` / `32.51s` (**`2.68x` faster than `P0`, `1.79x` faster than `master`**).
- 100-game center-opening matches (`center_50_prefixes.jsonl`, `--jobs 16 --reuse-engine-state`):
  - **`P1b` vs `P0` (`e68d198`) Renju `1s/move`**: **`59-39-2` (`60.0%`, `+70.44 Elo`, 95% CI `[+27.84, +115.24]`, `WW=10, WD=1, WL/DD=38, LD=1, LL=0`)**, `0` errors / `0` timeouts; median **`83.3 ms`** vs `390.0 ms` (**`4.68x` faster median**).
  - **`P1` vs `P0` (`e68d198`) Freestyle `1s/move`**: **`50-42-8` (`54.0%`, `+27.85 Elo`, `WW=4, WD=7, WL/DD=35, LD=1, LL=3`)**, `0` errors / `0` timeouts.
  - **`P1` vs `master` Renju `1s/move`**: **`84-15-1` (`84.5%`, `+294.61 Elo`, 95% CI `[+216.3, +409.5]`, `WW=34, WD=1, WL/DD=15, LD=0, LL=0`)**, `0` errors / `0` timeouts; avg **`453.7 ms`** / med **`8.0 ms`** (faster than classical `master` `469.2 ms` / `17.8 ms`).
  - **`P1b` vs `master` Renju untimed `d8/w40`**: **`77-20-2` (`78.57%`, `+225.71 Elo`, `WW=27, WD=2, WL/DD=20, LD=0, LL=0`)** (+ 1 `master`-driven 900s timeout on `case=37` where `P1b` max move was only `15.6s` vs `master` `40.9s`); avg **`630.6 ms`** vs `master` `1631.4 ms` (**`2.59x` faster than `master`, `3.36x` faster than `M3.2`**), p95 **`2675 ms`** vs `6902 ms`, max **`15.6s`** vs `40.9s`.

Decision:

- `P0 + P1b` achieves a massive dual-mode win: `+70.4 Elo` over `P0` in `1s/move` Renju (`LL=0`), `+27.9 Elo` over `P0` in `1s/move` Freestyle, `+295 Elo` over `master` in `1s/move`, and `78.57%` (`LD=0, LL=0`) while running **`2.59x` faster than classical `master`** in fixed-depth `d8/w40`. Merged into `master` (`27ce460`, tagged `baseline/nnue-p1b-27ce460`).

### 2026-11-23 (3): `P2` (Deep Zobrist-TT Attacker VCF + Match-Clock `timectl` + 4T ABDADA+NNUE Verification) (`exp/p2-tactics-timectl`)

Hotspot evidence & target:

- Legacy `VCFM = 5` caps root VCF search at 5 fours, forcing alpha-beta to rediscover 6–16-four forced wins in tactical midgames; `GomocupProtocol::current_search_limits` previously passed full `time_left_ms` directly as a single-turn limit in match-clock games; multi-threaded ABDADA + NNUE had not yet been benchmarked or match-verified.

Modification scope & stop-loss ablations:

- **`P2-1` (`src/threats/vcf.rs`, `src/search/root.rs`)**:
  - Added 64K-entry (1 MiB) generation-cleared Zobrist `VcfTt` and zero-allocation `search_deep_for_rule` (`max_depth = 16`, `node_limit = 20_000`, `3%` timed sub-budget) on the NNUE root attacker VCF path (`use_nnue = false` keeps `VCFM = 5` untouched).
  - **Stop-loss #1 (reverted)**: Proof-set opponent VCF filtering (`opp_depth = 12`) scored `45-52-3` (`46.5%`, `LL = 5`) vs `P1b` in `1s/move`. Root cause proved on `case6_ply16`: Black's winning move `(9, 7)` was neither in White's VCF proof set nor an immediate `Win5/A4/B4`, yet it turned Black's forced reply `(10, 8)` into a Black counter-four refuting White's VCF — proving proof-set filtering is unsound in Gomoku/Renju. Full-window `opp_depth = 6` filtering fixed `case6`/`case40` but added `+21 ms/move` overhead (`39-51-10`, `44.0%`). Reverted opponent VCF filter and VCT verify depth to `P1b` (`< 0.5 ms`).
- **`P2-2` (`src/protocol/gomocup.rs`)**:
  - Added dynamic match-clock pacing in `current_search_limits` when `use_nnue` is active and `time_left_ms < 50_000_000.0` (leaving pure `timeout_turn` and classical `base` protocol alignment 100% unchanged).
  - **Stop-loss #2 (reverted)**: Partial last-depth adoption on timeout dropped `1s/move` score from `46.5%` to `44.5%` (`41-52-7`, `LL = 6`) because adopting a partial fail-low move before remaining root candidates finish is unsound. Reverted.
- **`P2-3` (ABDADA + NNUE verification)**:
  - Confirmed `OnceLock<Arc<Mix9svqWeight>>` shares weights zero-copy across worker threads and `sync_worker_state` hot-reloads NNUE properly.

Verification & A/B benchmarks (vs `P1b` `27ce460`, `center_50_prefixes.jsonl`):

- Gates: `cargo fmt --check`, `git diff --check`, full `cargo test` (zero warnings), `android/rust_bridge` 6/6, and `run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed**).
- **`P2` (`1T`) vs `P1b` (`1T`) Renju `1s/move` (100 games, `--jobs 16 --reuse-engine-state`)**:
  - **`48-43-9` (`52.5%`, `+17.39 Elo`, 95% CI `[-17.51, +52.64]`, `WW=4, WD=2, WL/DD=39, LD=5, LL=0`)**, `0` errors / `0` timeouts; avg **`542.3 ms`** vs `P1b` `562.9 ms` (`-20.6 ms/move`, 91 fewer alpha-beta entries).
- **`P2` (`4T`) vs `P2` (`1T`) Renju `1s/move` (100 games, `--jobs 4 --reuse-engine-state`)**:
  - **`56-39-5` (`58.5%`, `+59.64 Elo`, 95% CI `[+9.12, +112.82]`, `WW=11, WD=2, WL/DD=32, LD=3, LL=2`)**, `0` errors / `0` timeouts; 4T avg **`488.2 ms`** / median **`56.2 ms`** vs 1T avg `523.9 ms` / median `1000.1 ms` (`2.29x` effective TT throughput).
- Single-move scaling (`cases/renju/vct_dense_ply78.json`, Renju `d8/w40`, NNUE):
  - 1T: `94,218,254 nodes` / `120.03s` (`move=[13,5]`, `score=754`) → 4T: `22,534,665 nodes` / `11.22s` (**`10.70x` speedup**, exact same move & score).

Decision:

- Keep attacker deep Zobrist-TT VCF (`depth=16`, `20k` nodes, `3%` timed sub-budget), match-clock `timectl`, and verified 4T ABDADA+NNUE. Ready to commit on `exp/p2-tactics-timectl` and merge into `master` upon user approval.

### 2026-11-25: `P3 / P4` NNUE Search Overhaul & Locked New Baseline (`f53a0f2`, `exp/p3-search-strength`)

Hotspot evidence & target:

- Fair head-to-head baseline (`p4_base`, `4065c80`, `opponent/baselines/gomocup_nnue_p4_base`) against same-weight Rapfi (`Mix9svq`, 32 MiB TT, Renju `1s/move`, `database=off`, `--reuse-engine-state`, 100 games on `center_50_prefixes.jsonl`) scored **`27.5%`** (`27-72-1`, `-168.4 Elo`, `LL=22`) on 1T and **`31.0%`** (`31-69-0`, `-139.0 Elo`) on 4T.
- Primary bottleneck identified via EBF & search-trace audit: nominal depth lagged Rapfi (`EBF 2.81` vs `2.09` on `c4_ply20`), NNUE PVS+LMR ran an extra full-window re-search at reduced depth and did not force `cut_node=true` on reduced LMR scouts, multi-threaded NNUE stalled worker cores on serial root df-pn VCT, and mate-band TT/QVCF edge cases leaked non-exact bounds.

Modification scope (`31a807e..f53a0f2`):

- **Correctness & mate-band hardening (`31a807e`, `956163f`, `6f0eecf`)**: kept QVCF narrow-window mate cutoffs as lower bounds, kept NNUE 4-way TT mate bounds exact-only with 4T confirmation guard, and bypassed QVCF delta pruning in the mate band when TT holds a mate lower bound.
- **`O(1)` tactical fast paths (`bb22ec3`, `25e8816`)**: added `O(1)` `has_double_four` / `has_four_three` / `has_double_three` bitmasks in `src/eval/caches.rs` (exhaustively verified on all 3,969 `shape_table.txt` entries with 0 false positives), non-root `quick_win_check_nnue` + Mate Distance Pruning, and O(1) `single_forcing` movegen bypass when `l5_count(-side) == 1`.
- **Multi-threaded NNUE root VCT policy (`4e13c53`)**: added `effective_compute_vct(nnue_active)` in `src/config.rs` to skip serial root df-pn VCT when `nnue_active && threads > 1 && !overlap_vct_alphabeta` (`+31.35 Elo` in 4T self-play, `54.5%`, `WW=10, LL=4`, while keeping root VCT on 1T and classical).
- **Rapfi-aligned pruning & PVS+LMR rewrite (`f1dea84`, `855bb36`, `f53a0f2`)**: aligned NNUE policy LMR with Rapfi `policyReduction`, aligned move-count pruning with Rapfi `futilityMoveCount`, rewrote NNUE PVS+LMR re-search flow to remove redundant reduced full-window re-searches and enforce `cut_node=true` on reduced LMR scouts, and raised `cut_node` LMR add to `1.80`.

Stop-loss reverts (do not repeat):

- Calling NNUE policy on `depth == 1` nodes dropped NPS `-9.7%` (`48.5%` 1T self-play) -> reverted.
- Fusing `MainHistory` + `CounterMove` into NNUE policy (`d0869c2`) saturated at `d10..14` and overrode NNUE policy (`23.5%` vs Rapfi 1T, `-5.5%` NPS) -> cleanly reverted (`34ea674`).
- `OPPO5_EXT = 1.1` on `single_forcing` reduced effective depth on `c4_ply20` and scored `27.0%–27.5%` vs Rapfi 1T -> reverted.

Verification & fair A/B vs Rapfi (`f53a0f2`, archived at `opponent/baselines/gomocup_nnue_p4_v2`):

- **Correctness**: `cargo test --locked` (379 tests), `android/rust_bridge` (6 tests), `git diff --check`, and classical `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed** in `29.5s`).
- **1s timed probe**: `c4_ply20 d12 -> d15` (`+3` plies over `p4_base`), `loss4t_c9_ply22 d14 -> d19` (`+5` plies, `307 KNPS`), `prior-10-white-11` solves `-19971` mate in `0.66s`.
- **1T vs Rapfi 1T (100g, Renju `1s/move`, 32 MiB TT)**: **`35.5% [29.16%, 41.84%]`** (`34-63-3`, `-103.73 Elo`, **`+64.7 Elo`** over `p4_base`, `LL: 22 -> 13`), avg `506.2 ms` / med `556.7 ms` / max `1001 ms`, `0` errors / `0` timeouts.
- **4T vs Rapfi 4T (100g, Renju `1s/move`, 32 MiB TT)**: **`34.0% [27.41%, 40.59%]`** (`33-65-2`, `-115.23 Elo`, **`+23.8 Elo`** over `p4_base`, `LL: 19 -> 15`), avg `500.5 ms` / med `401.7 ms` / p95 `1001.0 ms` / max `1001.9 ms` (faster than Rapfi avg `557.2 ms` / max `1065.4 ms`), `0` errors / `0` timeouts.

Decision:

- Lock `f53a0f2` (`opponent/baselines/gomocup_nnue_p4_v2`) as the new NNUE baseline (clear gains over `p4_base` on both 1T and 4T, zero classical diff regression) and integrate it into Desktop GUI and Android for the next release before resuming search-strength tuning.

### 2026-11-26: ARM64 NEON + DOTPROD SIMD & Bit-Exact NNUE `EvalCaches + movegen` Speedup (`7cf575d..98406c0`, `exp/neon-and-nnue-speed`)

Hotspot evidence & target:

- **Direction 1 (Android ARM64 NNUE SIMD)**: `src/nnue/simd.rs` and `src/nnue/mix9svq.rs` previously had x86_64 AVX2/AVX-VNNI kernels only, so Android `aarch64` builds fell back to scalar Mix9svq inference.
- **Direction 2 (Classical `EvalCaches + movegen` CPU overhead in NNUE mode)**: Prior `perf record` showed classical `EvalCaches + movegen` consumed `30.2%` CPU on `c4_ply20` (`42.7%` on `vct_dense_ply78`), including unused classical `dgn` tracking, 608B `EvalSnapshot` copies (`368B` `empty_bucket_counts`), redundant QVCF coverage maintenance, per-node `Vec<Candidate>` allocations, redundant Renju `classify_forbidden_move` calls in movegen, and `LazyLock<Box<[[i32; 3969]; 2]>>` (`31.8 KiB`) atomic checks in `SHAPE_TABLE`.

Modification scope & net-gain hypothesis:

- **Direction 1 (`7cf575d`, `src/nnue/simd.rs`, `src/nnue/mix9svq.rs`)**:
  - Implemented all 11 bit-exact `aarch64` NEON kernels (`mulhi_i16_neon`, `dot2_u7i8_neon`, `linear_i8_chunk4_neon`, `linear_i8_row4_neon`, `linear_i16_chunk2_prerelu_neon`, `policy_dot16_neon`, `avg_pool_2x2_u8_neon`, `update_point_neon`, `commit_conv_cell_neon`, `commit_vsum_neon`, `star_block_neon`) plus runtime-dispatched ARMv8.2 `dotprod` (`sdot`) variants (`is_aarch64_feature_detected!("dotprod")`, stable inline asm `sdot {acc:v}.4s, {a:v}.16b, {b:v}.16b`).
  - Disassembly of release `librust_gomoku_android.so` confirmed `84 sdot`, `352 smull`, `124 smull2`, `380 sqxtn`, `348 sqxtn2`, `120 urhadd`, `135 uzp2`, `99 addp`, `80 sadalp`.
- **Direction 2 (5 bit-exact atomic commits)**:
  - **Item 2A (`4e7badb`)**: Monomorphized `value_wide_compute_for_rule_no_dgn` (`TRACK_DGN = false`) in `src/eval/local.rs` and wired NNUE search (`search_with_coverage`, `vcfsearch`, `vcfdefend`) to skip unused `dgn_cc`/`dgn_sum`/`blocked_neighbor_count` tracking (`+7.4%` KNPS alone).
  - **Item 2B (`58aeb37`)**: Added 12B `threat_counts: [[u8; 6]; 2]` + lightweight `EvalSnapshotNnue` (`176B` vs `608B`, `-71%` snapshot copy) in `src/eval/caches.rs`, letting NNUE mode skip `empty_bucket_counts` and `nonzero_bucket_mask` updates (`+5.3%` additional KNPS).
  - **Item 2C (`bcd74af`)**: Shrunk `CoverageTracker::counts` from `u16` to `u8` (`450B -> 225B`) and dropped unused `coverage` updates from `vcfsearch`/`vcfdefend`.
  - **Item 2D (`a9233ab`)**: Added `AlphaBetaSearcher::candidate_pool` (zero per-node `Vec<Candidate>` heap alloc), single-pass `no_l4_l5` movegen, and `O(1)` Renju black legality via `caches.value_cache[0][x][y] != 0` in `src/search/movegen.rs`.
  - **Item 2E (`98406c0`)**: Replaced `LazyLock<Box<[[i32; 3969]; 2]>>` (`31.8 KiB`) with compile-time `.rodata` `static SHAPE_TABLE: [[u8; 3969]; 2]` (`7.9 KiB`, `4x` smaller), compile-time `u8` `COVER_NEIGHBORS` (`14.4 KiB -> 7.2 KiB`), and compile-time `QVCF_NEIGHBOR_MASKS` (`+1.45%` additional KNPS).

Stop-loss reverts (do not repeat):

- Journaling `empty_bucket_counts` deltas into `bucket_log` was `-20.6%` slower (`581.8ms -> 732.6ms`) because 40–60 log pushes/pops cost far more than a 368B AVX2 memcpy -> reverted in favor of `threat_counts` + `EvalSnapshotNnue`.
- Adding `covered_mask` bitboard branches inside `CoverageTracker::add_move`/`remove_move` added `+47ms` overhead across leaf/QVCF nodes -> reverted.
- Inlining `SHAPE_DIR_TABLE` (`1 KiB`) into `update_bucket_attack` was neutral/slightly slower (`549.5ms -> 552.6ms`) due to code bloat -> reverted.
- Passing `&mut Board` into `value_wide_compute_for_rule` for in-place forbidden checks regressed `vct_dense_ply78` by `+6.2%` due to aliasing inhibition -> reverted.

Verification & benchmarks:

- **Fixed-depth bit-exactness & P-core pinned 7-repeat interleaved A/B (`7cf575d` vs `98406c0`, 8 Renju+Freestyle cases, `276,472` nodes)**:
  - **`0` move / score / depth / node mismatches** across all 8 cases.
  - Aggregate alphabeta time: **`700.55ms -> 604.08ms` (`-13.77%` wall time, `+15.97%` KNPS: `394.7k -> 457.7k`)**, winning `+12.6% ~ +18.9%` KNPS on every case (`vct_dense_ply78 +18.67%`, `g7_case7 +18.85%`, `g2_case1 +14.56%`, `freestyle_g2 +13.18%`).
- **Correctness gates**: `cargo fmt --check`, `git diff --check`, full `cargo test --locked` (all passing, 0 warnings), `android/rust_bridge` 7/7, `cargo ndk -t arm64-v8a build --release` (0 warnings), `./gradlew test lint assembleDebug`, and classical `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed** in `31.5s`).
- **Head-to-head 1T self-play (`dir2_1t` vs `p4_v2_1t`, 100g, Renju `1s/move`, 32 MiB TT, `j=8`, `--reuse-engine-state`)**:
  - **`50-43-7` (`53.5% ± 4.79%`, `+24.36 Elo`, `WW=4, WD=2, WL/DD=41, LD=3, LL=0`)**, `0` errors / `0` timeouts; `dir2_1t` avg **`468.0 ms`** / median **`81.5 ms`** vs `p4_v2_1t` avg `497.1 ms` / median `422.0 ms`.
- **Fair vs Rapfi (`Mix9svq`, 32 MiB TT, Renju `1s/move`, 100g, `--reuse-engine-state`)**:
  - **1T (`j=10`)**: `32-65-3` (`33.5% ± 6.57%`, `-119.11 Elo`, `LL=15`, 1 ply-224 black-no-legal-move loss, 0 real errors / 0 timeouts; avg `524.1 ms` / p95 `1000.5 ms` / max `1001.0 ms`).
  - **4T (`j=4`)**: `32-66-2` (`33.0% ± 6.69%`, `-123.02 Elo`, `LL=16`, 0 errors / 0 timeouts; avg **`489.9 ms`** / median **`279.7 ms`** / p95 `1000.9 ms` / max `1007.5 ms` vs `p4_v2_4t` avg `500.5 ms` / median `401.7 ms`).

Decision:

- Keep all 6 commits (`7cf575d..98406c0`) on `exp/neon-and-nnue-speed`: 100% search-tree equivalent in both classical and NNUE, `+15.97%` x86_64 KNPS, `+24.36 Elo` (`53.5%`, `0 LL`) in 1T self-play vs `p4_v2`, and full ARM64 NEON+DOTPROD SIMD ready for Android. Ready to merge into `master` upon user approval.


### 2026-11-26: Timed Search Max Depth `25 -> 48` & 1T Mate-Confirmation Ablation (`exp/timed-depth48-and-mate-confirm`)

Hotspot evidence & target:

- Midgame move-time audit on `dir2_4t_vs_rapfi_4t_center50.json` (`1s/move`, `--reuse-engine-state`) found that `20/20` sampled non-forced-4 midgame `<20ms` moves were warm-TT `win_mate`/`loss_mate` continuations where `+10` `SUBPLY_TT_WIN_BONUS` pushed entries past `DEFAULT_TIMED_SEARCH_MAX_DEPTH = 25`, and `5/20` already hit `d25` even on cold probes.
- Verified NNUE subply TT depth encoding (`SUBPLY_TT_SCALE_F64 = 64.0`, `NNUE_MAIN_TT_DEPTH_OFFSET = 3`, `SUBPLY_TT_DEPTH_CAP = 3200`, `SUBPLY_TT_WIN_BONUS = 640`, `DEPTH_MAX = 4095`) safely supports up to `d49` without TT format changes or cutoff saturation (`48 * 64 + 3 = 3075 <= 3200`, `+640 = 3715 <= 4095`).

Modification scope & stop-loss:

- **Item 1 (kept)**: Raised `DEFAULT_TIMED_SEARCH_MAX_DEPTH` from `25` to `48` in `src/config.rs` and synced `README.md`. Zero TT encoding change; classical fixed-depth `base` untouched.
- **Item 2 (reverted — stop-loss)**: Tested adding 4T-style NNUE mate confirmation (`WIN_MATE_CONFIRM_ITERS = 2`, `LOSS_MATE_CONFIRM_ITERS = 6`, `tt_only_iter` filter) to the 1T iterative-deepening loop in `src/search/root.rs`. Incremental 1T result was neutral (`48-47-5`, `50.5% ± 4.48%`, `+3.47 Elo`, `WW=2, WD=2, WL/DD=42, LD=3, LL=1`) while adding `+33.9 ms` avg move time (`477.1 ms -> 511.0 ms`, median `57.9 ms -> 678.9 ms`) and ~35 lines of state-machine duplication -> cleanly reverted.

Verification & 100-game matches (Renju `1s/move`, 32 MiB TT, `center_50_prefixes.jsonl`, `--reuse-engine-state`):

- **Correctness**: `cargo fmt --check`, `git diff --check`, full `cargo test --locked`, and classical `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed** in `29.2s`).
- **`exp_d48_1t` vs `v0118_1t` (`j=8`)**: **`51-41-8` (`55.0% ± 5.98%`, `+34.86 Elo`, `WW=7, WD=1, WL/DD=38, LD=3, LL=1`)**, `0` errors / `0` timeouts; avg `492.5 ms` / med `112.6 ms` / p95 `1000.4 ms` / max `1000.8 ms` vs `v0118_1t` `482.5 ms`.
- **`exp_d48_4t` vs `v0118_4t` (`j=4`)**: `45-47-8` (`49.0% ± 5.14%`, `-6.95 Elo`, `WW=2, WD=3, WL/DD=38, LD=5, LL=2`, 200-game 1T+4T combined `96-88-16` / `52.0%`), `0` errors / `0` timeouts; avg `539.2 ms` / med `1000.4 ms` / p95 `1000.9 ms` / max `1007.3 ms` vs `v0118_4t` `466.0 ms`.
- **`exp_d48_1t` vs Rapfi 1T (`j=10`, fair `Mix9svq`, 32 MiB TT)**: **`32-63-5` (`34.5% ± 6.32%`, `-111.37 Elo`, `WW=0, WD=0, WL/DD=32, LD=5, LL=13`)**, `+1.0%` (`+7.74 Elo`, `LL: 15 -> 13`) over `v0.1.18 1T` (`33.5%`, `32-65-3`, `LL=15`); 1 ply-224 Renju black-no-legal-move loss, `0` real errors / `0` timeouts; avg `537.1 ms` / med `1000.2 ms` / p95 `1000.5 ms` / max `1000.9 ms` vs Rapfi `657.2 ms` / max `1052.6 ms`.
- **`master_d48_4t` vs Rapfi 4T (`j=4`, fair `Mix9svq`, 32 MiB TT, 2×100g = 200g)**:
  - Run 1: `30-68-2` (`31.0% ± 6.86%`, `-138.99 Elo`, `LL=18`, 0 errors/timeouts; 4/5 worse games diverged at ply 4–10 from Rapfi 4T's own nondeterminism).
  - Run 2: **`31-63-6` (`34.0% ± 6.31%`, `-115.23 Elo`, `LL=13`, 0 errors/timeouts; avg `559.2 ms` / p95 `1000.9 ms` / max `1001.6 ms` vs Rapfi `623.0 ms` / max `1079.3 ms`)**.
  - 200-game combined: `61-131-8` (`32.5%`, on par with `v0.1.18` `33.0%`).

Decision:

- Keep `DEFAULT_TIMED_SEARCH_MAX_DEPTH = 48` only; revert 1T mate confirmation; merge into `master`.


### 2026-11-26: Native Rapfi-Aligned NNUE Engine Core (`src/fast_engine/`, `exp/fastboard-nnue-core`)

Hotspot evidence & target:

- Forensic autopsy of `master_d48` vs fair same-weight `pbrain-rapfi` (`Mix9svq`, 32 MiB TT, `1s/move`, `center_50_prefixes.jsonl`) traced the ~115 Elo deficit (`34.5%` 1T / `32.5%` 4T) to two structural limits of the legacy classical-hosted NNUE path:
  1. Legacy `Board + EvalCaches` (`value_wide_compute_for_rule_no_dgn` + 176B `EvalSnapshotNnue` + `CoverageTracker`) capped tactical midgame throughput at `250K–425K NPS` vs Rapfi's `350K–808K NPS`.
  2. Legacy 7-tier `attack_level` could not represent 14-level `Pattern4` (`B_FLEX4`, `C_BLOCK4_FLEX3`, `D_BLOCK4_PLUS`, `F_FLEX3_2X`, `J_FLEX2_2X`, `FORBID`), blocking exact `DEFENDFOUR`/`DEFENDB4F3`, `OPPO5_EXT = 1.1`, `extraExtension`, `lmrExtension`, and false-forbidden LMR adjustments.
- Target: build an isolated native Rust NNUE engine core (`src/fast_engine/`) matching Rapfi ~50/50 under strictly equal conditions (both 1T & 4T, Renju & Freestyle) while leaving classical `pygomoku` `base` (`use_nnue = false`) 100% untouched.

Modification scope:

- **`src/fast_engine/pattern.rs` & `src/fast_engine/board.rs` (M1)**: Zero-allocation 15×15 `FastBoard` with 4-direction 64-bit rotated bit keys, dense-key `Pattern` / 14-level `Pattern4` tables (`14,641` Freestyle / `132,496` Renju), exact Renju forbidden point detector (verified 100% against all hand fixtures + 1,000 dense random boards / ~180k empty cells, including perpendicular exact-five overriding double-three), incremental `Bitboard225` candidate set (`SQUARE3_LINE4`), flat `u32` `MoveRestore`, and `quick_win_check`.
- **`src/fast_engine/{movegen,movepick,history,tt,search}.rs` (M2–M3)**:
  - Exact `Pattern4` move generators (`ALL`, `WINNING`, `DEFEND_FIVE`, `DEFEND_FOUR`, `DEFEND_B4F3`, `VCF`, `RANGE_SQUARE2_LINE4` neighbor `QVCF`, `validate_opponent_c_move`).
  - 5-stage `MovePicker` with `MaybeUninit` `MoveList` & `HistoryTracker` (eliminating ~3.2 KiB per-node stack zeroing), `MainHistory(ATTACK/QUIET)` + `CounterMove` + `Killers`.
  - Native AB + QVCF search (`Aspiration Window`, `Razoring`, `RFP`, `NMP` via `Mix9svq` dual-accumulator zero-cost pass, `IIR/IID`, `FHR`, `LMP/Trivial/Policy/Distract` pruning, `OPPO5_EXT = 1.1`, `Singular/Double SE`, `lmrExtension`, full `compute_lmr_reduction`).
  - Lockless 12B/entry 5-tier 64B-aligned XOR `FastTT` (`mulhi64` range mapping, integer ply depth `-20..235`, +25% entries in 32 MiB) + pure Lazy SMP multi-threading (`threadBias`, `pick_next_depth`, `pick_best_thread`).
- **`src/app.rs`, `src/protocol/gomocup.rs`, `src/bin/gomocup_engine.rs` (M4)**:
  - Routed NNUE mode (`use_nnue = true`) to `fast_engine` by default in `GomocupProtocol` (with `--legacy-nnue` / `INFO fast_nnue 0` fallback) and `GameController` (shared by Desktop GUI and Android JNI bridge, preserving `NnuePositionEstimate` win/draw/loss curve sampling). Classical `base` (`use_nnue = false`, Casual mode, and diff suites) remains 100% untouched.

Micro-benchmarks & probes:

- Release `play + undo` bench (200k iters, Renju): `FastBoard` **`179.2 ns/op`** vs legacy `Board + EvalCaches` `658.2 ns/op` (**`3.67x` faster**).
- Fixed `d15` on `OPEN_5`: **`55.7 ms / 18,680 nodes`** (vs `pbrain-rapfi` `63 ms / 28K nodes`); 1s 4T `OPEN_5`: **`d22-37 / 1.39M NPS`** (vs Rapfi `d20-40 / 1.63M NPS`).

300-game fair head-to-head vs `pbrain-rapfi` (`Mix9svq`, 32 MiB TT, `1s/move`, `database=off`, `center_50_prefixes.jsonl`, `--reuse-engine-state`):

- **Renju 1T (`j=10`, 100g)**: **`43-44-13` (`49.5% ± 3.82%`, `-3.47 Elo`, CI `[-30.1, +23.1]`, `WW=0, WD=5, WL/DD=39, LD=6, LL=0`)**, `0` errors / `0` timeouts; avg `732.4 ms` / med `970.4 ms` / p95 `978.5 ms` / max `985.7 ms` (vs Rapfi `727.2 / 970.5 / 975.7 / 1054.2 ms`). Gain over `master_d48_1t` (`34.5%`, `LL=13`): **`+107.9 Elo`**.
- **Renju 4T (`j=4`, 100g)**: **`42-41-17` (`50.5% ± 3.58%`, `+3.47 Elo`, CI `[-21.4, +28.4]`, `WW=1, WD=2, WL/DD=44, LD=3, LL=0`)**, `0` errors / `0` timeouts (1 Rapfi ply-224 overline loss); avg `707.2 ms` / med `970.2 ms` / p95 `980.5 ms` / max `989.3 ms` (vs Rapfi `705.3 / 970.8 / 977.4 / 1065.7 ms`). Gain over `master_d48_4t` (`32.5%`): **`+130.4 Elo`**.
- **Freestyle 1T (`j=10`, 100g)**: **`44-45-11` (`49.5% ± 3.02%`, `-3.47 Elo`, CI `[-24.5, +17.5]`, `WW=0, WD=2, WL/DD=45, LD=3, LL=0`)**, `0` errors / `0` timeouts; avg `716.4 ms` / med `970.2 ms` / p95 `978.4 ms` / max `987.1 ms` (vs Rapfi `706.4 / 970.3 / 975.5 / 1015.3 ms`).
- **Tactical prefixes (`strength_100_prefixes.jsonl`, Renju 1T, 100g)**: **`49-49-2` (`50.0% ± 2.53%`, `0.0 Elo`, `LL=0`, `0` errors / `0` timeouts)**.
- **300-game `center_50` combined**: **`129-130-41` (`49.83%`, `-1.16 Elo`)**, **`0 / 150` lost pairs (`LL = 0`)**, `0` engine errors, `0` timeouts across `9,766` moves.

Correctness gates:

- `cargo fmt --check`, `git diff --check`, full `cargo test --locked` (0 warnings), `android/rust_bridge` (7/7), `cargo ndk -t arm64-v8a build --release --locked` (0 warnings), `android/ ./gradlew test lint assembleDebug`, and classical `scripts/run_diff.py --profile all --check-baselines --jobs 8` (**13/13 passed** in `30.0s`).

Decision:

- Ready to merge `exp/fastboard-nnue-core` into `master` upon user approval.
