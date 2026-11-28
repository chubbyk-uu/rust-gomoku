# Classical search, ABDADA and tactical bitboards

> Snapshot from the former [perf-log.md](../../perf-log.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
