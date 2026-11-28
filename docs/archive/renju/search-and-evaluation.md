# Renju search and protocol integration

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

### Phase 4: Board And Movegen Integration

Only after the detector is stable:

- Add rule-aware legal move helpers without changing current `is_legal_move`
  behavior:
  - `is_legal_move_for_rule(move, side, rule)`
  - or equivalent API chosen during implementation.
- Keep `RuleSet::Freestyle` as default.
- Filter black forbidden moves in Renju movegen.
- Ensure fallback move selection also respects Renju legal moves.
- Ensure TT/searcher state is reset or keyed by rule when rule changes.

Tests:

- Freestyle test suite still passes unchanged.
- Renju black movegen excludes forbidden points.
- Renju white movegen does not exclude equivalent shapes.
- Empty board behavior unchanged.
- If all black moves are forbidden, engine has a defined failure/result path.

Validation:

- `cargo test --quiet`
- Targeted movegen tests.
- Oracle comparison for generated move lists on fixture positions.

Exit criteria:

- Freestyle behavior unchanged.
- Renju movegen never emits a black forbidden move in tested fixtures.

Current Phase 4 status:

- Added `RuleSet` to `EngineConfig`, defaulting to `RuleSet::Freestyle`.
- Added rule-aware board helpers while preserving the old freestyle-only
  `is_legal_move` / `play` behavior:
  - `forbidden_kind_for_rule(move, side, rule)`
  - `is_legal_move_for_rule(move, side, rule)`
- `generate_candidates` now filters moves through `is_legal_move_for_rule`, so
  Renju black forbidden moves are not emitted by normal movegen. Freestyle and
  white movegen remain unchanged by the forbidden detector.
- Root fallback move selection and root-allowed filtering now use the rule-aware
  legality helper.
- If fallback cannot find any rule-legal move, root search now returns a
  controlled losing result instead of panicking. Gomocup converts an invalid
  engine result with no legal fallback into `ERROR No legal move.`.
- Root VCF/VCT fast paths and the opponent VCF filter are intentionally disabled
  when `config.rule_set == RuleSet::Renju`. This avoids returning tactical moves
  proven by freestyle-only VCF/VCT logic before Phase 6 adds rule-aware tactical
  search. Freestyle behavior keeps the existing VCF-before-VCT ordering.
- Gomocup supports `INFO rule freestyle|free|0` and `INFO rule renju|4`, but only
  while `board.move_count() == 0`. A rule change after the game has started is
  ignored, so one game cannot switch between freestyle and Renju midstream.
- The web GUI exposes `无禁手` / `有禁手` as a new-game option. Human black
  forbidden moves in Renju mode are rejected with the detected reason instead
  of being played as an immediate loss. This matches the existing interactive
  UI style: illegal user input is blocked at the UI/protocol boundary.
- The GUI validates engine moves with rule-aware legality before playing them;
  if a Renju search path ever returns a forbidden black move, the UI reports an
  engine move failure instead of placing it.

Current Phase 4 validation:

```bash
cargo fmt --check
cargo test --quiet
cargo build --bin gomoku_gui --bin gomocup_engine --quiet
python3 scripts/renju_oracle_compare.py --quiet
python3 scripts/renju_dense_stress.py --skeleton all --count 400 --seed 20 \
    --output /tmp/renju_dense_seed20_phase4.jsonl
python3 scripts/renju_oracle_compare.py \
    --case-file /tmp/renju_dense_seed20_phase4.jsonl --quiet
git diff --check
```

The seed-20 dense batch generated 2000 cases with distribution
`double_four=601`, `double_three=176`, `none=414`, `overline=809`; local
detector, Rapfi, and `renju_forbid` had zero unexplained mismatches. The only
typed differences were two accepted overline/double-three coexistence reporting
differences.

Known Phase 4 performance cost:

- Freestyle movegen and fallback keep the old raw `is_legal_move` path.
- Renju black movegen calls the recursive forbidden detector for candidate
  filtering. Search-tree placement uses `play_assuming_rule_legal`, so it does
  not repeat the forbidden check for candidates already filtered by movegen; it
  only applies rule-aware terminal semantics. Further optimization, if needed,
  should use a rule-aware legality cache or incremental forbidden-point table;
  do not weaken true-open-three recursion to gain speed.

### Phase 5: Terminal Semantics And Protocol Surface

Add rule mode to config/runtime surfaces:

- Library config.
- Gomocup `INFO rule freestyle|renju`.
- Probe outputs.
- GUI option later, after engine correctness is established.

The library config, Gomocup rule switch, and GUI new-game rule option were
pulled into Phase 4 because they were needed to verify end-to-end movegen
filtering. Phase 5 should focus on terminal/win semantics and any remaining
protocol transcript coverage rather than re-adding those surfaces.

Terminal tests:

- Black exact five wins.
- Black overline is forbidden, not a black win.
- White overline wins.
- Black move that is exact five and also creates another scary-looking shape
  still wins by exact-five priority.

Validation:

- `cargo test --quiet protocol_alignment root_alignment`
- Gomocup transcript smoke:
  - set `INFO rule renju`;
  - try forbidden black move;
  - verify behavior is documented.

Exit criteria:

- Rule switching is explicit.
- Freestyle default remains unchanged.

Current Phase 5 status:

- Added `Board::play_for_rule(move, side, rule)` while keeping `Board::play`
  as the freestyle-compatible default.
