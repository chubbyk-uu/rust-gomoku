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
classify_black_move(board, move) -> ForbiddenKind
is_forbidden_black_move(board, move) -> bool
would_black_exact_five(board, move) -> bool
would_black_overline(board, move) -> bool
black_four_count_after(board, move) -> usize
black_true_open_three_count_after(board, move) -> usize
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

### Phase 3: Fuzz And Exhaustive Local Pattern Checks

Add broader validation before touching search:

- One-dimensional exhaustive checks:
  - Enumerate windows of length 9 and 11 with values `{empty, black, white}`.
  - Place candidate black at each empty point.
  - Compare exact-five, overline, four, and apparent-three classifications
    against a simple slow reference implementation.
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

Current random oracle smoke:

- Command shape:
  `python3 scripts/renju_random_cases.py --count 1000 --seed N --fill-renju-forbid --verify-rapfi --output /tmp/renju_random_seed_N.jsonl`
- Seeds `1`, `2`, and `3` completed with zero Rapfi/`renju_forbid`
  forbidden/none mismatches.
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

### Phase 5: Terminal Semantics And Protocol Surface

Add rule mode to config/runtime surfaces:

- Library config.
- Gomocup `INFO rule freestyle|renju`.
- Probe outputs.
- GUI option later, after engine correctness is established.

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
- Whether to cache forbidden results in Renju movegen. Start simple and measure
  after correctness is established.
