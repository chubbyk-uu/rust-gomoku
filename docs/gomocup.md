# Gomocup guide

Build the protocol binary and run a basic transcript:

```bash
cargo build --release --locked --bin gomocup_engine
printf 'START 15\nINFO rule 4\nBEGIN\nEND\n' | target/release/gomocup_engine --depth 2 --width 8
```

This uses the default classical path when no NNUE directory is configured. To start native NNUE, pass `--nnue-dir /path/to/weights`. See the [weight filenames](engine.md#nnue-weights) before selecting a rule. `--legacy-nnue` selects the classical search with NNUE evaluation. Use `--root-profile` or `INFO root_profile 1` for search diagnostics.

## Runtime controls

| Command | Meaning |
|---|---|
| `INFO rule 0` / `INFO rule 4` | Freestyle / Renju. Rule changes apply on an empty board. The names `freestyle` and `renju` are also accepted. |
| `INFO timeout_turn N` | Per-move time in milliseconds. `0` is interpreted as a 200 ms budget. |
| `INFO timeout_match N` / `INFO time_left N` | Match-clock setup and remaining time in milliseconds. Send `time_left` as appropriate during a match. |
| `INFO thread_num N` | Search threads, default 1. Native NNUE uses Lazy SMP and classical search uses ABDADA. CLI equivalent `--threads N`. |
| `INFO max_node N` | Node budget. Gomocup native NNUE uses one search thread when this is set. |
| `INFO max_memory N` | TT sizing hint in bytes. The process also uses memory for weights, accumulators, and workers. |
| `INFO tt_bits N` | Explicit TT sizing, also available as `--tt-bits N`. This takes precedence over automatic sizing. |
| `INFO nnue_dir PATH` | Set the directory and enable NNUE. CLI `--nnue-dir` and `RUST_GOMOKU_NNUE_DIR` also supply the directory. |
| `INFO use_nnue 0` / `INFO use_nnue 1` | Switch evaluation mode. |
| `INFO fast_nnue 0` / `INFO fast_nnue 1` | Select legacy or native NNUE when NNUE is enabled. Native is the default. |

Omit clock messages for fixed-depth regression. Use `--depth D` to fix the search depth, plus `--width W` for classical candidate width. Native NNUE has its own move picker and pruning. Classical Gomocup defaults to depth 8 and width 40 for fixed search, with a depth cap of 48 for timed search. Native NNUE defaults to a depth cap of 99 and a nominal 32 MiB FastTT. Classical automatic TT sizing depends on threads, depth, and `max_memory`.

## Classical search controls

The [source defaults](../src/config.rs) apply to the classical path and its legacy NNUE variant. The native engine handles search and tactical forcing separately.

| Controls | Defaults or scope |
|---|---|
| `compute_vcf`, `compute_vct` | Enabled in classical search. |
| `root_vcf_depth`, `opponent_vcf_depth` | 8 and 7. With legacy NNUE, root VCF uses depth 16 unless explicitly overridden. |
| `vcf_multi_reply` | On. |
| `root_vct_depth`, `vct_use_dfpn` | 6 and on. Setting `vct_use_dfpn 0` selects legacy IDDFS. |
| `vct_verify_opponent_vcf_depth`, `vct_strict_and_memo_key` | 4 and on. The strict memo key option applies to IDDFS. |
| `nonroot_vcf`, `overlap_vct_alphabeta` | Off. Overlap is an optional classical search experiment. |
| `static`, `dynamic_board_margin` | Static mode on, margin 4 when dynamic mode is selected. |

In classical timed search, root VCF, VCT, and alpha-beta share the move budget. All timed modes use cooperative checks, so externally observed response time includes protocol overhead. For match and repeatability settings see [testing](testing.md).
