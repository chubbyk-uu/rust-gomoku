# Renju Forbidden-Move Design

This document is the implementation plan for adding an optional Renju
forbidden-move rule set to `rust_gomoku`. It is intentionally written before
implementation so the hard rule questions, oracle checks, and test gates are
clear before search code changes.

## Goals

- Keep the existing freestyle behavior as the default and preserve all current
  base alignment expectations.
- Add an optional Renju rule mode where black forbidden moves are handled
  correctly.
- Implement forbidden detection first as an isolated, testable module before
  connecting it to move generation, alpha-beta, VCF, VCT, Gomocup, or GUI.
- Use external engines/libraries as oracles, especially for double-three
  corner cases.
- Start implementation work on a dedicated branch, not on `master`.

## Non-Goals For The First Implementation Pass

- Do not implement Renju opening rules such as RIF, Yamaguchi, Soosorv, or
  Swap2. The first rule mode is normal alternating play plus Renju forbidden
  moves.
- Do not change freestyle shape/eval/search behavior.
- Do not tune Renju strength until legality and tactical correctness have
  dedicated tests.

## Branch Rule

Before implementation starts:

```bash
git switch master
git pull --ff-only
git switch -c feature/renju-forbidden-rules
```

All implementation commits for this feature should stay on that branch until
the forbidden detector, oracle harness, and first integration tests are stable.

## Rule Semantics

The new rule enum should be explicit:

```text
RuleSet::Freestyle
RuleSet::Renju
```

Freestyle keeps the current behavior:

- Both sides are symmetric.
- Any empty point is legal.
- Five or more in a row wins.

Renju mode:

- Only black has forbidden moves.
- White has no forbidden moves.
- White wins with five or more in a row.
- Black wins only with exactly five in a row.
- A black move that makes exactly five is a win even if nearby shape checks
  would otherwise look forbidden.
- A black move that does not make exactly five is forbidden if it creates:
  - overline: six or more black stones in a row;
  - double-four: two or more four threats;
  - double-three: two or more true open-threes.

## Core Definitions

### Exact Five

A black exact five is a contiguous line of exactly five black stones where both
ends are not black. It may be blocked by white or edge; the important part is
that the line is not part of a longer black line.

White win detection remains five-or-more.

### Overline

An overline is a black line of length six or more after black plays the move.
It is forbidden only if the move did not also make an exact five. The exact-five
priority must be checked first.

### Four

For forbidden detection, a black four is a threat where black has at least one
legal next move that makes an exact five. Open fours and broken fours both
matter. Counting must avoid double-counting the same line pattern in a way that
does not represent two independent four threats.

The first implementation should model this through line-level pattern checks
and then validate against oracles. If a disagreement appears, store the case and
write the exact rule interpretation in the test case comment or fixture
metadata.

### True Open Three

A true open-three is not just a visual three such as `_XXX_`. It is a three
that has at least one legal extension move which creates an open four.

For double-three classification, only true open-threes in lines that contain
the original candidate move are counted. If the candidate makes another old
line become a true open-three but that line does not pass through the candidate,
that revived line is not part of the candidate's double-three count.

For Renju, "legal extension" is recursive:

- Temporarily place the original black candidate.
- Find candidate gain squares for each apparent open-three line.
- For each gain square, temporarily place black there.
- The gain square must create an open four.
- The gain square must not itself be forbidden, unless it makes an exact five
  and therefore wins by priority.

This is the central hard part. A line like `O_XXX_#` or `O_XXX_O` can look like
an open three locally because both adjacent cells around the three are empty,
but it is not a true open-three if neither adjacent move can produce a real
open four.

The recursion is non-monotonic. A gain square can look illegal because it seems
to create another forbidden shape, but that secondary forbidden shape may depend
on a continuation that is itself invalid because it creates an overline. In that
case the secondary shape is not real, the gain square can become legal again,
and the original apparent three may be a true open-three after all. The detector
must therefore verify the legality of gain squares through the same forbidden
logic instead of using a one-layer "apparent forbidden means false" shortcut.

## External References And Oracles

The implementation may study the following projects, but project code should be
written in Rust in this repository's style.

- `dhbloo/rapfi`: https://github.com/dhbloo/rapfi
  - Strong Gomoku/Renju engine.
  - Useful reference for full-engine integration.
  - Important ideas to compare against:
    - pattern table marks possible `FORBID`;
    - overline and double-four are confirmed directly;
    - double-three is recursively verified to remove false positives;
    - move picker filters forbidden moves in Renju black-to-move nodes;
    - tactical defence can change when black's only defence is forbidden.
