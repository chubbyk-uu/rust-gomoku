# SlowRenju implementation mapping

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

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

### SlowRenju Mapping Audit

Audit baseline:

- SlowRenju commit: `41007cf70762b62df77223da25c9605d0a853602`
  (2019-05-07).
- pygomoku commit: `e9b1ad8df6ce515b3cfde2ae2d5726a46fc0752d`.
- Rust audit point: `779f1ac86a0dbe4f3839270dd73c68c98691e3dd`.
- SlowRenju is GPL-3.0-or-later; this repository is also GPL-3.0, so adapted
  implementation work is license-compatible. The audit records behavior and
  provenance rather than copying source text into this document.

Static-data verification:

- SlowRenju `Common/global_value.cpp::para[]` contains 375 values. It is
  number-for-number equal to `data/static/default_eval_para.txt`.
- SlowRenju `Shape/ShapeList.cpp::ShapeList[2][3969]` contains 7938 entries.
  Both rows are exactly equal to `data/static/shape_table.txt`.
- `patterns::buckets::DOUBLE_SHAPE` is the same triangular 13-row mapping as
  SlowRenju `Value/ValueWide.cpp::doubleShape`.

Status meanings:

- **Aligned**: source structure and current tests establish the intended
  behavior.
- **Different implementation**: semantics are intended to match but Rust uses
  a safer or faster structure and still needs differential evidence where
  noted.
- **Rust extension**: deliberately outside SlowRenju parity; it must remain
  rule-correct but cannot be validated by one-to-one source mapping.
- **Open**: evidence is not yet strong enough to claim parity.

