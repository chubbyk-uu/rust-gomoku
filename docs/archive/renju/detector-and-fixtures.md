# Forbidden-move detector and fixtures

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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
