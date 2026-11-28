# Testing and engine matches

## Regression checks

Run the Rust tests and the protocol harness. Protocol examples are in the [Gomocup guide](gomocup.md), with additional Renju fixtures in the [rules guide](renju-forbidden-design.md).

```bash
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'
```

For search changes, compare representative fixed positions for both rules. Record the best move, score, completed depth, nodes, and PV or tactical trace. Keep the engine path, weights, TT, prior position state, and search limits constant. Use one thread, a fixed depth, and no clock messages when isolating deterministic search behavior. The native engine reports depth, score, nodes, time, and PV with `INFO root_profile 1`.

## Match procedure

Use [`run_gomocup_match.py`](../scripts/run_gomocup_match.py) for full games. Archive both executables, their settings, weight identities, and the opening file or its hash. Use the same opening positions in both colors and keep the number of concurrent games fixed between runs. `--reuse-engine-state` preserves engine state within a game. The runner normally sends `RESTART` per move when this option is omitted.

This is the Renju 1T setup for the [2026-11-28 benchmark](perf-log.md#2026-11-28--route-a-correctness-fixes-and-600-games). Set the four paths to the archived candidate build, its weight directory, the existing Rapfi directory, and the case file. The Rapfi working directory must contain its binary, weights, and configuration.

```bash
cargo build --release --locked --bin renju_referee
ENGINE_A=/path/to/gomocup_engine
NNUE_DIR=/path/to/mix9svq
RAPFI_DIR=/path/to/rapfi_mix9svq
CASE_FILE=/path/to/renju_100pairs.jsonl
threads=1
info=()
for option in timeout_turn=1000 timeout_match=1000000 time_left=1000000 \
              max_memory=33554432 thread_num="$threads" rule=4; do
  info+=(--engine-a-info "$option" --engine-b-info "$option")
done

python3 scripts/run_gomocup_match.py \
  --case-file "$CASE_FILE" --rule renju \
  --referee-command target/release/renju_referee \
  --engine-a-side both --reuse-engine-state --jobs 8 --max-moves 225 \
  --move-timeout-sec 120 --game-timeout-sec 900 \
  --engine-a-name candidate --engine-a-command "\"$ENGINE_A\" --nnue-dir \"$NNUE_DIR\"" \
  --engine-b-name rapfi --engine-b-command ./pbrain-rapfi --engine-b-cwd "$RAPFI_DIR" \
  "${info[@]}" --output /tmp/renju_1t.json
```

The 120- and 900-second values are harness watchdogs. The engine receives a 1-second move budget through `INFO timeout_turn`. For Freestyle use the Freestyle cases, `--rule freestyle`, and `rule=0` for both engines. Use [the benchmark matrix](perf-log.md#2026-11-28--route-a-correctness-fixes-and-600-games) to reproduce specific job counts. For a quick protocol match substitute the tracked [smoke cases](../cases/match/smoke_quick.jsonl) and keep the same options for both engines.

Report W–L–D, score counting a draw as half a point, paired confidence interval, Elo estimate, color split, errors and timeouts, and both engines' mean/median/P95/max move times. For comparisons between runs, use the same opening pairs and estimate the interval of the paired score change. When repeating the same openings across runs or thread counts, group the observations by opening for combined uncertainty. Rule forfeits, protocol failures, and watchdog timeouts should be reported separately.

The whole-game watchdog also applies while waiting for the Renju referee. A failed referee response records an incomplete game with `termination: "referee_error"`, with no win credited to either side.

For a single-thread timed repeat, record completed depth and nodes where possible. Equal clock settings allow different amounts of work to finish before the deadline. The [November 28 repeat](perf-log.md#renju-1t-repeat) gives an example with unchanged binaries.

## Same-position benchmarks

Use [`bench_match_cases.py`](../scripts/bench_match_cases.py) for one search per prefix. It exercises `case_probe` and is useful for classical positions. For native NNUE, replay the same positions through Gomocup with NNUE enabled and explicit limits.

```bash
cargo build --release --locked --bin case_probe
BASELINE_DIR=/path/to/baseline/bin
python3 scripts/bench_match_cases.py \
  --case-file cases/match/standard.jsonl \
  --baseline-binary "$BASELINE_DIR/case_probe" \
  --candidate-binary target/release/case_probe \
  --jobs 1 --output /tmp/same_position.json
```

For performance changes compare elapsed times on the same work where feasible. For search changes use matches as well as node counts and tactical checks.

## Classical differential regression

The older Python `pygomoku` engine remains a regression source for the classical path. Existing pinned expectations cover intentional differences between the engines.

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json
python3 scripts/run_diff.py --profile all --check-baselines --jobs 4
```

The full Python checkout is external. Configure it with `--ref-root` or `PYGOMOKU_REF_ROOT`. The default lookup is `~/python_ws/pygomoku`. The pinned suite covers fixed search, tactical paths, and budget cases. Use the native NNUE and match checks above when validating the Pro engine.