- `realjustice/renju_forbid`: https://github.com/realjustice/renju_forbid
  - Small Go library focused on forbidden detection.
  - Useful second oracle for `0 none / 1 double-three / 2 double-four /
    3 overline`.
  - Especially useful for hand-authored SGF cases.

Because oracle implementations can disagree or contain bugs, oracle agreement
is evidence, not a substitute for local tests and documented rule reasoning.

## Implementation Phases

### Phase 0: Design And Fixtures

Files to add:

- `docs/renju-forbidden-design.md`
- `cases/renju/forbidden_hand_cases.jsonl`
- `cases/renju/oracle_mismatches.jsonl` only when mismatches are found and
  need investigation.

The first hand-written cases are not authoritative oracle data. They are
manual draft positions derived from Renju rule definitions, the current
discussion, and small local pattern reasoning. Each case must be confirmed
against Rapfi and `renju_forbid` before it can be used as an acceptance gate.
If an oracle disagrees, the fixture should be corrected or explicitly marked as
a known interpretation difference.

Validation:

- `git diff --check`

Exit criteria:

- This document exists.
- The first hand-case fixture format is agreed before detector code starts.

### Phase 1: Oracle Harness, No Engine Changes

Add scripts that can compare a local forbidden detector against external
oracles once the detector exists. The scripts can initially contain stubs and
fixture parsing, then be completed with the detector.

Current local oracle paths:

- Rapfi checkout: `/home/jerry/downloads/oracle_ws/rapfi`
- Rapfi binary:
  `/home/jerry/downloads/oracle_ws/rapfi/Rapfi/build/gcc-oracle/pbrain-rapfi`
- `renju_forbid` checkout: `/home/jerry/downloads/oracle_ws/renju_forbid`

The harness also accepts `RAPFI_BIN` and `RENJU_FORBID_ROOT` to override these
defaults.

Planned scripts:

- `scripts/renju_oracle_compare.py`
  - Input: JSONL fixture or generated random boards.
  - Output: mismatch JSON with board, side, candidate move, local result,
    Rapfi result, renju_forbid result, and notes.
- `scripts/renju_random_cases.py`
  - Generate legal-looking alternating positions.
  - Keep positions modest at first, e.g. 5 to 40 plies.
  - Prefer candidate moves near existing stones so oracle fuzz finds useful
    patterns more often.
  - Optional `--fill-renju-forbid` fills `expected` from `renju_forbid`.
  - Optional `--verify-rapfi` cross-checks forbidden/none against Rapfi.
  - Skip already terminal positions unless explicitly testing terminal logic.

Oracle adapters:

- Rapfi adapter:
  - Prefer a built Rapfi binary path from `RAPFI_BIN`.
  - Use `INFO rule 4`, `YXBOARD`, and `YXSHOWFORBID`.
  - Rapfi protocol gives a forbidden-point set, so it is used as a
    forbidden/none oracle rather than a typed oracle.
- `renju_forbid` adapter:
  - Prefer a small Go helper under `/tmp` or a script-managed temp directory.
  - Convert fixture boards to SGF with setup `AB`/`AW` stones plus the black
    candidate as the final move. This avoids accidental alternating-move
    parsing for low-level fixture boards with repeated same-side stones.
  - Read `CheckForbid` result as a typed oracle:
    `none`, `double_three`, `double_four`, `overline`.

Validation:

- `python3 scripts/renju_oracle_compare.py --help`
- `python3 scripts/renju_oracle_compare.py`
- `python3 scripts/renju_random_cases.py --help`
- `python3 scripts/renju_random_cases.py --count 5 --seed 2 --fill-renju-forbid --verify-rapfi`
- No Rust behavior changes in this phase.

Exit criteria:

- Scripts can parse fixtures and report oracle availability cleanly.
- Rapfi and `renju_forbid` invocation is reproducible on a local machine.
- The first 22 hand cases have zero mismatches against Rapfi's forbidden/none
  result and `renju_forbid`'s typed result.

### Phase 2: Pure Forbidden Detector

Add a module that does not alter existing board semantics:

- `src/rules/mod.rs`
- `src/rules/forbidden.rs`

Initial public API:

