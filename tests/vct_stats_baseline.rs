//! VCT stats equivalence harness (scaffolding for the threat-candidate rewrite).
//!
//! Runs `VCTSearcher` at a fixed depth on a set of dense positions and prints
//! the full node/attack/defense/memo counters, so a candidate optimization can
//! be proven to leave the search tree byte-identical (only faster).
//!
//! Cases are read from a JSONL file (one object per line):
//!   {"name": "...", "side": 1|-1, "moves": [[x,y], ...]}
//! `moves` are the played stones in order (ply 0 = black), `side` is the side to
//! move on top of that prefix.
//!
//! Run:
//! ```bash
//! VCT_STATS_CASES=/path/to/vct_cases.jsonl VCT_STATS_DEPTH=6 \
//!   cargo test --test vct_stats_baseline -- --ignored --nocapture
//! ```
use rust_gomoku::{xy_to_move, Board, Move, RuleSet, Side, ThreatBoardView, VCTSearcher};

/// The 16 `THREAT_DIRS` offsets, duplicated here as an independent reference so
/// the incremental-adjacency `threat_moves` can be checked against the original
/// full-board rescan semantics.
const REF_DIRS: [(isize, isize); 16] = [
    (-2, -2),
    (-1, -1),
    (2, 2),
    (1, 1),
    (-2, 2),
    (-1, 1),
    (2, -2),
    (1, -1),
    (2, 0),
    (1, 0),
    (0, 2),
    (0, 1),
    (-2, 0),
    (-1, 0),
    (0, -2),
    (0, -1),
];

/// Reference implementation of `threat_moves`: for every empty point, in the
/// same `for x { for y }` order, emit it (once) if any `REF_DIRS` neighbor holds
/// a `side` stone and the point is rule-legal for `side`.
fn reference_threat_moves(view: &ThreatBoardView, side: Side) -> Vec<Move> {
    let board = &view.board;
    let rule = view.rule_set();
    let size = 15usize;
    let mut out = Vec::new();
    for x in 0..size {
        for y in 0..size {
            if board.at(x, y).unwrap() != 0 {
                continue;
            }
            for (dx, dy) in REF_DIRS {
                let xx = x as isize + dx;
                let yy = y as isize + dy;
                if xx >= 0
                    && yy >= 0
                    && (xx as usize) < size
                    && (yy as usize) < size
                    && board.at(xx as usize, yy as usize).unwrap() == side
                {
                    let m = xy_to_move(x, y).unwrap();
                    if board.is_legal_move_for_rule(m, side, rule) {
                        out.push(m);
                    }
                    break;
                }
            }
        }
    }
    out
}

fn assert_threat_moves_match(view: &ThreatBoardView, tag: &str) {
    for side in [1i8, -1i8] {
        let got = view.threat_moves(side);
        let want = reference_threat_moves(view, side);
        assert_eq!(
            got, want,
            "threat_moves mismatch vs reference ({tag}, side {side})"
        );
    }
}

/// Random play/undo walk asserting the incremental adjacency stays exactly in
/// sync with the full-rescan reference after every mutation, under both rules.
#[test]
fn threat_moves_incremental_matches_full_rescan() {
    for rule in [RuleSet::Freestyle, RuleSet::Renju] {
        let mut state = 0x9e3779b97f4a7c15u64 ^ (rule as u64).wrapping_mul(0xd1b54a32d192ed03);
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut view = ThreatBoardView::from_board_with_rule(Board::new(), rule);
        let mut stack: Vec<Move> = Vec::new();
        assert_threat_moves_match(&view, "empty");

        for _ in 0..4000 {
            let play = stack.is_empty() || (stack.len() < 60 && rng() % 100 < 70);
            if play {
                let side: Side = if stack.len() % 2 == 0 { 1 } else { -1 };
                // Collect legal empty points for this side, pick one at random.
                let mut legal = Vec::new();
                for x in 0..15usize {
                    for y in 0..15usize {
                        if view.board.at(x, y).unwrap() == 0 {
                            let m = xy_to_move(x, y).unwrap();
                            if view.board.is_legal_move_for_rule(m, side, rule) {
                                legal.push(m);
                            }
                        }
                    }
                }
                if legal.is_empty() {
                    // No legal move (e.g. all forbidden); undo instead.
                    if stack.pop().is_some() {
                        view.undo();
                        assert_threat_moves_match(&view, "undo");
                    }
                    continue;
                }
                let m = legal[(rng() as usize) % legal.len()];
                view.play(m, side);
                stack.push(m);
                assert_threat_moves_match(&view, "play");
            } else {
                stack.pop();
                view.undo();
                assert_threat_moves_match(&view, "undo");
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    side: i8,
    moves: Vec<[usize; 2]>,
}

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
#[ignore = "VCT stats baseline; run with --ignored --nocapture and VCT_STATS_CASES set"]
fn vct_stats_baseline() {
    let path = std::env::var("VCT_STATS_CASES")
        .expect("set VCT_STATS_CASES to a JSONL path of {name,side,moves} cases");
    let depth: i32 = std::env::var("VCT_STATS_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let raw = std::fs::read_to_string(&path).expect("cases file readable");
    let cases: Vec<Case> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("case line parses"))
        .collect();

    let rule = RuleSet::Renju;
    println!("vct stats baseline (depth {depth}, rule {rule:?}, {} cases):", cases.len());
    println!(
        "  {:<12} {:>4} {:>5} {:>6} {:>9} {:>9} {:>9} {:>9} {:>8} {:>8} {:>7} {:>7}",
        "name", "mv", "found", "solved", "or", "and", "attacks", "defenses", "exact", "shfound",
        "maxA", "maxD"
    );
    let (mut t_or, mut t_and, mut t_atk, mut t_def) = (0usize, 0usize, 0usize, 0usize);
    for case in &cases {
        let board = build_board(&case.moves, rule);
        let mut searcher = VCTSearcher::default();
        let result = searcher.search_for_rule(&board, case.side, depth, rule);
        let s = &searcher.stats;
        println!(
            "  {:<12} {:>4} {:>5} {:>6} {:>9} {:>9} {:>9} {:>9} {:>8} {:>8} {:>7} {:>7}",
            case.name,
            result.move_.map(|m| m as i64).unwrap_or(-1),
            result.found as u8,
            result.solved as u8,
            s.or_nodes,
            s.and_nodes,
            s.attacks_generated,
            s.defenses_generated,
            s.memo_exact_hits,
            s.memo_shallow_found_hits,
            s.max_attack_count,
            s.max_defense_count,
        );
        t_or += s.or_nodes;
        t_and += s.and_nodes;
        t_atk += s.attacks_generated;
        t_def += s.defenses_generated;
    }
    println!("  TOTAL or={t_or} and={t_and} attacks={t_atk} defenses={t_def}");
}
