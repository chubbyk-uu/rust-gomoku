# Renju rules and validation

## Supported play

Both rules use a 15×15 board with alternating turns. In Freestyle either side wins with five or more contiguous stones. In Renju, white keeps that win condition and black wins with exactly five. Black's exact five has priority over forbidden shapes produced by the same move.

A black move without an exact five is forbidden when it makes an overline of six or more, two or more fours, or two or more true open threes. A four must have an exact-five continuation. A true three must have a legal continuation to an open four. The continuation's legality can itself require recursive double-three checks. Only the lines through the original move contribute to its double-three count.

The engine supports alternating play with black forbidden moves. Renju opening procedures such as RIF, Yamaguchi, Soosorv, and Swap2 are outside the supported protocol surface.

## Engine behavior

- The general-purpose checker is [`src/rules/forbidden.rs`](../src/rules/forbidden.rs). The native search uses [incremental board patterns](../src/fast_engine/board.rs) and rule-aware move checks. Tactical search must account for a defending move's legality as well as the attack.
- The desktop and Android interfaces mark forbidden black moves and reject them as player input.
- Gomocup accepts rule `4`. When black has no legal point, the engine outputs a forbidden point for referee adjudication and records a white win internally. The desktop and Android controller ends the game as a white win after the last legal move. The match harness uses the [Renju referee](../src/bin/renju_referee.rs).
- Replaying and undoing a terminal move must keep the board, move count, side to move, and hash consistent.

## Rule regression

Hand-written cases live in [`cases/renju/`](../cases/renju/). Use the rule probe, oracle harness, and tactical regressions for changes that affect forbidden classification or searched continuations.

```bash
python3 scripts/renju_oracle_compare.py --case-file cases/renju/forbidden_hand_cases.jsonl
python3 scripts/renju_dense_stress.py --count 400 --seed 20 --output /tmp/renju_dense_seed20.jsonl
python3 scripts/renju_oracle_compare.py --case-file /tmp/renju_dense_seed20.jsonl --quiet
```

The oracle harness can use local Rapfi and `renju_forbid` installations. Use `--require-oracles` when both comparisons are required. Accepted reporting-convention differences are documented in the [oracle mismatch fixtures](../cases/renju/oracle_mismatches.jsonl). For strength matches, see [testing](testing.md). The detector design, implementation phases, and SlowRenju comparisons are available in the [Renju archive](archive/README.md#renju).
