# rust_gomoku

`rust_gomoku` is a Rust rewrite of the classic mainline of the Python reference project `pygomoku`, with optional Renju forbidden-move support. Current work preserves both rule modes while reducing search latency and improving playing strength through measured experiments.

A Chinese version of this document is available at [README-cn.md](README-cn.md).

## Current Status

Completed:

- 15x15 freestyle and Renju state machine, zobrist, config, pattern, eval cache, movegen, ordering, TT, and alpha-beta root search.
- Renju black forbidden-move handling for overline, double-four, and recursive true double-three, with exact-five priority; freestyle remains the default.
- Rule-aware root VCF and root-only VCT trigger/verify/trace.
- Gomocup stdin/stdout engine entry point and a local Web GUI.
- Rust/reference diff harness, Rust/reference matches, base/fast matches, and same-position benchmark scaffolding.
- Renju oracle, dense-stress, exhaustive-line, candidate-diagnostic, performance, and rule-aware match-referee tooling.
- Non-root candidate ordering cost optimization: keeps the original ordering key while reducing `getmi` and candidate copy overhead.
- Fast profile enables third-generation history/killer ordering by default: only reorders quiet moves within the same static-ordering group; base is unaffected.
- External reference engines such as SlowRenju and Rapfi are used from local
  checkouts when needed; bundled lightweight opponents are no longer committed.

Optional diagnostics / experiments:

- `overlap_vct_alphabeta`: experimental switch to overlap VCT with alpha-beta after a VCF miss.
- `root_profile`: per-root-candidate timing diagnostics.
- TT generation observation: Gomocup trace can report the new/old ratio of cross-move TT best-move hints. It does not affect TT cutoff, replacement, or ordering policy.

To shorten the perceived wait during manual GUI play, the GUI entry enables `overlap_vct_alphabeta` by default; Gomocup, diff, case probe, and the library still keep it off.
Historical performance experiments and stop-loss conclusions are recorded in `docs/perf-log.md`; the README does not maintain a list of failed approaches.
The Android architecture, mobile interface, JNI boundary, phased validation
gates, and current implementation status are documented in
`docs/android-app-design.md`. The Android app builds ARM64 debug and signed
release artifacts and uses the shared Rust controller through the tested JNI
bridge.

## Default Parameters

The main defaults live in `src/config.rs`.

| Parameter | Default |
|---|---:|
| Fixed search depth | `8` |
| Fixed root width | `40` |
| Time-control max depth | `25` |
| Time-control max width | `40` |
| `root_vcf_depth` | `8` |
| `opponent_vcf_depth` | `7` |
| `vct_verify_opponent_vcf_depth` | `4` |
| `vcf_multi_reply` | `true` |
| `root_vct_depth` | `4` |
| `vct_strict_and_memo_key` | `true` |
| TT bucket bits | `20` |
| `compute_vcf` / `compute_vct` | enabled |
| `overlap_vct_alphabeta` | off; GUI entry turns it on |
| `fast_history_ordering` | off in base, on in fast |
| `nonroot_vcf` | off |
| `static_board` | on |
| `dynamic_board_margin` | `4` |

The desktop GUI and Android app expose five shared difficulty presets:

| Difficulty | Search | VCF / VCT |
|---|---:|---:|
| Beginner | `d1 / w10` | off |
| Junior | `d2 / w10` | off |
| Intermediate | `d4 / w20` | off |
| Senior | `d6 / w30` | on, `root_vct_depth=4` |
| Master | `d8 / w40` | on, `root_vct_depth=4` |

Intermediate is the default for the desktop GUI and Android app. Difficulty is
independent from the Base/Fast profile: difficulty controls depth, width, and
tactical search, while Base/Fast controls ordering behavior.

For strict diffs against the Python reference or reproducing reference experiments, `depth=6,width=20,root_vct_depth=4` is typically passed explicitly.

The default profile is `base`, used to preserve classic semantics and reference diffs. `--profile fast` currently enables `fast_history_ordering` by default; pass `--no-fast-history-ordering` to disable it for comparison.

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

The GUI opens `http://127.0.0.1:18080` in the default browser automatically; pass `--no-open-browser` to disable that behavior. It supports a vs-engine mode (playing black/white) and a two-player mode (two humans alternating on the same board with no engine), freestyle or Renju rules, five difficulty levels, undo, restart, Base/Fast mode switching, async thinking, move number display, a result dialog, and a status panel; shortcuts: `U` to undo, `R` to restart. The engine-only controls (side, Base/Fast, difficulty) are hidden in two-player mode. Rule selection applies only when starting a new game. During a Renju black turn, forbidden intersections are marked with red crosses and forbidden input is rejected without placing a stone (this holds in both modes). Difficulty and Base/Fast can only change while the engine is idle and affect the next engine think without resetting the board.