```text
RuleSet
ForbiddenKind
classify_forbidden_move(board, move, side, rule) -> Result<ForbiddenKind, BoardError>
classify_forbidden_stones(stones, candidate, side, rule) -> Result<ForbiddenKind, BoardError>
```

Important constraints:

- These functions must not mutate the input board permanently.
- Probe play/undo or local line arrays must be balanced and test-covered.
- The detector must be independent of eval cache and search state.
- It may use existing line extraction utilities when they match the needed
  semantics, but Renju-specific logic should not alter freestyle tables.

Unit tests:

- Exact five:
  - black exactly five at center;
  - black six with embedded five is overline, not exact-five win;
  - black exact five near edge;
  - white five and white overline are not black forbidden.
- Overline:
  - six contiguous black stones;
  - seven contiguous black stones;
  - overline near edge.
- Four:
  - simple open four;
  - broken four;
  - apparent four that only makes overline should not count as legal exact-five
    four;
  - two independent fours is double-four;
  - two encodings of the same four line are not accidentally double-counted.
- Three:
  - true `_XXX_` with enough outside space;
  - fake `O_XXX_#`;
  - fake `O_XXX_O`;
  - fake where the only gain square creates overline;
  - fake where the only gain square creates double-four;
  - fake where the only gain square creates double-three;
  - true where an apparent recursive double-three/double-four at the gain
    square disappears because one of its own continuations is overline;
  - true broken-three patterns such as `_XX_X_` when their gain creates open
    four;
  - double-three with two independent true threes;
  - apparent double-three where one apparent three is only revived elsewhere
    and does not pass through the original candidate;
  - apparent double-three where only one branch is true.

Oracle tests:

- Convert all hand cases to oracle format.
- Compare local result to Rapfi and `renju_forbid`.
- Every mismatch must be triaged into:
  - local bug;
  - oracle bug;
  - rule interpretation difference;
  - fixture error.

Validation:

- `cargo test --quiet rules`
- `python3 scripts/renju_oracle_compare.py --case-file cases/renju/forbidden_hand_cases.jsonl`
- `git diff --check`

Exit criteria:

- Hand cases pass locally.
- Oracle comparison has zero unexplained mismatches.
- At least one case covers the fake open-three example that motivated this
  work.

Current Phase 2 status:

- Added `src/rules/forbidden.rs` and `src/rules/mod.rs`.
- Added `src/bin/renju_rule_probe.rs` so Python oracle scripts can call the
  Rust detector without wiring it into movegen/search.
- `scripts/renju_oracle_compare.py` now compares the local Rust detector,
  Rapfi, and `renju_forbid`; `--quiet` suppresses successful per-case output
  for random batches.
- The hand cases pass the local detector and Rapfi forbidden/none checks.
  `renju_forbid` has zero unexplained typed mismatches; one coexisting-reason
  case is accepted as a reporting-convention difference and recorded in
  `cases/renju/oracle_mismatches.jsonl`.
- Added same-line double-four cases (`same_line_df_*`), where both fours share
  one line. This is a distinct code path from cross double-four (one four in each
  of two directions); same-line requires counting multiple four-shapes within a
  single direction. Four motifs are covered, each on all four axes
  (`_h`/`_v`/`_dd`/`_du` = horizontal, vertical, main and anti diagonal), 16
  cases total, all confirmed against Rapfi and `renju_forbid`:
  - `solid` — `BBB_X_BBB`: two straight fours, gaps adjacent to the candidate.
  - `broken` — `BB_BX_BB`: two broken fours, completion gaps two cells away.
  - `broken_mirror` — `BB_XB_BB`: mirror of `broken`, candidate on the other
    side of its triple.
  - `split` — `B_BXB_B`: candidate flanked by black on both sides.
  In every motif the empty completion cells keep each completion an exact five
  rather than an overline, so these are genuine double-fours. The detector
  already handled all of them (it counts multiple four-shapes within a single
  direction); these cases lock that behavior in. The `_du` cases also give the
  first coverage of the anti-diagonal `(1, -1)` direction in the suite.
- Added overline and double-three shape coverage on all four axes:
  - `overline_end6_*` (five extended to six at one end), `overline_mid6_*`
    (gap fill to six), `overline_seven_mid_*` (gap fill to seven), each on
    `h`/`v`/`dd`/`du`.
  - `double_three_straight_{hd,ha,vd,va}` fills in the four axis-pair
    combinations that were previously uncovered (the suite already had
    horizontal+vertical and the two-diagonal cross); plus
    `double_three_jump_*` for broken open-three combinations.
  - Negative coverage on `v`/`dd`/`du`: `single_straight_three_*`,
    `single_jump_three_*`, `single_four_*`, and `fake_three_blocked_*` all
    classify as `none`, guarding against false-positive forbidden reports.
