# Engine guide

The Rust engine supports 15×15 Freestyle and Renju. Both engines are available through the same [Gomocup interface](gomocup.md) and interactive controller.

## Search paths

| Path | Implementation | Common use |
|---|---|---|
| Native NNUE | [`src/fast_engine/`](../src/fast_engine/) and [`src/nnue/`](../src/nnue/) | Pro play and timed strength matches. Incremental `FastBoard` patterns, `FastTT`, alpha-beta with tactical QVCF, and Lazy SMP. |
| Classical | [`src/search/`](../src/search/), [`src/eval/`](../src/eval/), [`src/threats/`](../src/threats/) | Casual play, weight-free operation, and pinned classical regressions. Rule-aware VCF and df-pn VCT at the root, with ABDADA for multithreaded search. |
| Legacy NNUE | Classical search with a `Mix9svq` evaluator | Compatibility and comparisons via `--legacy-nnue`. |

The native engine evaluates incrementally on both sides. It uses `Mix9svq` value and policy outputs and can load the optional `model210901.bin` classical gate. CPU kernels include a scalar fallback, x86-64 AVX2 and AVX-VNNI, and AArch64 NEON with runtime dot-product detection.

## NNUE weights

Use a directory containing the compatible Rapfi `Mix9svq` weights for the rule:

| Rule | Files |
|---|---|
| Freestyle | `mix9svqfreestyle_bsmix.bin.lz4` |
| Renju | `mix9svqrenju_bs15_black.bin.lz4` and `mix9svqrenju_bs15_white.bin.lz4` |

For Gomocup, use `--nnue-dir`, `RUST_GOMOKU_NNUE_DIR`, or `INFO nnue_dir`. Without a configured directory it uses classical search. The desktop GUI checks the explicit path and environment variable first, followed by `<exe_dir>/nnue`, the executable directory, `./nnue`, and the local development directory `opponent/rapfi-networks/mix9svq`. The Android packaging task is described in the [Android guide](android-app-design.md).

The tested weights come from Rapfi. Our [test results](perf-log.md) identify the network and configuration used for each comparison.

## Interactive presets

Pro mode uses iterative deepening with a 1-second default and 0.5-, 2.5-, and 5-second alternatives. The native path allows up to depth 99 in the UI and uses a persistent 32 MiB FastTT. If the weights for the chosen rule are unavailable, the interactive controller can use classical evaluation. The desktop and Android share a controller. They default to four search threads on devices with at least four available cores, and at most two on smaller devices.

| Casual difficulty | Depth / width | Root tactics |
|---|---|---|
| Beginner | 2 / 10 | Off |
| Normal | 4 / 20 | Off |
| Senior | 6 / 30 | VCF |
| Master | 8 / 40 | VCF and VCT |

Normal is the default difficulty when Casual mode is selected. Pro's win-rate curve displays a model estimate during play.

## Coordinates and source map

Use zero-based `(x, y) = (column, row)` in protocol, GUI, and cases. In Rust the matrix is `grid[row][col]`, and `Move` is `row * BOARD_SIZE + col`. See [`src/board.rs`](../src/board.rs) for conversions and board state.

- [`src/app.rs`](../src/app.rs) contains the shared game controller. [`src/bin/`](../src/bin/) contains the desktop GUI, protocol engine, and diagnostic binaries.
- [`src/config.rs`](../src/config.rs) supplies engine defaults. Interactive presets live in the controller.
- [`cases/`](../cases/) contains rule fixtures, pinned diffs, and match positions. [`scripts/`](../scripts/) contains match, oracle, and benchmark tools.
- [`data/static/`](../data/static/) contains classical parameter tables. The historical Python differential setup is in the [testing guide](testing.md#classical-differential-regression).