| SlowRenju surface | SlowRenju source | Rust surface | Status | Audit result |
|---|---|---|---|---|
| Rule selection | `Common/main.cpp` (`rule 0/1/4`, `fflag`, `nosix`) | `rules::RuleSet`, `protocol::gomocup` | Aligned, narrower scope | Rust supports the requested freestyle and Renju modes. Standard Gomoku (`rule 1`) remains intentionally out of scope. Rule changes are locked after the first move. |
| Renju one-dimensional shape table | `Shape/ShapeList.cpp`, `Shape/line.cpp` | `data/static/shape_table.txt`, `patterns::line` | Aligned | The complete two-row table is identical. Rust selects row 1 only for hypothetical black moves under Renju; white continues to use the freestyle row. |
| Exact five / overline terminal semantics | `Shape/line4v.cpp::foulr`, `A5`, `overline` | `rules::forbidden`, `Board::is_winning_move_for_rule` | Aligned | Black exact five wins; black overline is forbidden and is not a win. White still wins with five or more. |
| Double-four | `line4v::double4`, `B4` | `count_four_shapes_through`, four-direction sum | Different implementation, validated | Rust counts distinct four shapes, including same-direction multiplicity. One-dimensional exhaustive tests and oracle fixtures cover the rare window/overline boundaries. |
| Recursive true double-three | `line4v::A3r`, recursive `foulr` and `A5test` | `is_true_open_three_direction`, recursive `is_legal_black_gain` | Different implementation, validated | Both require an apparent three to have a legal black gain that creates a real open four. Rust has hand fixtures, dense stress, and Rapfi/`renju_forbid` comparison. |
| Forbidden reason priority | `foulr`: exact five, double-four, double-three, overline | exact five, overline, double-four, double-three | Intentional convention difference | Legality agrees. Coexisting overline/double-three positions can report different primary types; the oracle harness accepts this only when every implementation still says forbidden. |
| `ValueWide` shape/value caches | `Value/ValueWide.cpp` (`shapeM`, `valueM`, `attackM`) | `eval::EvalCaches`, `eval::local` | Aligned, different storage | Direction shapes, bucket selection, attack levels, snapshots, and incremental updates have direct Rust equivalents. Rust also maintains bucket counts for faster global evaluation. |
| Renju incremental radius | `ValueWideCompute`: `ar=(nosix||fflag)?5:4` | `value_wide_compute_for_rule`: Renju radius 5, freestyle radius 4 | Aligned | The extra Renju cell needed to observe six-in-a-row is present. |
| Non-local double-three invalidation | `pretrd3` / `trd3` forced recomputation | full-board apparent-double-three early-out refresh | Aligned semantics, different implementation | Rust deliberately uses a stateless scan so snapshot/restore stays simple. Incremental-vs-full randomized regression covers the non-monotonic legality flip. |
| Black forbidden-point eval suppression | `ComputeValue1b`: clear `valueM/attackM` for forbidden black points | `compute_bucket_and_attack_for_rule` | Aligned | Exact five keeps priority. Apparent four/overline points use the shape cache gate; apparent double-threes call the full recursive detector. Suppression-vs-detector gates passed hand and dense fixtures. |
| Evaluation parameter tables | `para[]`, `LASTEVAL`, `NEXTEVAL`, `ATTACKVALUE`, `DEFENDVALUE` | `DEFAULT_EVAL_PARA`, `EvalBucketTables` | Aligned | All 375 values are identical, including the distinct last/next/attack/defend tables and search parameters. There is no separate missing “Renju weight table” in SlowRenju; Renju changes cached black point semantics while reusing these tables. |
| Global `ValueW` evaluation | `Value/ValueW.cpp::value` | `eval::global_eval` | Aligned | Offensive/defensive bucket sums, DGN term, LAST5 recursion, NEXT4 and NEXT43 branches map directly to Rust. |
| White exploiting a forbidden black block | `ValueW.cpp:158` (`fflag & moveValue1bWide(...) < 0`) | `evaluate_last5_branch` full detector gate | Aligned | Rust uses explicit forbidden classification instead of relying on the negative cached move value. A focused test proves white wins when black's only block is forbidden. |
| Main move generation | `AI/AIx.cpp` covered set, `moveValue1bWide`, attack priorities | `search::movegen`, `CoverageTracker` | Aligned classic flow; diagnostics complete | The 32-point coverage set, move score, hostile-three bonus, forcing collapse, and ordering descend from SlowRenju/pygomoku. Renju black points are filtered through the cache prefilter plus full detector. Production-path candidate diagnostics and complete-game comparisons are available; exporting SlowRenju's internal candidate list remains an optional provenance enhancement rather than a correctness blocker. |
| Fallback move scoring | `Value/ValueB.cpp::value1b`, `AI/AIs.cpp` | `search::root::{fallback_move_score,fallback_ai_move}` | Aligned semantics, different forbidden representation | The offensive/defensive formula and weights map term-for-term. SlowRenju assigns forbidden black points a large negative offensive value; Rust excludes them before selection. Shared production scoring is fixed-tested for exact five, overline, double-four, double-three, and a recursive apparent-double-three point that remains legal. |
| Root VCF | `AIx.cpp::rootsearch`, `VCF.cpp::VCFd_hash` | `RootSearcher`, `VCFSearcher` | Aligned plus fixes | Depth normalization and core threat flow map to SlowRenju. Rust additionally supports multi-reply handling and explicitly removes forbidden attacker/defender moves. Renju tactical fixtures cover overline, double-four, double-three, and forbidden black defence. |
| Opponent VCF root filter | `AIx.cpp` variable `vctt`: test each root move against opponent `VCFd_hash` | `RootSearcher::apply_opponent_vcf_filter` | Aligned | Despite the SlowRenju variable name, this is an opponent-VCF evasion filter, not a general VCT search. |
| General VCT | no independent equivalent in this SlowRenju revision | `VCTSearcher`, rule-aware `ThreatBoardView` | Rust extension | It must be validated by Rust tactical fixtures and rule-legality invariants, not claimed as a direct SlowRenju port. |
| Search tree legality | forbidden points suppressed indirectly by `valueM`, plus `foulr` inside VCF | rule-aware movegen and tactical search; internal play skips duplicate recheck | Aligned with stronger explicit contract | Every emitted black candidate is checked or proven safe by the Renju cache gate. Tree play assumes movegen already established legality. |
| Protocol input | Gomocup `INFO rule`, board reconstruction | `GomocupProtocol` | Aligned | Standard `START -> INFO rule -> BEGIN` works; illegal Renju black input is rejected; one game cannot change rules after its first move. |
| GUI | no browser equivalent | `bin/gomoku_gui` | Rust extension | New-game rule selection, forbidden input rejection, and black-turn forbidden-point red crosses are implemented and smoke-tested. |

