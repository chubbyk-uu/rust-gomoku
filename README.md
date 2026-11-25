# rust_gomoku

`rust_gomoku` is a Rust rewrite of the classic mainline of the Python reference project `pygomoku`, with optional Renju forbidden-move support. Current work preserves both rule modes while reducing search latency and improving playing strength through measured experiments.

A Chinese version of this document is available at [README-cn.md](README-cn.md).

## Screenshots

Download the Windows x64 program and signed Android ARM64 APK / AAB from the [latest release](https://github.com/chubbyk-uu/rust-gomoku/releases/latest).

### Windows / Desktop GUI

The Windows program opens this local browser interface. Play against the engine or another person, choose freestyle or Renju, switch between Pro mode (NNUE `Mix9svq` with per-move time budgets `500 ms`–`5000 ms`, default `1000 ms`) and Casual mode (four classical fixed-depth presets), and toggle multi-threaded search from the side panel.

![Desktop GUI showing a game against the engine and the rule, thinking-mode, difficulty, and multi-thread controls](docs/screenshots/desktop-gui.png)

### Android

The mobile interface puts the board, turn status, and undo/new-game controls together. It bundles the `Mix9svq` NNUE weights and defaults to Pro timed mode (`500 ms` / `1000 ms` / `2500 ms` / `5000 ms`, default `1000 ms`, unified `tt_bits = 20` / 32 MiB TT), while also offering Casual fixed-depth mode (Beginner / Normal / Senior / Master) and a multi-threaded search toggle.

<p>
  <img src="docs/screenshots/android-portrait.png" alt="Android interface in portrait orientation during a Renju game" width="320">
  <img src="docs/screenshots/android-difficulty.png" alt="Android settings preview from an earlier release" width="320">
</p>

Landscape mode places the controls beside the board:

![Android interface in landscape orientation](docs/screenshots/android-landscape.png)

Capture notes: the desktop screenshot was taken from the running GUI in WSL, which serves the same page as the Windows build. Android images are browser previews from an earlier release using a real Rust-controller snapshot, without Android system bars; current builds use the Pro (`500 ms`–`5000 ms`, default `1000 ms`) + Casual (4-level) segmented controls.

## Current Status

Completed:

- 15x15 freestyle and Renju state machine, zobrist, config, pattern, eval cache, movegen, ordering, TT, and alpha-beta root search.
- Renju black forbidden-move handling for overline, double-four, and recursive true double-three, with exact-five priority; freestyle remains the default.
- Rule-aware root VCF (including deep 16-ply Zobrist-TT attacker VCF under NNUE) and `df-pn` VCT solver (`src/threats/dfpn.rs`) with `Pattern4` incremental bitboard/LUT tactical acceleration (`5.5x`–`8.6x` faster on dense Renju VCT6).
- Pure-Rust incremental `Mix9svq` NNUE evaluation (compatible with Rapfi CC0 `.bin.lz4` weights) with Value + Policy ordering, AVX2 / VEX AVX-VNNI SIMD on x86_64, 64B-aligned 4-way TT caching `static_eval`, Rapfi-calibrated `policyReduction` / `futilityMoveCount` / `PVS+LMR`, `O(1)` quickWinCheck & single-forcing fast path, leaf QVCF, and asynchronous Lazy SMP multi-threaded NNUE search.
- Multi-threaded ABDADA classical search (`--threads N` / `INFO thread_num N`) with `1/64`-ply sub-ply TT depth encoding and mode-isolated dual TT bucket planes.
- Gomocup stdin/stdout engine entry point, local Web GUI (`gomoku_gui`), and signed Android ARM64 APK / AAB with bundled `Mix9svq` weights and a shared Rust `GameController`.
- Rust/reference diff harness (`13/13` classical baseline checks passing), head-to-head Gomocup match runner, Renju oracle/stress tooling, and same-position benchmark scaffolding.
- External reference engines such as SlowRenju and Rapfi are used from local checkouts when needed; bundled lightweight opponents are not committed.

Optional diagnostics / experiments:

- `overlap_vct_alphabeta`: experimental switch to overlap VCT with alpha-beta after a VCF miss.
- `root_profile`: per-root-candidate timing diagnostics.
- TT generation observation: Gomocup trace can report the new/old ratio of cross-move TT best-move hints. It does not affect TT cutoff, replacement, or ordering policy.

To shorten the perceived wait during single-threaded manual GUI play, the GUI entry enables `overlap_vct_alphabeta` when `threads == 1` (off when multi-threaded to avoid a second TT snapshot and core contention); Gomocup, diff, case probe, and the library keep it off by default.
Historical performance experiments and stop-loss conclusions are recorded in `docs/perf-log.md`; the README does not maintain a list of failed approaches.
The Android architecture, mobile interface, JNI boundary, phased validation gates, and release details are documented in `docs/android-app-design.md`.

## Default Parameters

The main defaults live in `src/config.rs`.

| Parameter | Default |
|---|---:|
| Fixed search depth | `8` |
| Fixed root width | `40` |
| Time-control max depth | `25` |
| Time-control max width | `40` |
| `root_vcf_depth` | `8` (classical); automatically deepens to `16` with 64K Zobrist VCF TT in NNUE mode unless explicitly overridden |
| `opponent_vcf_depth` | `7` |
| `vct_verify_opponent_vcf_depth` | `4` |
| `vcf_multi_reply` | `true` |
| `root_vct_depth` | `6` |
| `vct_use_dfpn` | `true` (`df-pn` VCT up to `(root_vct_depth * 3).clamp(2, 22)` plies; set `0` for legacy IDDFS VCT) |
| `vct_strict_and_memo_key` | `true` (legacy IDDFS VCT only) |
| `use_nnue` | `false` in library/Gomocup unless `--nnue-dir` / `INFO nnue_dir` / `RUST_GOMOKU_NNUE_DIR` is supplied; enabled by default in GUI/Android Pro mode when weights are available |
| TT bucket bits | `20` (32 MiB) unified for GUI & Android; library/Gomocup defaults to `20` (1–3 threads or depth `< 5`) / `21` (4+ threads at depth `>= 5`), or auto-sized via `INFO max_memory` |
| `threads` | `1` (lib/Gomocup); `4` on 4+ cores or `min(2, cores)` on `< 4` cores (GUI & Android; override with `--threads N` / `set_threads`) |
| `compute_vcf` / `compute_vct` | VCF enabled; VCT enabled in classical & 1T NNUE, skipped by default on multi-threaded NNUE (`threads > 1` without overlap; override via `INFO compute_vct`) |
| `overlap_vct_alphabeta` | off; GUI enables it only when `threads == 1` |
| `nonroot_vcf` | off |
| `static_board` | on |
| `dynamic_board_margin` | `4` |

The desktop GUI and Android app support two modes (both using unified `tt_bits = 20` / 32 MiB TT):

1. **Pro mode (default)** — NNUE (`Mix9svq`) timed search with four per-move time limit presets (`500 ms`, `1000 ms`, `2500 ms`, `5000 ms`; default `1000 ms`), running iterative deepening up to `d25 / w40` with VCF enabled (and 1T root VCT; multi-threaded NNUE skips serial root df-pn VCT by default). If NNUE weights are unavailable on desktop, Pro mode falls back cleanly to classical evaluation.

2. **Casual mode** — four shared classical fixed-depth difficulty presets:

| Difficulty | Search | VCF / VCT |
|---|---:|---:|
| Beginner (`beginner`) | `d2 / w10` | off |
| Normal (`normal`, default casual) | `d4 / w20` | off |
| Senior (`senior`) | `d6 / w30` | VCF on, VCT off |
| Master (`master`) | `d8 / w40` | VCF on, VCT on (`root_vct_depth=6`) |

For strict diffs against the Python reference or reproducing reference experiments, `depth=6,width=20,root_vct_depth=4` is typically passed explicitly.

Renju mode implements alternating play plus black forbidden moves. Renju opening protocols such as RIF, Yamaguchi, Soosorv, and Swap2 are not implemented.

## Coordinate Convention

External coordinates are uniformly `(x, y) = (column, row)`, matching Gomocup, the GUI, and case JSON files. Internally, the board matrix follows Rust array conventions as `grid[row][col]`, and `Move` is encoded as `row * BOARD_SIZE + col`. Prefer `xy_to_move` / `move_to_xy` in code; in matrix contexts the aliases `rc_to_move` / `move_to_rc` can be used to avoid confusion.

## Reference Path

The full Python reference is expected to live in an external local directory by default:

```bash
~/python_ws/pygomoku
```

You can also point to it explicitly:

```bash
export PYGOMOKU_REF_ROOT=~/python_ws/pygomoku
```

Scripts resolve the reference in this order: `--ref-root`, `PYGOMOKU_REF_ROOT`, then `~/python_ws/pygomoku`.

## Common Commands

Build and test:

```bash
cargo build --release
cargo test --quiet
python3 scripts/run_diff.py --jobs 10
```

Android debug/release gates:

```bash
cd android
./gradlew test lint assembleDebug
./gradlew test lint assembleRelease bundleRelease
```

Release signing reads `~/.android/rust-gomoku-release.properties` by default;
see `docs/android-app-design.md` for the local signing file format and override
environment variable.

Gomocup smoke:

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
printf 'START 15\nINFO rule 4\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

Launch the GUI:

```bash
cargo run --release --bin gomoku_gui
```

The GUI opens `http://127.0.0.1:18080` in the default browser automatically; pass `--no-open-browser` to disable that behavior, `--nnue-dir PATH` to specify a weight directory (auto-detects `<exe_dir>/nnue`, `./nnue`, `RUST_GOMOKU_NNUE_DIR`, or local dev weights by default), or `--no-nnue` to force classical evaluation. It supports a vs-engine mode (playing black/white) and a two-player mode (two humans alternating on the same board with no engine), freestyle or Renju rules, Pro (`500 ms`–`5000 ms` NNUE timed, default `1000 ms`) and Casual (four classical fixed-depth levels) modes, a multi-threaded search toggle, undo, restart, async thinking, move number display, a result dialog, and a status panel; shortcuts: `U` to undo, `R` to restart. The engine-only controls (side, mode, casual difficulty / pro time limit, multi-threading) are hidden in two-player mode. On an empty board, rule, game mode, and Casual/Pro mode apply immediately; once a game is underway (`move_count > 0`), changing them shows a pending notice and takes effect when starting the next game. During a Renju black turn, forbidden intersections are marked with red crosses and forbidden input is rejected without placing a stone (this holds in both modes). Casual difficulty, Pro time limit, and the multi-thread toggle can be adjusted mid-game while the engine is idle and affect the next engine move without resetting the board.

Gomocup engine:

```bash
cargo run --release --bin gomocup_engine
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --threads 4 --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --tt-bits 22
```

Common `INFO` commands:

- `INFO rule 0|4` or `INFO rule freestyle|renju` (only while the board is empty)
- `INFO timeout_turn N`
- `INFO time_left N`
- `INFO max_memory N` (bytes; auto-sizes TT capacity when `tt_bits` is not explicitly pinned)
- `INFO max_node N`
- `INFO thread_num N` (ABDADA multi-threaded search; `1` stays deterministic)
- `INFO compute_vcf 0|1`
- `INFO root_vcf_depth N`
- `INFO opponent_vcf_depth N`
- `INFO vct_verify_opponent_vcf_depth N`
- `INFO vcf_multi_reply 0|1`
- `INFO compute_vct 0|1`
- `INFO root_vct_depth N`
- `INFO vct_use_dfpn 0|1`
- `INFO use_nnue 0|1`
- `INFO nnue_dir PATH` (directory containing Rapfi CC0 `mix9svq*.bin.lz4` weights; CLI `--nnue-dir PATH` or env `RUST_GOMOKU_NNUE_DIR`)
- `INFO vct_strict_and_memo_key 0|1`
- `INFO nonroot_vcf 0|1`
- `INFO overlap_vct_alphabeta 0|1`
- `INFO tt_bits N`
- `INFO static 0|1`
- `INFO dynamic_board_margin N`
- `INFO root_profile 0|1`

VCF, VCT and alpha-beta share a deadline and cumulative node budget across a move.
When both time controls are given the smaller limit applies; under a time limit,
root VCT is capped at 30% of the move budget so alpha-beta still runs when VCT
does not find a forced win. Cancellation is
cooperative. Finishing a board operation, selecting a legal fallback and
protocol setup can add wall time. Under a budget, the reported node count
includes tactical visits. Without limits it retains the alpha-beta count.

Use `scripts/run_diff.py --profile all --check-baselines` to verify pinned
Rust and Python values for fractional TT depth and whole-move budget changes.
The default diff still reports every mismatch. For same-position Renju
benchmarks pass `--rule renju` to `scripts/bench_match_cases.py`.

## Diffs and Matches

Single-case diff:

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11_d6_w5.json --root-profile
```

Rust default parameters vs. the Python reference, 9 openings, both sides, 18 games:

```bash
cargo build --release --bin gomocup_engine
python3 scripts/run_engine_match.py \
  --opening-set 9 \
  --jobs 18 \
  --output /tmp/rust_vs_reference_9_openings.json
```

Generic engine match:

Set `BASELINE_DIR` to the directory containing the baseline release builds of `gomocup_engine` and `case_probe`.

```bash
cargo build --release --bin gomocup_engine --bin case_probe
BASELINE_DIR=/path/to/baseline/bin
python3 scripts/run_gomocup_match.py \
  --case-file cases/match/smoke_quick.jsonl \
  --engine-a-side both \
  --engine-a-command 'target/release/gomocup_engine --depth 6 --width 20' \
  --engine-b-command "\"$BASELINE_DIR/gomocup_engine\" --depth 6 --width 20" \
  --output /tmp/engine_version_match.json
```

Same-position single-move benchmark:

```bash
python3 scripts/bench_match_cases.py \
  --case-file cases/match/standard.jsonl \
  --baseline-binary "$BASELINE_DIR/case_probe" \
  --candidate-binary target/release/case_probe \
  --jobs 1 \
  --output /tmp/engine_version_bench.json
```

`run_gomocup_match.py` is used for real matches and strength evaluation; `bench_match_cases.py` is used for single-move searches on the same batch of prefix positions. To judge whether an optimization is genuinely faster, look at the same-position benchmark first, then at match win rate and long-tail latency.

Without distinct engines or options, the match and benchmark scripts flag identical configurations as self-play or repeatability checks.

For Renju matches, pass `--rule renju`. The match runner then configures both
engines with Gomocup rule `4` and uses the rule-aware referee to reject illegal
black moves:

```bash
cargo build --release --bin gomocup_engine --bin renju_referee
python3 scripts/run_gomocup_match.py \
  --rule renju \
  --case-file cases/renju/strength_100_prefixes.jsonl \
  --engine-a-side both \
  --engine-a-command 'target/release/gomocup_engine --depth 8 --width 40' \
  --engine-b-command "\"$BASELINE_DIR/gomocup_engine\" --depth 8 --width 40"
```

The full Renju design, validation evidence, SlowRenju comparison contract, and
remaining work are maintained in `docs/renju-forbidden-design.md`.

To observe real cross-move TT behavior, pass `--reuse-engine-state` to `run_gomocup_match.py`. By default the match script issues `RESTART` per move, which is fairer for reproducibility; `--reuse-engine-state` retains engine/searcher state and aggregates each move's `MESSAGE tt_generation current=... old=...` into the `tt_generation` field of the output JSON.

## Directory Layout

```text
src/                 Rust engine, NNUE, Gomocup, GUI, diff/case probe
android/             Android ARM64 app (Kotlin/WebView) and JNI rust_bridge crate
cases/diff/          root diff cases
cases/match/         match and benchmark prefix positions
cases/renju/         forbidden, tactical, diagnostic, and strength fixtures
data/static/         static matrices extracted from the reference
docs/                design docs, perf log, and screenshots
scripts/             diff, match, benchmark, and case extraction scripts
tests/               Rust automated tests
```

## Current Focus

1. Preserve verified rule correctness and classical regression coverage across Freestyle and Renju (forbidden-move oracle/stress suite and `13/13` Python reference baseline diffs).
2. Build on the locked `Mix9svq` NNUE baseline (`35.5%` 1T / `34.0%` 4T vs same-weight Rapfi at 32 MiB TT, Renju `1s/move`) to further improve search efficiency, tactical defense movegen, aspiration windows, and multi-thread scaling.
3. Add ARM64 NEON SIMD kernels for `Mix9svq` to boost mobile search depth on Android devices.
4. Require every performance or strength change to report correctness, latency, nodes, and fair match results together in `docs/perf-log.md`.

## License

This project is licensed under the GNU General Public License v3.0 or later. See [LICENSE](LICENSE).
