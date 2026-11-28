# SlowRenju strength comparison

> Snapshot from the former [renju-forbidden-design.md](../../renju-forbidden-design.md). Commands, paths, baselines and acceptance criteria describe the captured version.

### SlowRenju Strength Comparison Contract

The checked SlowRenju revision does not use the same default search envelope as
Rust:

- its Gomocup entry calls `rootsearch(24, 60, 1, 1)`;
- `pDepth=24` means the iterative loop calls alpha-beta through depth 25;
- iterative deepening is normally stopped by its time thread before that cap;
- root VCF uses requested depth 8 and opponent-VCF root filtering uses depth 7;
- it has no independent general VCT equivalent to Rust's `VCTSearcher`;
- Rust fixed defaults are depth 8 / width 40, while timed search uses maximum
  depth 25 / width 40.

Current Rust timing behavior also needs to be stated precisely:

- the GUI always uses fixed depth/width;
- Gomocup without `INFO timeout_turn` or `INFO time_left` uses fixed
  depth/width;
- Gomocup with a time INFO passes a deadline into iterative alpha-beta;
- root VCF, root VCT, VCT verification, and opponent-VCF root filtering run
  outside that alpha-beta deadline and can make total move time exceed the
  nominal limit;
- the existing match script sends no time INFO by default, so its current Rust
  games are fixed-depth games.

Therefore a single default-vs-default match cannot distinguish engine strength
from parameter differences. Use three separate lanes:

1. **Search-semantic baseline**
   - make SlowRenju depth and width configurable without changing search code;
   - run both engines at identical fixed depth, root width, and child-width
     ratio 1:1;
   - use root VCF depth 8 and opponent VCF depth 7 on both;
   - disable Rust general VCT because SlowRenju has no corresponding module;
   - use the Rust VCF mode that most closely reproduces the SlowRenju revision,
     with any stronger multi-reply behavior reported separately.
2. **Evaluation isolation**
   - disable root VCF, opponent VCF, and Rust VCT on both sides;
   - use identical fixed depth and width;
   - treat this only as a diagnostic of eval/movegen/search-core divergence,
     not as the product-strength result.
3. **Equal-time product match**
   - give both engines identical `timeout_turn`, `timeout_match`, and
     `time_left` inputs;
   - keep SlowRenju's normal VCF behavior;
   - enable the complete Rust Renju path, including its VCF fixes and general
     VCT;
   - Rust being stronger is acceptable and expected from additional correct
     search features. A clear loss requires diagnosis by replaying the losing
     positions through lanes 1 and 2.
   - this lane is blocked until one move-level deadline is propagated through
     Rust root VCF, VCT, VCT verification, opponent-VCF filtering, and
     alpha-beta. Fixed-depth lanes do not depend on that work.

All lanes require:

- an independent Renju referee that rejects black overline, double-four, and
  double-three moves and applies exact-five priority;
- paired openings with colors swapped;
- identical board size, rule, static-board mode, process count, and comparable
  TT memory;
- release builds with no profiling instrumentation in the timed result;
- reporting wins, losses, draws, illegal moves, timeouts, avg/median/p95/max
  move time, and available depth/node/tactical-path diagnostics.

The repository now contains a minimal Linux/WSL compatibility build:

```bash
python3 scripts/build_slowrenju_linux.py \
    --source path/to/SlowRenju
```

It produces `target/release/slowrenju_linux`. The build copies the selected
SlowRenju revision into `target/`, mechanically normalizes its Windows
backslash includes and old allocator spelling, then compiles the original
AI/eval/shape/VCF/hash sources with a separate Linux Gomocup entry. The
SlowRenju checkout is not modified.

The compatibility entry exposes:

- `--depth N`: maximum completed alpha-beta iteration depth. It maps to
  SlowRenju `pDepth=N-1`;
