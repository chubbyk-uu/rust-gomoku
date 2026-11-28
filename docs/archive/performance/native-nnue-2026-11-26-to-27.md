# Native NNUE migration and Rapfi validation

> Snapshot from the former [perf-log.md](../../perf-log.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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

### 2026-11-27: Pure-CPU `Mix9svq` SIMD + Hybrid Classical Gate + `CandArea` Exact Parity (`exp/rapfi-60pct-phase1`, `012ec27..14b3424`)

Hotspot evidence & target:

- After `exp/fastboard-nnue-core` (`master @ b7d750e`) reached 50/50 parity with `pbrain-rapfi` (`49.83%`, `LL=0`, `407K` 1T / `1,886K` 4T NPS on `mid10`), component profiling showed NNUE inference (`play_move` + `star_block` + `policy_eval`) and move picking dominated CPU time, while 3 subtle search/eval divergences remained vs Rapfi:
  1. `futility_margin` in `src/fast_engine/search.rs` missed `.max(0)` vs Rapfi `parameter.h:125`, yielding negative margins `-125..-1` on `0 < d < 2.0, improving = true`.
  2. `evaluate_static` always invoked NNUE `evaluate_value` (no `model210901.bin` classical hybrid gate) and `QVCF` lacked learned `P4SCORES` + `POLICY_CROSS` scoring.
  3. `FastBoard::empty_candidates()` lacked Rapfi's `CandArea` bounding-box clip (`expand_dist = 3`), generating ~8–15 extra `dist=4` perimeter moves per node, and iterative deepening started at `d=1` instead of `d=2`.
- Target: maximize pure-CPU throughput under identical May 2025 `Mix9svq` weights and achieve exact single-node search parity with `pbrain-rapfi` so the raw CPU speed edge converts cleanly into win-rate gain across Renju & Freestyle, 1T & 4T, `0` errors / `0` timeouts, while leaving classical `base` (`src/search/`, `use_nnue = false`) 100% untouched.

Modification scope:

- **Bit-exact `Mix9svq` SIMD & memory layout (`012ec27..208153f`, `src/nnue/mix9svq.rs`, `src/nnue/simd.rs`, `src/fast_engine/movepick.rs`)**:
  - `play_move_avx2` + `STAR_CONV_MASK` (~35% fewer `map_conv` ops) + VQ zero-delta skip + `star_block_batch4_avxvnni` + `policy_eval_batch4_avx2` + 64B `AlignedRow<N>` + 2-pass prefetch + `HugeBox<T>` (2 MiB THP) + lazy top-6 `MovePicker` + ARM64 NEON sync.
- **Correctness fix, hybrid classical gate, & root symmetry (`036aab6`, `src/fast_engine/{classical_model,board,movepick,search}.rs`, `src/nnue/mod.rs`)**:
  - Fixed `futility_margin` `.max(0)` clamp (`futility_margin_clamps_negative_to_zero`).
  - Added lock-free `ClassicalModel` loader (`model210901.bin`, `RFCLMOD1` v8, automatic warm in `load_mix9svq_evaluator_for_rule` with clean `None` fallback when absent; zero binaries committed), incremental `StateInfo::value_black` + `FastBoard::evaluate_classical<RENJU>` (verified bit-exact across 40 random play/pass/undo trials), hybrid `classicalEvalMargin` lazy NNUE gate across all 9 `evaluate_static` call sites, and learned `ClassicalPolicyScorer` (`P4SCORES` + `POLICY_CROSS`) for `QVCF`.
  - Added lossless root dihedral `D4` symmetry deduplication (`filter_root_symmetry_moves`, matching Rapfi `opening.cpp:250-301`).
- **`CandArea` bounding-box clip & `start_depth = 2` (`14b3424`, `src/fast_engine/{board,search}.rs`)**:
  - Added compile-time L1-resident `CAND_COL_MASK` / `CAND_ROW_MASK` tables (`2 ns`, branchless) clipping `empty_candidates()` to Rapfi `CandArea` (`expand_dist = 3`).
  - Changed iterative deepening start depth from `1` to `2.min(max_depth)` (matching Rapfi `searchcommon.h:137`).

Stop-loss reverts (do not repeat):

- `CorrectionHistory` (`47.5%`), `ContinuationHistory` (`49.0%`), un-aged persistent `MainHistory` (`47.0%`), LLVM PGO (`< 1–2%`), `3A` cut-node `-1.5` SE, `3B` `0.99` time ratio (`46.5%`), `3C` `HELPER_LMR_DELTA` (`50.5%`), `3D-1` Cut-node IIR, `3D-2` `depth8 - 8*rel_age` (`49.5%`), `3D-3` Forward FP at `d < 4` (regressed pure Renjunet `old_50` at `1s/move` vs Rapfi -> cleanly reverted in `036aab6`).

Single-node parity & component timing (`14b3424`, 32 MiB TT, 1s warm):

- **Exact single-node parity vs `pbrain-rapfi`** (`case 22 ply 16`, Renju 1T): `d1` (`295 == 295`, `650 == 650`), `d2` (`249 == 249`, `859 == 859`), `d6` (`1405 == 1405`, `624 == 624`), `d10` (`4363 == 4363`, `462 == 462`); at `1s` Rust reaches **`872K NPS`** finding **`+M31`** (`862ms`) vs Rapfi's `692K NPS` **`+M37`** (`883ms`).
- **`fast_engine_component_timing`**:
  - `mid10 1T`: **`730.4K NPS`** (`+79.5%` vs `master` `407K`, **`1.59x`** Rapfi `460K`)
  - `mid10 4T`: **`3,654.0K NPS`** (`+93.7%` vs `master` `1,886K`, **`1.68x`** Rapfi `2,180K`)
  - `open4 1T`: **`624.9K NPS`** (`+87.6%` vs `master` `333K`, **`1.49x`** Rapfi `418K`)
  - `open4 4T`: **`2,535.0K NPS`** (`d22/39`); fixed `d15 open4`: `20.46 ms` (`9,762` nodes).

600-game final fair validation vs `pbrain-rapfi` (`14b3424`, identical `Mix9svq` + `model210901.bin`, `1s/move`, `32 MiB TT`, `database=off`, `--reuse-engine-state`, **0 errors / 0 timeouts**):

| Matchup | Games (Pairs) | W-L-D | Score % (95% CI) | Elo | Pentanomial (`WW/WD/WL+DD/LD/LL`) | Rust Timing (`avg / med / p95 / max ms`) | Rapfi Timing (`avg / med / p95 / max ms`) | Err / TO |
| --- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| Renju 1T (Total) | 200 (100) | `87-80-33` | **`51.75% ± 3.19%`** | `+12.17` | `3 / 13 / 73 / 10 / 1` | `729.2 / 970.0 / 975.3 / 986.9` | `738.7 / 970.5 / 974.9 / 1043.1` | `0 / 0` |
| — Renju 1T `old_50` (Renjunet) | 100 (50) | `45-44-11` | `50.50% ± 2.97%` | `+3.47` | `LL = 0` | — | — | `0 / 0` |
| — Renju 1T `new_50` (balanced) | 100 (50) | `42-36-22` | `53.00% ± 5.36%` | `+20.88` | `LL = 1` | — | — | `0 / 0` |
| Renju 4T (Total) | 200 (100) | `85-81-34` | **`51.00% ± 2.77%`** | `+6.95` | `3 / 7 / 81 / 9 / 0` | `715.9 / 970.3 / 979.7 / 1000.3` | `728.8 / 971.5 / 979.4 / 1075.5` | `0 / 0` |
| — Renju 4T `old_50` (Renjunet) | 100 (50) | `43-41-16` | `51.00% ± 3.12%` | `+6.95` | `1 / 3 / 43 / 3 / 0` | — | — | `0 / 0` |
| — Renju 4T `new_50` (balanced) | 100 (50) | `42-40-18` | `51.00% ± 4.19%` | `+6.95` | `2 / 4 / 38 / 6 / 0` | — | — | `0 / 0` |
| Freestyle 1T | 100 (50) | `37-39-24` | `49.00% ± 4.77%` | `-6.95` | `0 / 7 / 36 / 5 / 2` | `769.9 / 970.3 / 976.2 / 986.1` | `780.2 / 970.8 / 975.9 / 1021.5` | `0 / 0` |
| Freestyle 4T | 100 (50) | `42-39-19` | **`51.50% ± 4.26%`** | `+10.43` | `1 / 6 / 38 / 5 / 0` | `726.4 / 970.4 / 979.6 / 989.8` | `754.0 / 971.8 / 979.6 / 1026.3` | `0 / 0` |
| **Combined 4T (Renju + Freestyle)** | **300 (150)** | **`127-120-53`** | **`51.17%`** | **`+8.11`** | **`4 / 13 / 119 / 14 / 0` (ZERO LL)** | — | — | **`0 / 0`** |
| **Aggregate 600g Total** | **600 (300)** | **`251-239-110`** | **`51.00% ± 1.65%`** | **`+6.95`** | **`7 / 33 / 228 / 29 / 3`** | **`avg -10..-28 ms`, `max -35..-75 ms` vs Rapfi** | — | **`0 / 0`** |

*(Note: combined post-fix Renju 1T across `036aab6` + `14b3424` = `400g`, `177-162-61` = `51.88%`, `+13.0 Elo`.)*

Decision:

- `exp/rapfi-60pct-phase1` (`14b3424`) delivers `+79.5%..+93.7%` NNUE NPS over `master` (`1.49x–1.68x` `pbrain-rapfi`), achieves exact single-node `d1..d10` parity with Rapfi, beats `pbrain-rapfi` overall across 600 fair games (`51.00% ± 1.65%`, `0` errors / `0` timeouts, `0 LL` across all 300 4T games, consistently lower avg and max turn latency), and keeps classical `base` (`13/13` diffs) untouched. Held on `exp/rapfi-60pct-phase1` pending user approval to merge into `master`.
