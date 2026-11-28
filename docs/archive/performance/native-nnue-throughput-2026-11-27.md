# Native NNUE throughput experiments

> Snapshot from the former [perf-log.md](../../perf-log.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
- **Step 3D-2 (rejected & reverted) — Aggressive TT replacement aging (`depth8 - 8 * rel_age`)**:
  - 200-game 1T Renju screen vs `gomocup_3d1` (`renju_100pairs.jsonl`, `300ms/move`, 8 MiB TT, `--reuse-engine-state`): `82-84-34` (`49.5% ± 3.59%`, `-3.47 Elo`, `WW=4, WD=9, LD=13, LL=3`). Penalizing `rel_age == 1` by 8 plies prematurely evicted useful previous-move PV entries; reverted cleanly.
- **Step 3D-3 (kept, `4d2430d`) — Forward Futility Pruning in move loop** (`!pv_node && !mt.important && oppo4 == 0 && move_count > 1 && depth < 4.0 && eval != VALUE_NONE && eval + 90 + (75.0 * depth) as i32 <= alpha`):
  - 200-game 1T Renju screen vs `gomocup_3d1` (`renju_100pairs.jsonl`, `300ms/move`, 32 MiB TT, `--reuse-engine-state`): **`93-80-27` (`53.25% ± 4.06%`, `+22.62 Elo`, `WW=8, WD=12, WL/DD=69, LD=7, LL=4`)**, `avg 222.5ms` vs `222.5ms`, `0` errors / `0` timeouts.
- **Step 3C (rejected & reverted) — 4T Helper LMR Delta** (`HELPER_LMR_DELTA = [0.0, 0.20, -0.20, 0.10]` for `tid > 0`):
  - 100-game 4T Renju screen vs `gomocup_3d3` (`center_50_prefixes.jsonl`, `300ms/move`, 32 MiB TT, `--reuse-engine-state`, `--jobs 3`): `41-40-19` (`50.5% ± 4.68%`, `+3.47 Elo`, `WW=0, WD=9, LD=6, LL=1`), `0` timeouts (1 ply-224 full-board `ERROR No legal move.` on `rust_3d3_4t`). Below the `>= 51.0%` retention gate; cleanly reverted.
