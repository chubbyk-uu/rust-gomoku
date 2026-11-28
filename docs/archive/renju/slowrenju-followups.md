# SlowRenju follow-up tests and regression gates

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

### Remaining Work After SlowRenju Alignment

1. **Strength confidence**
   - The initial 100-game fixed-depth gate is complete and shows no evidence
     that Rust is weaker than SlowRenju.
   - Future strength expansion should use additional source games rather than
     more nearby prefixes from these same 18 trajectories.
   - Keep VCT-off and complete-product lanes separate so independent VCT value
     remains measurable.
2. **Performance**
   - Compare nodes and time-per-node before concluding the remaining gap is
     implementation speed rather than additional Rust search work.
   - Profile forbidden checks, rule-aware eval refresh, candidate generation,
     opponent-VCF filtering, VCT misses, TT, and alpha-beta separately.
   - Keep `tests/renju_perf.rs` as the micro-regression gate, but judge
     end-to-end changes with same-position probes and matches.
3. **Rapfi architecture audit**
   - Map Rapfi line-pattern/forbidden tables, incremental board state, move
     ordering, tactical search, TT, pruning/extensions, time management, and
     parallel search to the current Rust modules.
   - The primary success criterion is increased playing strength, not source
     similarity or raw microbenchmark speed. Performance work matters because
     it can buy deeper search under the same budget.
   - Select one idea at a time. Each Rapfi-inspired experiment must preserve
     Renju legality, pass freestyle diffs, and beat the appropriate base in
     paired strength matches; latency and node changes must be reported even
     when the accepted benefit is strength.
4. **Rule surface intentionally deferred**
   - Opening protocols remain separate future features.
   - Equal-time product matches remain blocked until one deadline covers root
     VCF, opponent-VCF filtering, VCT/verification, and alpha-beta together.

### Freestyle Regression Against SlowRenju

SlowRenju is also useful as a complete-game freestyle regression opponent. Run
the same Linux adapter with Gomocup rule `0` and the Rust engine in
`RuleSet::Freestyle`, using paired openings and matched fixed depth/width.

This lane complements rather than replaces the Python reference gates:

- Python reference/root diffs remain the semantic authority for classic
  move/score/nodes/trace alignment.
- Rust-vs-SlowRenju freestyle matches detect practical playing-strength
  regressions that fixed-position diffs may not expose.
- Compare experimental Rust search builds with the current mainline before
  accepting search changes.
- Report wins/losses/draws, color split, avg/median/p95/max move time,
  timeouts, nodes when available, and the exact search/tactical settings.

The first 100-game freestyle gate reused
`cases/renju/strength_100_prefixes.jsonl` so the rule comparison has the same
positions, colors, fixed depth 8 / width 40, 120-ply cap, and eight-way
parallelism as the Renju batch. Both engines received Gomocup rule `0`; Rust
kept its complete base VCF/VCT configuration.

Results:

- Rust 55, SlowRenju 43, draw 2, errors/timeouts 0;
- Rust as black: 33 wins, 15 losses, 2 draws;
- Rust as white: 22 wins, 28 losses;
- paired openings: 41 split pairs, 6 Rust sweeps, 1 SlowRenju sweep, and 2
  pairs containing a draw;
- Rust score was 56.0%, with no evidence that current freestyle playing
  strength is below SlowRenju on this gate.

The same revision also passed all 11 classic root diff cases with all 18
reported fields aligned. Together, the match and diff results show no current
freestyle regression signal. They do not replace a direct old-revision-vs-new-
revision match when a future change specifically claims to preserve strength.

Parallel-batch move timing was Rust avg/median/p95/max
912.578/45.926/3631.614/8596.887 ms and SlowRenju
852.695/11.202/3165.806/12792.387 ms. Compared with the Renju batch, Rust
freestyle was materially faster, especially at median and maximum latency.
SlowRenju's average was similar across the two rule modes. As with the Renju
batch, CPU contention means these figures are end-to-end throughput evidence,
not clean per-node microbenchmarks.

The direct historical A/B is also complete. The pre-Renju baseline is
`dafb664` (`Add GPL license declaration`), the last commit before the Renju
design/oracle work began. It was built in a detached worktree and played the
same 50 paired prefixes against the current `20dfab4` base profile at fixed
depth 8 / width 40:

