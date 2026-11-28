# Classical hot paths, df-pn and legacy NNUE

> Snapshot from the former [perf-log.md](../../perf-log.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