- Added recursive fake-open-three coverage for the `is_legal_black_gain` path,
  which is the hardest part of double-three detection:
  - `recursive_fake_three_both_gains_forbidden`: a vertical true three plus a
    visual horizontal three whose *both* extension squares are double-four
    forbidden, so the horizontal line is not a true open-three and the candidate
    is `none`.
  - `recursive_control_only_left/right_gain_forbidden`: same skeleton with only
    one extension trapped, leaving the other legal, so the result flips back to
    `double_three`. These controls prove the recursion is load-bearing rather
    than incidental.
- Full hand-case suite is now 72 cases with zero detector/Rapfi mismatches,
  zero unexplained `renju_forbid` mismatches, and one accepted typed
  reporting-convention difference.
- Reproducible dense stress: `scripts/renju_dense_stress.py` builds positions
  from fixed forbidden-shape skeletons (`cross_three`, `ff_cross`, `ff_inline`,
  `overline`, `ol_seven`) plus seeded random nearby black/white interference,
  then labels each with the local detector. It is deterministic per `--seed`
  (skeletons iterated in sorted order, fixed per-skeleton seed offsets), so the
  same command reproduces byte-identical output. Run:

  ```bash
  python3 scripts/renju_dense_stress.py --skeleton all --count 400 --seed 20 \
      --output /tmp/renju_dense_seed20.jsonl
  python3 scripts/renju_oracle_compare.py --case-file /tmp/renju_dense_seed20.jsonl --quiet
  ```

  `renju_oracle_compare.py` runs the Rapfi oracle in parallel by default
  (`--jobs`, default `min(8, cpu count)`); each fixture is an independent Rapfi
  subprocess and `map()` keeps output deterministic. On a 2000-case batch this is
  about 7x faster (≈2 min serial → ≈18 s at `--jobs 8`). `renju_forbid` and the
  local detector already run as a single batched stdin pass.

  These skeleton batches hit the double-three recursion far more often than the
  earlier sparse random smoke, and `renju_random_cases.py --min-plies 30
  --max-plies 70` gives a complementary deep-random batch. Across these batches
  the detector matches Rapfi on the forbidden/legal boolean with zero mismatches
  (e.g. 2000/2000 on `--seed 20`). The only typed differences are the
  overline/double-three coexistence convention (see below): such positions recur
  in overline-heavy batches, so this is a *category* of accepted difference, not
  a fixed list. `renju_oracle_compare.py` auto-accepts `detector=overline` /
  `renju_forbid=double_three` when Rapfi also confirms the point is forbidden,
  and reports them as `accepted_coexisting_overline_double_three`. The reverse
  direction is not auto-accepted: if the detector reports `double_three` while
  `renju_forbid` reports `overline`, that may indicate the detector missed an
  overline and should be triaged as a mismatch.
- Not yet covered by a dedicated hand case: the non-monotonic recursion, where a
  gain square looks forbidden but is actually legal because its own forbidding
  shape depends on an illegal (overline) continuation. The detector's recursive
  structure handles it in principle and the dense batches give indirect
  confidence; a constructed case is left as follow-up.

#### Classification Convention: Coexisting Forbidden Reasons

RIF 9.2 lists overline, double-four and double-three as *parallel* forbidden
reasons. A single move can satisfy more than one at once (for example a move that
makes a seven-in-a-row and also two true threes). For such a move every tool
agrees it is forbidden; only the reported *type* can differ.

The detector returns a single `ForbiddenKind`, chosen as a primary reason with
the priority `exact_five (legal win) > overline > double_four > double_three`
(see `classify_placed_black`). This is a reporting convention, not a claim that
the lower-priority reasons are absent. It is sufficient for move generation and
search, which only need the forbidden/legal boolean; the type is used for tests
and diagnostics.

Consequence for oracle comparison: when reasons coexist, a type-blind oracle
(Rapfi) can only confirm forbidden/legal, and a typed oracle (`renju_forbid`)
may report a different reason than the detector. The hand case
`overline_and_double_three_coexist` is one example: detector reports `overline`,
`renju_forbid` reports `double_three`, both correct on legality. Dense
overline-heavy batches produce more examples such as `overline_61` and
`overline_318`, so this is handled as a general comparison convention instead
of a growing per-case allowlist.