- current 49, pre-Renju 49, draw 2, errors/timeouts 0;
- current as black: 27 wins, 22 losses, 1 draw;
- current as white: 22 wins, 27 losses, 1 draw;
- all 49 decisive opening pairs split 1:1 and the remaining pair was a double
  draw;
- all 50 paired games had exactly identical coordinate/side sequences after
  swapping engine assignments;
- both versions searched 1391 moves and reported the same aggregate TT
  generation counts.

This is stronger than a 50% match result: under this deterministic fixed-search
gate, current freestyle base behavior is sequence-identical to the pre-Renju
baseline. The classic 11-case reference diff also remained fully aligned.

Parallel timing was current avg/median/p95/max
942.377/90.531/3733.413/11472.891 ms versus pre-Renju
922.284/89.250/3614.786/11068.765 ms. The roughly 2% aggregate difference is
too small to call a performance regression under eight-way CPU contention;
use same-position single-process benchmarks for that decision.

The historical check of the retired Fast profile used the same 100 games:

- fast 52, base 44, draw 4, errors/timeouts 0;
- fast as black: 28 wins, 18 losses, 4 draws;
- fast as white: 24 wins, 26 losses;
- paired openings: 39 split pairs, 6 fast sweeps, 1 base sweep, and 4 pairs
  containing a draw;
- fast scored 54.0%, passing the project requirement that fast must not score
  below 50% against base.

Parallel timing was fast avg/median/p95/max
1176.237/454.412/3991.700/13259.241 ms versus base
1114.297/339.188/4248.023/13863.986 ms. Fast improved p95/max in this batch
but worsened avg/median and searched slightly more moves. The experiment passed its strength gate and had mixed performance results.

### Post-optimization re-run (bit-identical perf work)

After the search/eval performance series (release LTO profile; branchless
shape reader; build-the-four-forbidden-lines-once; incremental
apparent-double-three set — all proven bit-identical by the eval/movegen/root
alignment suites, the incremental-vs-full stress test, and a snapshot/restore
revert test; see `docs/perf-log.md`), both 100-game gates were re-run at the
same fixed depth 8 / width 40 on `strength_100_prefixes.jsonl`, twelve-way
parallel:

- Renju: Rust 55, SlowRenju 44, draw 1; 4 of Rust's wins were adjudications of
  SlowRenju forbidden black moves, so normal games were Rust 51, SlowRenju 44,
  draw 1 (~53.6%) — statistically the same as the original 52.6% gate.
- Freestyle: Rust 55, SlowRenju 45, draw 0 (55.0%) — the same as the original
  56.0% gate.

Strength is unchanged, as expected: at fixed depth/width the optimizations are
bit-identical, so the engine plays the same moves; the +/-1 game drift comes
from SlowRenju's time-based search under different parallelism, not from the
Rust side. The engine is measurably faster end-to-end: parallel-batch Rust move
time fell to avg 1148 ms (Renju, from 1469 ms) and avg 839 ms (Freestyle, from
912 ms), the latter now on par with SlowRenju's ~853 ms average. These remain
contended end-to-end figures, not single-process microbenchmarks; the clean
per-node deltas are in `docs/perf-log.md`.

### Forced-move selection parity fix (tactical search off)

A latent search bug surfaced while adding the difficulty presets: with tactical
search disabled (Beginner/Junior/Intermediate), the root win-priority and single-forcing branches
returned the first candidate in scan order instead of the strongest forcing
move, so the engine could miss an immediately available win. The bug is
rule-independent (present in both freestyle and Renju) and was mirrored from the
Python reference, which had the same defect; SlowRenju was already correct.

The fix selects the highest-`order_score` candidate (lowest move on ties), which
equals the presorted order, and was applied to the Rust engine
(`src/search/movegen.rs`) and to `pygomoku` in lockstep so the diff harness
stays bit-identical. It only changed behavior at the root, where the candidate
list is intentionally left in scan order. Regressions guard the path with
VCF/VCT off: `cases/diff/root_win_priority_no_tactics_31.json` (Rust-vs-reference
parity), `src/search/root.rs` (Rust), and `tests/test_search.py` (pygomoku).

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
