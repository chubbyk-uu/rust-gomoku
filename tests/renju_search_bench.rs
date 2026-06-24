//! Single-process, fixed-position full-move search baseline.
//!
//! Unlike `renju_perf.rs` (which probes one legality call or one movegen node),
//! this runs a complete fixed depth/width root search per position and reports
//! `nodes`, wall `ms`, and `ns/node`. The point is to separate *per-node cost*
//! from *node count* when comparing Freestyle vs Renju (and, externally, Rust
//! vs SlowRenju): if Renju searches similar node counts but spends more ns/node
//! the cost is implementation overhead; if it expands many more nodes the gap is
//! ordering/pruning, not raw speed.
//!
//! Run (release matters; the parallel-batch match numbers are CPU-contended and
//! are not a clean per-node measurement):
//!
//! ```bash
//! cargo test --release --test renju_search_bench -- --ignored --nocapture
//! # optional overrides:
//! BENCH_DEPTH=8 BENCH_WIDTH=40 BENCH_POSITIONS=6 \
//!   cargo test --release --test renju_search_bench -- --ignored --nocapture
//! ```

use std::time::Instant;

use rust_gomoku::{load_default_config, xy_to_move, Board, RootSearcher, RuleSet, SearchLimits};

#[derive(serde::Deserialize)]
struct Prefix {
    name: String,
    moves: Vec<[usize; 2]>,
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Replay a pre-validated prefix (moves alternate black, white, ... from ply 0)
/// under the given rule so the board state matches the engine's expectations.
fn build_board(moves: &[[usize; 2]], rule: RuleSet) -> Board {
    let mut board = Board::new();
    for (index, m) in moves.iter().enumerate() {
        let side = if index % 2 == 0 { 1 } else { -1 };
        board
            .play_for_rule(xy_to_move(m[0], m[1]).unwrap(), Some(side), rule)
            .unwrap_or_else(|err| panic!("prefix move {m:?} illegal under {rule:?}: {err:?}"));
    }
    board
}

#[test]
#[ignore = "performance baseline; run with --ignored --release"]
fn search_baseline_fixed_positions() {
    let depth: i32 = env_or("BENCH_DEPTH", 8);
    let width: usize = env_or("BENCH_WIDTH", 40);
    let count: usize = env_or("BENCH_POSITIONS", 4);

    let raw = include_str!("../cases/renju/strength_100_prefixes.jsonl");
    let prefixes: Vec<Prefix> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(count)
        .map(|line| serde_json::from_str(line).expect("prefix line parses"))
        .collect();

    let limits = SearchLimits {
        max_depth: depth,
        root_width: width,
        node_limit: None,
        time_limit_ms: None,
    };

    println!(
        "fixed-position search baseline (depth {depth}, width {width}, {} positions):",
        prefixes.len()
    );
    for rule in [RuleSet::Freestyle, RuleSet::Renju] {
        let mut total_nodes = 0usize;
        let mut total_ns = 0u128;
        println!("  rule {rule:?}:");
        for prefix in &prefixes {
            let mut config = load_default_config();
            config.rule_set = rule;
            let mut board = build_board(&prefix.moves, rule);
            // Fresh searcher per position => no TT carryover between rows.
            let mut searcher = RootSearcher::new(config);

            let start = Instant::now();
            let result = searcher.search(&mut board, Some(limits));
            let elapsed = start.elapsed();

            let ns = elapsed.as_nanos();
            let ns_per_node = if result.nodes > 0 {
                ns as f64 / result.nodes as f64
            } else {
                0.0
            };
            total_nodes += result.nodes;
            total_ns += ns;
            println!(
                "    {:<42} move={:>3} score={:>8} depth={:>2} nodes={:>9} {:>7.1}ms {:>6.0}ns/node",
                prefix.name,
                result.move_,
                result.score,
                result.depth,
                result.nodes,
                ns as f64 / 1e6,
                ns_per_node,
            );
        }
        let agg_ns_per_node = if total_nodes > 0 {
            total_ns as f64 / total_nodes as f64
        } else {
            0.0
        };
        println!(
            "    TOTAL nodes={total_nodes} {:.1}ms {agg_ns_per_node:.0}ns/node",
            total_ns as f64 / 1e6
        );
    }
}