Current conclusion:

- Forbidden legality, static shape data, value tables, black suppression,
  non-local double-three invalidation, and the key white `ValueW` forbidden
  defence branch are substantially aligned with SlowRenju.
- Candidate selection, fallback scoring, and complete-game search behavior
  have now been exercised against a runnable SlowRenju baseline. The completed
  100-game fixed-depth gates show no evidence that Rust is weaker in either
  Renju or freestyle; remaining work is performance refinement and
  strength-focused architecture study.
- The independent Rust VCT implementation is a separate extension. Its Renju
  correctness is covered by focused tests, but its strength impact must be
  measured rather than inferred from SlowRenju.

Candidate diagnostic status:

- `diagnose_candidates` reuses the same analysis and selection functions as
  production movegen. It reports every covered point's raw and hostile-adjusted
  move value, attack levels, full-detector requirement, forbidden type,
  rule legality, retention state, rejection reason, order score, and final
  classic-order rank.
- `src/bin/renju_candidate_probe.rs` emits those diagnostics as JSON for one
  JSON case or a JSONL case file:

  ```bash
  cargo run --quiet --bin renju_candidate_probe -- \
      --case-file cases/renju/candidate_diagnostic_cases.jsonl
  ```

- The durable diagnostic fixtures cover exact-five priority, overline,
  double-four, and recursive double-three. The three forbidden fixtures expose
  the target point as `requires_full_detector=true`, with the expected
  forbidden type and `rejection_reason=forbidden`; exact five remains legal and
  is retained as the forcing candidate.
- A movegen unit regression builds the double-three position through normal
  alternating play and proves that incremental and full Renju caches produce
  identical complete diagnostics after every move.
- Refactoring movegen around the shared diagnostic analysis did not change the
  classic root surface: all 11 default root diff cases still match.
- The release movegen probe measured about 1542 ns per Renju node after
  avoiding a large by-value analysis return, versus the Phase 9 baseline of
  about 1604 ns/node. The diagnostic path itself is not called by search.
- `fallback_move_score` is the single per-point implementation used by
  production `fallback_ai_move` and fixed tests. The tests read the
  oracle-validated hand-case file and prove that exact five keeps the
  SlowRenju score `5_000_015`, all three forbidden classes are excluded, and
  `recursive_fake_three_both_gains_forbidden` stays legal with the unchanged
  `value1b` score `670`.

Completed SlowRenju gates:

1. Static extraction confirmed all 375 evaluation parameters and both rows of
   the 7938-entry shape table are identical.
2. The Windows-era source now builds through the Linux/WSL compatibility
   adapter with configurable fixed depth and width.
3. Candidate diagnostics, fallback scoring, tactical legality, and complete
   game matches are available and have been exercised at the Rust default
   depth/width.

Remaining SlowRenju/Renju gates:

1. Add independent-source strength positions when expanding beyond the
   completed 100-game paired gate; avoid treating nearby prefixes from the
   same trajectories as independent evidence.
2. Add a durable optional provenance command/test for rechecking external
   SlowRenju static data without making normal tests depend on that checkout.
3. Profile complete searches by detector, eval, movegen, VCF/VCT, alpha-beta,
   and TT cost; optimize only with legality, freestyle diff, and strength gates.
4. Audit Rapfi primarily for strength improvements. Candidate ordering,
   evaluation, tactical search, TT policy, pruning/extensions, and time
   management are higher-level targets; faster line/forbidden tables are
   supporting work when they permit deeper or broader search. Do not replace
   the search architecture wholesale.