- `--width N`: SlowRenju root width;
- `--ratio-num` / `--ratio-den`: child-width ratio, default 1:1;
- `INFO rule`, `INFO max_node`, `INFO compute_vcf`, `INFO static`,
  `INFO sr_depth`, and `INFO sr_width`.

Linux build and protocol smoke passed for `START`, `ABOUT`, and `BOARD`. Renju
smoke positions for overline and cross double-three returned moves other than
the known forbidden candidate. These checks establish build/protocol viability;
the match harness now also has a rule-aware referee before recording results.

`src/bin/renju_referee.rs` is a persistent line-oriented JSON referee used by
`scripts/run_gomocup_match.py --rule renju`. It:

- replays the complete alternating prefix under Renju rules;
- rejects black overline, double-four, and double-three moves;
- preserves exact-five priority;
- declares black victory only for exact five and white victory for any line of
  at least five;
- reports forbidden type so an illegal engine move is recorded as a loss.

The referee shares the Rust detector implementation, so it protects match
adjudication from the old freestyle `>=5` rule but is not an independent oracle.
Before a strength result is treated as final, all black moves from the saved
games should also be batch-checked against the existing Rapfi/renju_forbid
oracle chain.

The first end-to-end smoke used one center opening with colors swapped:

```bash
python3 scripts/run_gomocup_match.py \
    --rule renju \
    --opening-set 5 \
    --limit-openings 1 \
    --engine-a-side both \
    --jobs 1 \
    --max-moves 80 \
    --engine-a-name rust \
    --engine-b-name slowrenju \
    --engine-a-command \
      'target/release/gomocup_engine --depth 1 --width 8' \
    --engine-b-command \
      'target/release/slowrenju_linux --depth 1 --width 8' \
    --engine-a-info compute_vct=0 \
    --engine-a-info vcf_multi_reply=0 \
    --output /tmp/rust_vs_slowrenju_renju_smoke.json
```

It completed with no illegal moves, protocol errors, or timeouts:
SlowRenju won one game and the other reached the configured 80-ply draw cap.
This depth-1 result is only a harness smoke, not strength evidence. Rust had one
3.36-second move despite alpha-beta depth 1, confirming that root tactical
search cost must be reported separately from alpha-beta depth.

The first fixed-search pilot used depth 2 / width 8, the same VCF requests,
Rust VCT disabled, and Rust multi-reply VCF disabled. Across five one-stone
openings with colors swapped:

- Rust: 8 wins;
- SlowRenju: 2 wins;
- draws, illegal moves, protocol errors, and timeouts: 0;
- Rust timing: avg 3.348 ms, median 1.039 ms, p95 15.330 ms, max 52.413 ms;
- SlowRenju timing: avg 0.904 ms, median 0.526 ms, p95 3.165 ms, max 7.254 ms.

This result is not strength evidence. Four of those five openings are symmetric
corner placements, so the 10 games are neither independent nor balanced, and
depth 2 is too shallow. All 201 black moves were still useful as a legality
audit: Rust, Rapfi, and `renju_forbid` reported every move legal with zero
mismatches.

Two further fixed-search pilots used the same VCF alignment:

| Depth / width | Rust | SlowRenju | Draw | Rust avg / p95 / max | SlowRenju avg / p95 / max |
|---|---:|---:|---:|---:|---:|
| 3 / 12 | 5 | 5 | 0 | 6.217 / 19.527 / 67.898 ms | 1.945 / 5.402 / 13.540 ms |
| 4 / 20 | 8 | 2 | 0 | 38.741 / 45.448 / 1716.252 ms | 7.143 / 15.716 / 93.102 ms |

At depth 3 every game was won by the black engine, so the 5:5 result was
entirely a color/opening effect. The depth-4 8:2 result reused the same
symmetry-heavy one-stone set and must also not be treated as evidence that Rust
is stronger. The depth-3 and depth-4 logs contributed another 205 and 277 black
moves respectively; both batches passed Rust/Rapfi/renju_forbid with zero
legality mismatches.

