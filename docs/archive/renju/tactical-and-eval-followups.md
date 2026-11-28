# Forbidden detection performance and static evaluation

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

### Phase 7: SlowRenju-Style Forbidden Performance

Goal: reduce Renju movegen overhead without changing legality semantics. The
first implementation should follow SlowRenju's `shapeM`/`valueM` approach
rather than inventing a separate geometric prefilter.

Current evidence in this repository:

- `EvalCaches` already stores per-side, per-point, per-direction `shape_cache`
  plus `value_cache` and `attack_cache`.
- `src/eval/local.rs::compute_direction_shape` currently computes shapes with
  `freestyle=true`, so the cache is still classic/freestyle shaped even when
  `EngineConfig.rule_set == RuleSet::Renju`.
- `generate_candidates` currently filters Renju black candidates through
  `Board::is_legal_move_for_rule`, which calls the full forbidden detector for
  every covered black candidate.
- `tests/renju_perf.rs` provides ignored release probes for:
  - one `is_legal_move_for_rule` call;
  - one interior-node `generate_candidates` call.

Implementation order:

1. Add rule identity to eval caches.
   - Store the `RuleSet` used to build `EvalCaches`.
   - Recompute caches when the requested rule differs from the cached rule.
   - Preserve the current freestyle cache contents and behavior when
     `RuleSet::Freestyle` is used.
2. Make shape computation rule-aware.
   - Thread `RuleSet` into `compute_direction_shape`,
     `recompute_point_caches`, `recompute_all`, `value_wide_compute`, and the
     helper paths that update one direction.
   - For `RuleSet::Renju` and black, use the non-freestyle/forbidden-aware row
     of the existing SlowRenju-style shape table.
   - For `RuleSet::Freestyle`, keep the old `freestyle=true` path.
   - For white in Renju, keep no-forbidden semantics; white has no forbidden
     moves.
3. Add a SlowRenju-style candidate prefilter for Renju black movegen.
   - Inspect the four cached black directional shapes for the candidate.
   - If any direction is `L5`, the candidate is an exact-five candidate and is
     not forbidden by exact-five priority.
   - If the cached shapes show possible `L6`, two-or-more four threats, or
     two-or-more open-three threats, call the full detector to confirm.
   - Otherwise treat the move as legal without calling the recursive detector.
   - The prefilter must be conservative: false positives are acceptable because
     they fall back to the full detector; false negatives are not acceptable.
4. Keep strict legality at external boundaries.
   - `Board::is_legal_move_for_rule`, `play_for_rule`, Gomocup input, and GUI
     input should continue to use the full detector.
   - The cache prefilter is only for movegen/search hot paths where candidates
     are already filtered again by tests and tactical validation.
5. Re-measure and record results.
   - Run the ignored release perf test before and after the prefilter.
   - Record `ns/call` and `ns/node` in `docs/perf-log.md` if the change is kept.

Tests and validation:

```bash
cargo fmt --check
cargo test --quiet
cargo build --bin gomoku_gui --bin gomocup_engine --quiet
python3 scripts/renju_oracle_compare.py --quiet
python3 scripts/renju_dense_stress.py --skeleton all --count 400 --seed <seed> \
    --output /tmp/renju_dense_prefilter.jsonl
python3 scripts/renju_oracle_compare.py \
    --case-file /tmp/renju_dense_prefilter.jsonl --quiet
cargo test --release --test renju_perf -- --ignored --nocapture
git diff --check
```

Exit criteria:

- Freestyle movegen and eval behavior stay unchanged.
- Renju movegen still never emits forbidden black moves in fixture and dense
  oracle validation.
- The perf probes show a clear reduction in Renju movegen `ns/node`.
- Any remaining detector calls in movegen are limited to SlowRenju-style
  suspicious points.

### Phase 8: Renju Static Evaluation

Goal: make quiet-position evaluation understand Renju forbidden points while
preserving all freestyle evaluation behavior.

Status: implemented in `src/eval/local.rs` by applying SlowRenju-style
black-point suppression when building rule-aware eval caches.

The bundled bucket weights already follow the SlowRenju/pygomoku `para[]`
layout (`last_eval`, `next_eval`, `attack_value`, `defend_value`). Phase 8
should not copy another unrelated weight table. It should reuse the existing
weights and port SlowRenju's rule-gated evaluation semantics.

Implementation order:

1. Keep eval tables shared, but make eval cache contents rule-aware.
   - Freestyle keeps current buckets and attack levels.
   - Renju black uses the rule-aware shape cache from Phase 7.
2. Apply SlowRenju-style black forbidden-point suppression.
   - If a black empty point is an exact-five candidate (`L5`), keep its winning
     value.
   - If a black empty point is confirmed forbidden by overline, double-four, or
     true double-three, set its `value_cache` and `attack_cache` contribution to
     the safe/zero value used by the existing bucket system.
   - Use the full detector only for suspicious double-three cases that cannot
     be proven from cached shapes alone.
3. Keep white evaluation non-forbidden.
   - White has no forbidden moves.
   - White threats that become stronger because black's only defence is
     forbidden should be handled as a separate sub-step after black forbidden
     point suppression is stable.
4. Add targeted eval tests.
   - Freestyle cached eval and scan eval remain unchanged on existing tests.
   - In Renju mode, a black forbidden four-four or overline point does not keep
     high attack/value.
   - A black exact-five point remains high value.
   - White's equivalent point is not suppressed.

Validation:

```bash
cargo test --quiet
cargo test --test root_alignment --quiet
python3 scripts/renju_oracle_compare.py --quiet
cargo test --release --test renju_perf -- --ignored --nocapture
git diff --check
```

Exit criteria:

- Freestyle eval tests and fixed search alignment stay unchanged.
- Renju eval no longer rewards black forbidden continuations as strong quiet
  moves.
- Any intentional Renju move/score changes are documented with fixture names.

Implementation notes:

- Freestyle keeps the existing bucket and attack computation.
- Renju black first uses the rule-aware shape cache from Phase 7.
- If black has an exact-five candidate, the point keeps its winning value.
- If black has cached double-four or overline evidence and no exact five, the
  point's black `value_cache` and `attack_cache` are set to zero.
- If black has cached two-or-more open-three lines and no exact five, the full
  forbidden detector confirms whether it is a true double-three before
  suppression.
- White keeps the freestyle/no-forbidden evaluation path under Renju.

Validation run:

```bash
cargo test --quiet
cargo build --bin gomoku_gui --bin gomocup_engine --quiet
python3 scripts/renju_oracle_compare.py --quiet
python3 scripts/renju_dense_stress.py --skeleton all --count 400 --seed 33 \
    --output /tmp/renju_dense_phase8.jsonl
python3 scripts/renju_oracle_compare.py \
    --case-file /tmp/renju_dense_phase8.jsonl --quiet
python3 scripts/run_diff.py --jobs 4
cargo test --release --test renju_perf -- --ignored --nocapture
```

Result summary:

- Freestyle root diff: 11/11 default cases passed.
- Dense oracle stress: 2000/2000 local/Rapfi/renju_forbid comparisons had no
  unexplained mismatches.
- Perf after Phase 8: Renju movegen measured about 1503 ns/node on the existing
  forcing-node probe; direct full detector measured about 726 ns/call.

### Phase 9: Remaining Rule Surface And Regression Gates

Goal: finish integration after performance and static eval are stable.

Remaining work:

- Add more white-positive tactical fixtures where black's only defence is
  forbidden.
- Add a broader eval-suppression consistency gate: when rule-aware Renju eval
  suppresses a black point that freestyle eval considered valuable, the full
  forbidden detector should also classify that point as forbidden. Phase 8
  covers the hand fixtures; Phase 9 should extend this to dense/random cases so
  cached four/overline labels cannot silently over-suppress detector-legal
  strong points.
- Add GUI and Gomocup smoke coverage for new-game rule selection and illegal
  forbidden input.
- Confirm one game cannot switch between freestyle and Renju after the first
  move.
- Re-run root/search alignment tests with Renju-specific fixtures after Phase 8
  changes eval.
- Keep opening rules such as RIF/Yamaguchi/Soosorv/Swap2 out of scope unless
  explicitly requested later.

Recommended release gate for the Renju feature:

```bash
cargo fmt --check
cargo test --quiet
cargo build --bin gomoku_gui --bin gomocup_engine --quiet
python3 scripts/renju_oracle_compare.py --quiet
python3 scripts/renju_dense_stress.py --skeleton all --count 400 --seed <seed> \
    --output /tmp/renju_dense_phase9.jsonl
python3 scripts/renju_eval_suppression_check.py \
    --case-file /tmp/renju_dense_phase9.jsonl
python3 scripts/renju_oracle_compare.py \
    --case-file /tmp/renju_dense_phase9.jsonl --quiet --require-oracles
python3 scripts/run_diff.py --jobs 4
cargo test --release --test renju_perf -- --ignored --nocapture
git diff --check
```

Phase 9 status: complete.

- Added `scripts/renju_eval_suppression_check.py`, which wraps an ignored Rust
  test to validate arbitrary JSONL fixture files. The gate asserts that if
  Renju eval suppresses a black point that freestyle eval considered valuable,
  the full forbidden detector also classifies that point as forbidden.
- The suppression gate passed the 72 hand fixtures and a 2000-case dense batch
  generated with seed 34.
- Gomocup `INFO rule` follows the standard Piskvork flow: it may arrive after
  `START`, `RECTSTART`, or `RESTART` while the board is still empty. Rule
  changes are ignored once the first move has been played, so switching modes
  still requires starting/restarting to an empty board first.
- Added a stateless SlowRenju `trd3`-style eval invalidation step for Renju:
  after each incremental `value_wide_compute_for_rule`, scan the full board with
  cheap shape-cache early-outs and rerun the full forbidden detector only for
  black empty points that are apparent double-threes. This keeps recursive
  double-three legality flips from leaving stale suppressed eval cache entries.
- Promoted the seeded dense incremental-vs-full recompute stress test to a
  normal unit regression. It covers the original non-local stale-suppression
  failure where a later move made a previously forbidden black point legal.
- The browser GUI applies freestyle/Renju selection only when starting a new
  game. During a Renju black turn it exposes the detector's current forbidden
  points in the state response and draws red cross markers on those
  intersections. Clicking a marked point is rejected without changing the
  board, with the specific forbidden reason retained in the error message.
  Freestyle and white turns expose no forbidden markers.
- GUI smoke covered new-game rule selection, rule persistence after a move,
  forbidden-point display, and forbidden input rejection. The rendered marker
  was also checked manually in the browser.
- Final Phase 9 release gate used dense seed 36 with 2000 cases:
  `double_four=614`, `double_three=167`, `none=406`, `overline=813`.
  Eval suppression matched the full detector for all cases. The local
  detector, Rapfi, and `renju_forbid` had zero unexplained mismatches; three
  overline/double-three coexistence cases were accepted by the documented
  reporting-convention rule.
- The 72 hand fixtures passed all three detectors with zero unexplained
  mismatches, and the freestyle root diff passed all 11 default cases.
- Final release performance measured about 859 ns per full Renju legality
  check and 1604 ns per Renju movegen node on the existing release probes.
  These figures are regression baselines, not strength measurements.