Gomocup engine:

```bash
cargo run --release --bin gomocup_engine
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --tt-bits 22
target/release/gomocup_engine --profile fast
```

Common `INFO` commands:

- `INFO rule 0|4` or `INFO rule freestyle|renju` (only while the board is empty)
- `INFO timeout_turn N`
- `INFO time_left N`
- `INFO max_node N`
- `INFO profile base|fast`
- `INFO compute_vcf 0|1`
- `INFO root_vcf_depth N`
- `INFO opponent_vcf_depth N`
- `INFO vct_verify_opponent_vcf_depth N`
- `INFO vcf_multi_reply 0|1`
- `INFO compute_vct 0|1`
- `INFO root_vct_depth N`
- `INFO vct_strict_and_memo_key 0|1`
- `INFO fast_history_ordering 0|1`
- `INFO nonroot_vcf 0|1`
- `INFO overlap_vct_alphabeta 0|1`
- `INFO tt_bits N`
- `INFO static 0|1`
- `INFO dynamic_board_margin N`
- `INFO root_profile 0|1`

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

```bash
python3 scripts/run_gomocup_match.py \
  --case-file cases/match/smoke_quick.jsonl \
  --jobs 12 \
  --max-moves 80 \
  --move-timeout-sec 90 \
  --game-timeout-sec 600 \
  --output /tmp/base_fast_smoke_quick.json
```

Same-position single-move benchmark:

```bash
cargo build --release --bin case_probe
python3 scripts/bench_match_cases.py \
  --case-file cases/match/standard.jsonl \
  --jobs 16 \
  --output /tmp/base_fast_standard_bench.json
```

`run_gomocup_match.py` is used for real matches and strength evaluation; `bench_match_cases.py` is used for single-move searches on the same batch of prefix positions. To judge whether an optimization is genuinely faster, look at the same-position benchmark first, then at match win rate and long-tail latency.

For Renju matches, pass `--rule renju`. The match runner then configures both
engines with Gomocup rule `4` and uses the rule-aware referee to reject illegal
black moves:

```bash
cargo build --release --bin gomocup_engine --bin renju_referee
python3 scripts/run_gomocup_match.py \
  --rule renju \
  --case-file cases/renju/strength_100_prefixes.jsonl \
  --engine-a-side both \
  --engine-a-command 'target/release/gomocup_engine --depth 8 --width 40'
```

The full Renju design, validation evidence, SlowRenju comparison contract, and
remaining work are maintained in `docs/renju-forbidden-design.md`.

To observe real cross-move TT behavior, pass `--reuse-engine-state` to `run_gomocup_match.py`. By default the match script issues `RESTART` per move, which is fairer for reproducibility; `--reuse-engine-state` retains engine/searcher state and aggregates each move's `MESSAGE tt_generation current=... old=...` into the `tt_generation` field of the output JSON.

## Directory Layout

```text
src/                 Rust engine, Gomocup, GUI, diff/case probe
cases/diff/          root diff cases
cases/match/         match and benchmark prefix positions
cases/renju/         forbidden, tactical, diagnostic, and strength fixtures
data/static/         static matrices extracted from the reference
scripts/             diff, match, benchmark, and case extraction scripts
tests/               Rust automated tests
```

## Current Focus

1. Preserve the completed 100-game fixed-depth Renju-vs-SlowRenju gate; future expansion should add independent source games rather than nearby prefixes from the same trajectories.
2. Continue reducing the remaining Renju per-node overhead and VCF/VCT long tails from the current post-optimization baseline; keep performance changes behind oracle, root-diff, and strength gates.
3. Audit Rapfi primarily to improve playing strength, focusing on candidate ordering, evaluation, tactical search, TT, pruning/extensions, and time management. Line-pattern and forbidden-table ideas are also relevant when they enable deeper search.
4. Preserve the completed 100-game freestyle gates: Rust-vs-SlowRenju, current-vs-pre-Renju sequence equivalence, classic reference/Rust diffs, and fast-vs-base strength checks.
5. All performance experiments must report correctness, latency, nodes, and any changes to move/score/trace together.

## License

This project is licensed under the GNU General Public License v3.0 or later. See [LICENSE](LICENSE).