- Renju terminal semantics now live at the board play boundary:
  - black wins only when the played move creates an exact five;
  - black overline is rejected as an illegal Renju move before placement;
  - white still wins with five-or-more, including overline.
- Search, Gomocup, and GUI placement paths now call `play_for_rule`, so terminal
  semantics and move legality use the same configured rule. Existing replay and
  default `play` behavior remain freestyle to preserve current reference
  alignment tests.
- Search-tree internals use `play_assuming_rule_legal` after candidate
  generation has already filtered forbidden moves. This avoids a second
  recursive forbidden check per Renju black candidate while still applying
  black-exact-five terminal semantics.
- Added board-level tests for black exact-five win, black overline rejection,
  white overline win, and freestyle black overline win.

Current Phase 5 validation:

```bash
cargo test --quiet
cargo build --bin gomoku_gui --bin gomocup_engine --quiet
python3 scripts/renju_oracle_compare.py --quiet
git diff --check
```

### Phase 6: VCF/VCT Integration

This is the high-risk phase. Do not start until phases 2 to 5 are stable.

Required tactical semantics:

- A black attacking line that relies on a forbidden move is not a valid black
  win.
- If black's only defence to white's threat is forbidden, that defence is not
  available.
- Apparent black A3/B4 threats that are forbidden false positives must not
  drive VCT/VCF success.
- White threats can exploit black forbidden points.

Tests:

- VCF:
  - black VCF line contains a forbidden overline point;
  - black VCF line contains a forbidden double-four point;
  - black VCF line contains a forbidden double-three point;
  - white VCF where black's only defence is forbidden.
- VCT:
  - black apparent dual-A3 win where one A3 is fake by Renju legality;
  - black true dual-A3 win;
  - white VCT using black forbidden defence.
- Root:
  - root VCF still has priority in freestyle;
  - Renju root rejects forbidden tactical moves;
  - trace identifies rejected tactical path where useful.

Validation:

- `cargo test --quiet vcf_alignment vct_alignment root_alignment`
- `python3 scripts/renju_oracle_compare.py --case-file cases/renju/tactical_cases.jsonl`
- Add at least one probe case for every tactical bug found during development.

Exit criteria:

- VCF/VCT do not produce illegal black Renju moves in fixture and fuzz tests.
- Any remaining known limitations are documented and default-disabled.

Current Phase 6 status:

- VCF has a rule-aware entry point:
  - `VCFSearcher::search_for_rule`
  - `VCFSearcher::search_with_multi_reply_for_rule`
- `ThreatBoardView` can now carry a `RuleSet`. In Renju mode it filters threat
  moves and broken-four replies through rule-aware legality before VCF consumes
  them.
- Root and non-root VCF calls pass `config.rule_set`; root VCF is enabled again
  in Renju mode.
- VCT is now rule-aware as well:
  - `VCTSearcher::search_for_rule` runs over a `ThreatBoardView` carrying the
    `RuleSet`; root VCT is enabled in Renju mode (sequential and overlap paths),
    gated by `has_vct_trigger_for_rule`.
  - Attack classification is reconstructed from rule-legal winning completions
    (`classify_attack_at_renju`): a black "four" whose only completion is an
    overline is demoted (not a four), and an open three whose open-four
    extensions are all illegal/overline is rejected as a fake threat.
  - Defender legality is rule-aware everywhere (`is_rule_legal`): forbidden
    black blocks are dropped from forced defenses and counter-threats, so a
    white four/three whose only black block is forbidden becomes unstoppable.
  - Freestyle stays byte-identical via the separate `classify_attack_at_freestyle`
    path and the `from_board` default.
- Added a VCF fixture where black's apparent direct completion is an overline:
  freestyle VCF finds the move, Renju VCF rejects it.
- Added VCF fixtures where black's apparent tactical attacking move is a
  double-four or double-three forbidden move. Freestyle VCF may return the
  move; Renju VCF must not return the forbidden point.
- Added a positive white VCF fixture: white creates a broken four whose only
  black reply is a double-three forbidden move. Freestyle reply generation sees
  the block; Renju reply generation filters it out and VCF treats white's line
  as winning.
- Added a root regression for the overline shape so root VCF cannot return the
  forbidden point.
- Added VCT fixtures: classification demotes a black overline-four, rejects a
  fake open three, and promotes a white four to unstoppable when black's block
  is forbidden; end-to-end Renju VCT never returns a forbidden double-four move
  and finds the white win when black cannot block; a root regression confirms
  VCT is engaged in Renju and stays legal.
- Added an A3 classification regression where white has a real open-three gain
  but every direct black defense is overline-forbidden. The attack must remain
  `A3` with an empty legal-defense list rather than being discarded as a fake
  three.

Known Phase 6 limitations:

- `has_vct_trigger_for_rule` still uses freestyle pattern counts as the gate,
  so it may over-trigger for black (a forbidden-dependent shape can start a VCT
  search that then correctly finds nothing). This is safe: the gate only decides
  whether to run the now-sound VCT, never the result.
- Renju attack classification simulates rule-aware completions, so Renju VCT is
  slower than freestyle. Acceptable for opt-in Renju mode; revisit if profiling
  shows it dominates.

Current Phase 6 VCF/VCT validation:

```bash
cargo test --test vcf_alignment --quiet
cargo test --test vct_alignment --quiet
cargo test --test root_alignment --quiet
cargo test --quiet
cargo build --bin gomoku_gui --bin gomocup_engine --quiet
python3 scripts/renju_oracle_compare.py --quiet
git diff --check
```