The depth-4 maximum was reproduced from its 49-ply prefix with root profiling:

- total Rust move time with normal aligned VCF: about 1697 ms;
- all four alpha-beta iterations combined: about 7.2 ms;
- total with all VCF disabled: about 10.6 ms;
- total with root VCF enabled but `opponent_vcf_depth=0`: about 10.7 ms.

Therefore the long tail came almost entirely from
`RootSearcher::apply_opponent_vcf_filter`, not from alpha-beta, static eval, or
ordinary Renju move legality filtering. The filter first detects an opponent
VCF and then reruns VCF after each candidate defence. SlowRenju has the same
high-level root filter, but its implementation is substantially faster in
these pilots. This is the next concrete performance investigation; it should
be optimized without changing the emitted legal root set.

Default-depth correction:

- The Linux adapter now sets a 24-hour CPU budget and resets `ts` before every
  search, preventing SlowRenju's retained `comphalfend` check from truncating a
  fixed-depth run.
- On the center-first position, both engines completed iteration depth 8 at
  width 40. SlowRenju selected `(7,6)` in about 3.38 seconds and Rust selected
  the mirror-equivalent `(7,8)` in about 4.26 seconds.
- A center-opening color swap produced 1:1, with black winning both games.

For a less biased comparison, `cases/renju/strength_prefixes.jsonl` contains
three legal, non-symmetric 10-ply prefixes: an asymmetric corner game, a central
game, and an edge attack. Every prefix black move was accepted by Rust, Rapfi,
and `renju_forbid`.

At depth 8 / width 40, with Rust VCT and multi-reply disabled to match the
SlowRenju feature set, the six paired games finished 3:3. Each prefix produced
the same winning color regardless of which engine played that color. The 69
played black moves passed all three legality checks.

Repeating the same six games with the actual Rust base defaults, including VCT
and multi-reply VCF, also finished 3:3 with the same winning-color pattern.
Those games contributed 73 more black moves with zero oracle mismatches.

The current small-sample conclusion is therefore:

- no evidence that Rust Renju strength is materially below SlowRenju at the
  real default depth and width;
- no evidence yet that Rust is stronger either;
- the clearest measured difference is speed: in the aligned default-depth
  prefix sample Rust averaged about 259 ms per move versus SlowRenju 106 ms,
  with materially worse long-tail latency.

Expanded default-depth comparison:

- A later batch selected 14 dihedral-unique, locally Renju-legal 10-ply
  prefixes from the standard match cases and played both colors, for 28 games
  per configuration at depth 8 / width 40.
- With Rust VCT disabled, the recorded score was Rust 15,
  SlowRenju 13. Two Rust wins were match-referee adjudications after
  SlowRenju played black forbidden moves: one overline and one double-three.
  Excluding those two illegal games, normally completed games were 13:13.
- With the normal Rust VCT path enabled, the recorded score was Rust 16,
  SlowRenju 12. One SlowRenju overline remained; excluding it, normally
  completed games were Rust 15, SlowRenju 12.
- VCT changed one normally completed game from a SlowRenju win to a Rust win.
  It also changed the continuation of the position where the VCT-disabled run
  ended with SlowRenju's double-three.
- Rust average move time increased from about 784 ms with VCT disabled to
  959 ms with VCT enabled. SlowRenju averaged about 476 ms and 548 ms in the
  respective batches. The batches are paired but still too small for a strong
  win-rate claim; they do show no current evidence of a material Rust strength
  deficit and confirm a significant Rust latency gap.

The planned 100-game fixed-depth gate is now complete. The durable opening set
is `cases/renju/strength_100_prefixes.jsonl`:

- 50 locally Renju-legal positions, each played with colors swapped;
- all positions are unique under the eight square-board symmetries;
- positions come from 18 source trajectories at plies 10, 14, 18, 22, and 26,
  so the set includes opening and early/middle-game states but is not 50
  statistically independent source games;