`renju_oracle_compare.py` accepts this category only when all of the following
are true:

- the detector-labelled expected kind is `overline`;
- `renju_forbid` reports `double_three`;
- Rapfi confirms the candidate is forbidden.

The script still reads `cases/renju/oracle_mismatches.jsonl` as a durable record
of manually analyzed examples, but new dense-batch coexistence positions no
longer need to be added one by one. Exact-type agreement remains subject to this
single-reason reporting convention; forbidden/legal agreement remains the
primary oracle boundary for movegen and search integration.

A future option, if a multi-reason output is ever needed, is to return all
satisfied reasons plus a `primary_reason`; this is not required for the movegen
and search integration.

### Phase 3: Fuzz And Exhaustive Local Pattern Checks

Add broader validation before touching search:

- One-dimensional exhaustive checks:
  - Enumerate windows of length 9 and 11 with values `{empty, black, white}`.
  - Place candidate black at each empty point.
  - Compare exact-five, overline, and four-shape count against a simple slow
    reference implementation.
  - Do not treat double-three as a one-dimensional property; recursive
    true-open-three validation stays covered by hand cases and dense oracle
    stress.
- Random-board oracle fuzz:
  - Generate thousands of legal-looking positions.
  - For every empty candidate, compare local forbidden classification to
    Rapfi and `renju_forbid` when available.
  - Save mismatches as reproducible JSONL fixtures.

Validation:

- `cargo test --quiet rules`
- `python3 scripts/renju_random_cases.py --count 1000 --seed 1 --fill-renju-forbid --verify-rapfi --output cases/renju/random_seed_1.jsonl`
- Repeat with at least three seeds before integration.

Exit criteria:

- Zero unexplained oracle mismatches on the selected random sample.
- Slow reference and optimized detector agree.
- Runtime is acceptable for movegen use, or the next phase includes a clear
  caching plan.

Current one-dimensional exhaustive check:

- Implemented in `src/rules/forbidden.rs` tests.
- Width 9 enumerates `3^8 = 6561` line states; width 11 enumerates
  `3^10 = 59049` line states.
- The test fixes the candidate at the center, projects the line onto the board,
  and compares production `has_exact_five`, `has_overline`, and
  `count_four_shapes_through` against an independent slow reference that
  directly counts the center run and all five-cell windows through the
  candidate.
- Each enumerated line is laid along all four directions `(1,0)`, `(0,1)`,
  `(1,1)`, `(1,-1)`, so the vertical/diagonal coordinate transforms
  (`step`/`offset`/`contiguous_segment`) are exhausted too, not just the
  horizontal row. The slow reference is purely 1-D, so its expected value is
  shared across the four directions. Total checked: `(6561 + 59049) * 4 = 262440`
  direction/line combinations.
- Current result: zero mismatches. This makes the single-direction exact-five,
  overline, and four-shape (including dedup) logic a complete proof rather than
  sampling, independent of any external oracle.

Current random oracle smoke:

- Command shape:
  `python3 scripts/renju_random_cases.py --count 1000 --seed N --fill-renju-forbid --verify-rapfi --output /tmp/renju_random_seed_N.jsonl`
- Seeds `1`, `2`, and `3` completed with zero Rapfi/`renju_forbid`
  forbidden/none mismatches.
- The Phase 2 Rust detector also matches the typed expected results for these
  three 1000-case random batches with zero mismatches.
- Distribution:
  - seed 1: `none=996`, `double_three=3`, `double_four=1`, `overline=0`
  - seed 2: `none=989`, `double_three=8`, `double_four=1`, `overline=2`
  - seed 3: `none=992`, `double_three=5`, `double_four=2`, `overline=1`

The random generator is useful as an oracle smoke test, but it is still sparse
for forbidden hits. Keep hand/constructed fixtures as the primary edge-case
coverage, and use random output mainly to catch unexpected broad mismatches.

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

### SlowRenju Alignment Plan

Near-term goal: finish the Renju implementation against SlowRenju before
borrowing stronger or more complex ideas from Rapfi. This project is already a
Rust port in the SlowRenju/pygomoku lineage, so the first target is semantic and
strength parity with that family: forbidden detection should behave like
SlowRenju, and Renju-mode playing strength should not drop materially from the
SlowRenju-style baseline.

