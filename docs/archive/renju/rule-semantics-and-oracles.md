# Renju rule design and oracle setup

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

# Renju Forbidden-Move Design

This document started as the implementation plan for an optional Renju
forbidden-move rule set and now records the implemented design, validation
evidence, SlowRenju comparison, and remaining optimization work.

## Current Status

The main Renju implementation is complete:

- freestyle and Renju coexist behind `RuleSet`, with freestyle still the
  default and rule changes locked after the first move;
- black exact-five, overline, double-four, and recursive true double-three
  semantics are integrated through board play, movegen, alpha-beta, fallback,
  VCF, VCT, static eval, Gomocup, and the GUI;
- the GUI rejects and marks black forbidden points, while protocol and search
  paths reject illegal black moves;
- detector fixtures, one-dimensional exhaustion, dense stress, incremental
  eval consistency, tactical regressions, and Rapfi/`renju_forbid` comparison
  gates are in place;
- SlowRenju static tables and the relevant `ValueWide`/`ValueW` semantics have
  been audited, and a Linux fixed-depth match adapter is available.

This is not a claim that Renju strength or performance is finished. The active
work is larger strength samples, search profiling and optimization, and a
Rapfi architecture audit whose primary objective is stronger play. Renju
opening protocols such as RIF, Yamaguchi, Soosorv, and Swap2 remain out of
scope.

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
