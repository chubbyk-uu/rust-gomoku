# rust_gomoku

`rust_gomoku` is a Rust rewrite of the classic mainline of the Python reference project `pygomoku`. The current focus is to preserve classic behavior while continuing to reduce average and long-tail search latency.

A Chinese version of this document is available at [README-cn.md](README-cn.md).

## Current Status

Completed:

- 15x15 free-rule Gomoku state machine, zobrist, config, pattern, eval cache, movegen, ordering, TT, and alpha-beta root search.
- Root VCF, root-only VCT trigger/verify/trace.
- Gomocup stdin/stdout engine entry point and a local Web GUI.
- Rust/reference diff harness, Rust/reference matches, base/fast matches, and same-position benchmark scaffolding.
- Non-root candidate ordering cost optimization: keeps the original ordering key while reducing `getmi` and candidate copy overhead.
- Fast profile enables third-generation history/killer ordering by default: only reorders quiet moves within the same static-ordering group; base is unaffected.
- `opponent/zhou` is kept in the repo as a lightweight opponent; the full Python reference is not committed.

Optional diagnostics / experiments:

- `overlap_vct_alphabeta`: experimental switch to overlap VCT with alpha-beta after a VCF miss.
- `root_profile`: per-root-candidate timing diagnostics.
- TT generation observation: Gomocup trace can report the new/old ratio of cross-move TT best-move hints. It does not affect TT cutoff, replacement, or ordering policy.

To shorten the perceived wait during manual GUI play, the GUI entry enables `overlap_vct_alphabeta` by default; Gomocup, diff, case probe, and the library still keep it off.
Historical performance experiments and stop-loss conclusions are recorded in `docs/perf-log.md`; the README does not maintain a list of failed approaches.

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
| `root_vct_depth` | `6` |
| `vct_strict_and_memo_key` | `true` |
| TT bucket bits | `20` |
| `compute_vcf` / `compute_vct` | enabled |
| `overlap_vct_alphabeta` | off; GUI entry turns it on |
| `fast_history_ordering` | off in base, on in fast |
| `nonroot_vcf` | off |
| `static_board` | on |
| `dynamic_board_margin` | `4` |

For strict diffs against the Python reference or reproducing reference experiments, `depth=6,width=20,root_vct_depth=4` is typically passed explicitly.

The default profile is `base`, used to preserve classic semantics and reference diffs. `--profile fast` currently enables `fast_history_ordering` by default; pass `--no-fast-history-ordering` to disable it for comparison.

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

Gomocup smoke:

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

Launch the GUI:

```bash
cargo run --release --bin gomoku_gui
```

Then open `http://127.0.0.1:18080`. The GUI supports playing black/white, freestyle or Renju rules, undo, restart, Base/Fast mode switching, async thinking, move number display, and a status panel; shortcuts: `U` to undo, `R` to restart. Rule selection applies only when starting a new game. During a Renju black turn, forbidden intersections are marked with red crosses and forbidden input is rejected without placing a stone. The Base/Fast switch is only allowed when the engine is idle, does not reset the current game, and only affects the next engine think.

Gomocup engine:

```bash
cargo run --release --bin gomocup_engine
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --tt-bits 22
target/release/gomocup_engine --profile fast
```

Common `INFO` commands:

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

To observe real cross-move TT behavior, pass `--reuse-engine-state` to `run_gomocup_match.py`. By default the match script issues `RESTART` per move, which is fairer for reproducibility; `--reuse-engine-state` retains engine/searcher state and aggregates each move's `MESSAGE tt_generation current=... old=...` into the `tt_generation` field of the output JSON.

## Directory Layout

```text
src/                 Rust engine, Gomocup, GUI, diff/case probe
cases/diff/          root diff cases
cases/match/         match and benchmark prefix positions
data/static/         static matrices extracted from the reference
opponent/zhou/       zhou baseline opponent
scripts/             diff, match, benchmark, and case extraction scripts
tests/               Rust automated tests
```

## Current Focus

1. Continue expanding reference/Rust diff coverage.
2. Target real slow moves to optimize VCT miss and alpha-beta long tail, prioritizing approaches that reliably lower p95/max latency.
3. Continue expanding fast vs. base match coverage for the fast profile to confirm that the default-on history/killer ordering does not lose win rate against base at larger sample sizes.
4. All performance experiments must report correctness, latency, nodes, and any changes to move/score/trace together.

## License

This project is licensed under the GNU General Public License v3.0 or later. See [LICENSE](LICENSE).