Execution order:

1. Build an explicit SlowRenju mapping table.
   - Map SlowRenju forbidden/rule, `ValueWide`, value table, move generation,
     VCF/VC2/VCT, and protocol-facing rule handling to the Rust modules.
   - Record every intentional difference, especially places where Rust keeps a
     simpler implementation for now.
2. Finish forbidden-rule parity.
   - Keep validating overline, double-four, and recursive double-three against
     SlowRenju-style semantics plus Rapfi/renju_forbid oracle checks.
   - Treat the current stateless `trd3`-style refresh as the baseline for
     recursive double-three eval invalidation.
   - Opening rules remain out of scope unless requested separately.
3. Finish SlowRenju-style static eval parity.
   - Reconfirm the Rust value tables match the SlowRenju/pygomoku tables.
   - Compare Rust Renju eval semantics with SlowRenju's black forbidden-point
     suppression and white use of black forbidden defences.
   - Add focused tests for any missing ValueW branch before changing weights.
4. Finish search and tactical parity.
   - Recheck movegen, VCF, VCT, fallback, and root legality so black never
     searches or selects forbidden moves under Renju.
   - Add tactical fixtures where white wins because black's only defence is
     forbidden.
   - Keep freestyle behavior and root diff stable.
5. Measure Renju performance after each change.
   - Keep the existing legality and movegen perf probes.
   - Add a dedicated `value_wide_compute_for_rule` perf probe if eval refresh
     cost becomes visible in search profiles.
   - Only consider Rapfi-style line tables after SlowRenju-style caching has a
     measured bottleneck that justifies the extra complexity.
6. Measure Renju strength.
   - Run fixed-position suites to catch obvious tactical/eval regressions.
   - Run Renju self-play or engine-vs-engine smoke matches with identical time
     controls and report win rate, draw rate, avg/p95 move time, illegal-move
     count, and timeout count.
   - Compare Renju mode against the SlowRenju-style target before adopting
     Rapfi ideas.
7. Rapfi follow-up, after SlowRenju parity is accepted.
   - Review Rapfi only for targeted upgrades: faster forbidden line tables,
     stronger candidate ordering, or tactical search improvements.
   - Keep each Rapfi-inspired idea behind a measured correctness/performance
     gate before mixing it into the default Renju path.

## Fixture Format Proposal

Use JSONL so cases are easy to append:

```json
{"name":"fake_open_three_edge","board_size":15,"moves":[{"x":9,"y":7,"side":-1},{"x":11,"y":7,"side":1},{"x":12,"y":7,"side":1},{"x":13,"y":7,"side":1}],"candidate":{"x":10,"y":7},"expected":"none","notes":"O_XXX_ with edge-limited right side is not a true open three"}
```

Fields:

- `name`: stable test id.
- `board_size`: initially always 15.
- `moves`: explicit stones, not necessarily alternating for low-level detector
  tests.
- `candidate`: black move to classify.
- `expected`: `none`, `double_three`, `double_four`, or `overline`.
- `expected_win`: optional bool for exact-five priority cases.
- `notes`: short human reason.
- `oracle`: optional field recording Rapfi or `renju_forbid` disagreement if
  a case is intentionally kept as a known difference.

## Mismatch Handling

Every oracle mismatch should become a durable artifact before code changes:

1. Save the board and candidate into `cases/renju/oracle_mismatches.jsonl`.
2. Record local result, Rapfi result, `renju_forbid` result, and command used.
3. Reduce the case if possible.
4. Decide whether the mismatch is local, oracle, fixture, or interpretation.
5. Promote resolved cases into `forbidden_hand_cases.jsonl`.

No mismatch should be silently ignored.

## Acceptance Gates Before Search Integration

Search integration should not begin until:

- All unit tests for `src/rules/forbidden.rs` pass.
- Hand fixtures pass.
- Random oracle comparison has zero unexplained mismatches for the agreed
  seeds and sample size.
- Freestyle tests still pass.
- The implementation branch has no unrelated changes.

## Open Questions

- Whether to expose `RuleSet::Standard` separately. Rapfi distinguishes
  freestyle, standard, and Renju. This project only needs freestyle and Renju
  for now unless a concrete use case appears.
- Whether forbidden move in Gomocup should be reported as illegal input,
  immediate loss, or internal engine-only filtering. This needs a protocol
  decision before Phase 5.