- all 398 black moves in the preset prefixes passed the Rust detector, Rapfi,
  and `renju_forbid` before the match.

Both engines used fixed depth 8 / width 40. Rust used its complete base
configuration, including rule-aware VCF and VCT. Eight games ran in parallel on
a 24-logical-CPU host:

- recorded result: Rust 54, SlowRenju 45, draw 1, timeout 0;
- four Rust wins were adjudications after SlowRenju, playing black, selected
  one overline and three double-three moves;
- excluding those illegal games, normally completed games were Rust 50,
  SlowRenju 45, draw 1, or 52.6% Rust points;
- Rust as black scored 27 wins, 22 losses, and 1 draw;
- Rust as white had 23 normal wins and 23 normal losses, plus the four
  SlowRenju forbidden-move losses;
- paired openings produced 39 split pairs, 7 Rust sweeps, 3 SlowRenju sweeps,
  and 1 pair containing the draw.

The post-match audit deduplicated the played black positions into 1174
fixtures: 1170 successful black moves plus the four rejected candidates. Rust,
Rapfi, and `renju_forbid` agreed on every fixture with zero mismatches. No Rust
move was illegal.

The parallel batch measured Rust at avg/median/p95/max
1469.849/143.775/5780.036/22399.597 ms per move and SlowRenju at
825.720/15.496/3095.064/12547.858 ms. These numbers confirm the end-to-end
latency gap but are not clean single-process benchmarks: eight simultaneous
games introduced CPU contention, especially for long tactical searches.
Use same-position and single-process profiling before attributing the ratio to
per-node implementation cost.

The SlowRenju forbidden losses were not caused by failing to send Gomocup rule
4. The adapter sends `INFO rule 4` after `START`/`RESTART`, and SlowRenju sets
`fflag=1`. Both offending positions reproduce directly with rule 4. Source
inspection confirms the failure contract: **SlowRenju soft-penalizes forbidden
black points rather than excluding them.**

`SlowRenju/Value/ValueB.cpp:69-87`, for a black move (`c==1`) under `fflag`
with no exact-five, sets the offensive score to a finite `-100000` for
double-four (`B4l>=2`), overline (`A6l`), or a `foulr()`-confirmed true
double-three (`A3l>=2`):

```cpp
if(fflag && !A5l && c==1) {
    if(B4l>=2)                       affensive=-100000; // double-four
    else if(A6l)                     affensive=-100000; // overline
    else if(A3l>=2 && l4v.foulr(...)) affensive=-100000; // true double-three
}
```

Consequences:

- `-100000` is a finite penalty, **not** the `-1000000000` "never select"
  sentinel used elsewhere (`valuee[]`);
- candidate generation never filters forbidden points out — they only carry a
  low score, and the defensive term can still be large (`A5l*50000`, double
  four-block `2000`, etc.);
- root candidate initialization marks every empty point and the search uses
  `vbw <= 0` as the effective filter, with no final `foulr()` legality gate
  before returning a move.

So when a forbidden point is the only move that blocks a winning white threat,
or when every legal alternative scores lower, `-100000` can still be the argmax
and SlowRenju plays an illegal move. That is exactly what produced the four
adjudicated losses (one overline, three double-three) — not a Rust
misjudgement: Rust, Rapfi, and `renju_forbid` agreed those points are forbidden.

Rust deliberately uses correct-by-construction hard exclusion instead: Phase 8
`compute_bucket_and_attack_for_rule` returns `(0,0)` for forbidden black
points, and `is_rule_legal_for_movegen` drops them via the full detector at
move generation, with the same rule-legality contract re-enforced at external
play boundaries. A forbidden black point therefore never enters the candidate
set, so the SlowRenju result is a reference-implementation Renju-compliance
defect, not a behavior to copy.
