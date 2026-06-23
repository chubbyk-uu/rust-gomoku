//! Renju forbidden-detection performance probes.
//!
//! These are `#[ignore]`d so they never run in the normal `cargo test` pass.
//! Run on demand (release is important):
//!
//! ```bash
//! cargo test --release --test renju_perf -- --ignored --nocapture
//! ```
//!
//! `gate` measures one `is_legal_move_for_rule` call; `movegen` measures one
//! interior-node candidate generation. Compare the Freestyle vs Renju rows to
//! see the forbidden-detection overhead and track optimisation progress.

use std::time::Instant;

use rust_gomoku::{
    generate_candidates, load_default_config, recompute_all, recompute_all_for_rule, xy_to_move,
    Board, EvalCaches, RuleSet, BLACK,
};

/// A dense, contested midgame so movegen yields many covered candidates, like an
/// interior search node. Strictly alternating so `Board::play` accepts it.
fn dense_midgame() -> Board {
    let mut board = Board::new();
    let seq = [
        (7, 7),
        (8, 7),
        (7, 8),
        (8, 8),
        (6, 8),
        (9, 7),
        (6, 6),
        (9, 9),
        (5, 9),
        (10, 6),
        (8, 5),
        (6, 10),
        (9, 5),
        (5, 6),
        (10, 8),
        (4, 7),
        (7, 10),
        (11, 7),
        (5, 5),
        (10, 10),
    ];
    for (i, (x, y)) in seq.into_iter().enumerate() {
        let side = if i % 2 == 0 { 1 } else { -1 };
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    board
}

#[test]
#[ignore = "performance probe; run with --ignored --release"]
fn perf_legality_gate() {
    let mut board = dense_midgame();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let empties: Vec<u16> = (0u16..225).filter(|&m| board.is_legal_move(m)).collect();
    let iters = 20_000usize;
    println!("legality gate over {} empty squares:", empties.len());
    for rule in [RuleSet::Freestyle, RuleSet::Renju] {
        let start = Instant::now();
        let mut acc = 0usize;
        for _ in 0..iters {
            for &m in &empties {
                if board.is_legal_move_for_rule(m, BLACK, rule) {
                    acc += 1;
                }
            }
        }
        let calls = iters * empties.len();
        println!(
            "  {:?}: {}ms, {:.0} ns/call (acc={acc})",
            rule,
            start.elapsed().as_millis(),
            start.elapsed().as_nanos() as f64 / calls as f64,
        );
    }
}

#[test]
#[ignore = "performance probe; run with --ignored --release"]
fn perf_movegen_node() {
    let mut board = dense_midgame();
    let iters = 50_000usize;
    println!("movegen per interior node:");
    for rule in [RuleSet::Freestyle, RuleSet::Renju] {
        let mut config = load_default_config();
        config.rule_set = rule;
        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut caches, rule);
        let start = Instant::now();
        let mut acc = 0usize;
        for _ in 0..iters {
            let r = generate_candidates(&board, &caches, BLACK, &config, None, None, None, false);
            acc += r.candidates.len();
        }
        println!(
            "  {:?}: {}ms, {:.0} ns/node (acc={acc})",
            rule,
            start.elapsed().as_millis(),
            start.elapsed().as_nanos() as f64 / iters as f64,
        );
    }
}
