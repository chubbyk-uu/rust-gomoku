use std::time::{Duration, Instant};

use rust_gomoku::constants::INF;
use rust_gomoku::{
    compute_corner_state, load_default_config, move_to_xy, recompute_all, rootbonus,
    terminal_score, xy_to_move, AlphaBetaSearcher, Board, EvalCaches, SearchOptions, SearchStats,
};

const MID_LADDER: &[(usize, usize, i8)] = &[
    (7, 7, 1),
    (8, 8, -1),
    (6, 6, 1),
    (9, 9, -1),
    (5, 5, 1),
    (10, 10, -1),
    (7, 8, 1),
    (8, 7, -1),
    (6, 9, 1),
    (9, 6, -1),
];

#[test]
fn rootbonus_prefers_low_height_moves() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 8).unwrap(), None).unwrap();
    assert!(rootbonus(&board, 1, 1, true) > rootbonus(&board, 7, 7, false));
}

#[test]
fn rootbonus_corner_mode_still_rewards_near_edge_moves() {
    let mut board = Board::new();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(14, 14).unwrap(), None).unwrap();
    assert!(rootbonus(&board, 1, 1, true) > 0);
}

#[test]
fn compute_corner_state_matches_expected_half_corner_cases() {
    let mut board = Board::new();
    assert_eq!(compute_corner_state(&board), (false, 0));
    board.play(xy_to_move(2, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 2).unwrap(), None).unwrap();
    assert_eq!(compute_corner_state(&board), (true, 2));
}

#[test]
fn terminal_score_uses_side_relative_inf_values() {
    let mut board = Board::new();
    let sequence = [
        (3, 7, 1),
        (0, 0, -1),
        (4, 7, 1),
        (1, 0, -1),
        (5, 7, 1),
        (2, 0, -1),
        (6, 7, 1),
        (3, 0, -1),
        (7, 7, 1),
    ];
    for (x, y, side) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    assert_eq!(terminal_score(&board, 1, 2), Some(INF - 2));
    assert_eq!(terminal_score(&board, -1, 2), Some(-INF + 2));
}

#[test]
fn alphabeta_returns_zero_when_deadline_expired_at_entry() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let side = board.side_to_move();
    let mut stats = SearchStats {
        deadline: Some(Instant::now() - Duration::from_secs(1)),
        ..SearchStats::default()
    };
    let (score, move_) = searcher.search(
        &mut board,
        &mut caches,
        side,
        3.0,
        -INF,
        INF,
        8,
        &mut stats,
        SearchOptions::default(),
    );
    assert!(stats.stop);
    assert_eq!((score, move_), (0, None));
}

#[test]
fn alphabeta_returns_zero_when_deadline_expires_on_periodic_check() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let side = board.side_to_move();
    let mut stats = SearchStats {
        nodes: 255,
        deadline: Some(Instant::now() - Duration::from_secs(1)),
        ..SearchStats::default()
    };
    let (score, move_) = searcher.search(
        &mut board,
        &mut caches,
        side,
        3.0,
        -INF,
        INF,
        8,
        &mut stats,
        SearchOptions::default(),
    );
    assert!(stats.stop);
    assert_eq!((score, move_), (0, None));
}

#[test]
fn alphabeta_returns_zero_when_node_limit_stops_at_entry() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let side = board.side_to_move();
    let mut stats = SearchStats {
        node_limit: Some(0),
        ..SearchStats::default()
    };
    let (score, move_) = searcher.search(
        &mut board,
        &mut caches,
        side,
        3.0,
        -INF,
        INF,
        8,
        &mut stats,
        SearchOptions::default(),
    );
    assert_eq!((score, move_), (0, None));
}

#[test]
fn alphabeta_leaf_matches_expected_sign_convention_on_simple_child_board() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 8).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let side = board.side_to_move();
    let mut stats = SearchStats::default();
    let (score, move_) = searcher.search(
        &mut board,
        &mut caches,
        side,
        0.0,
        -INF,
        INF,
        8,
        &mut stats,
        SearchOptions::default(),
    );
    assert_eq!(move_, None);
    assert_eq!(score, -1);
}

#[test]
fn alphabeta_finds_immediate_tactical_win_without_vcf_branch() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let side = board.side_to_move();
    let mut stats = SearchStats::default();
    let (score, move_) = searcher.search(
        &mut board,
        &mut caches,
        side,
        2.0,
        -20_000,
        20_000,
        8,
        &mut stats,
        SearchOptions::default(),
    );
    let (x, y) = move_to_xy(move_.expect("winning move should exist")).unwrap();
    assert!(matches!((x, y), (2, 7) | (6, 7)));
    assert!(score >= 15_000);
}

#[test]
fn alphabeta_matches_expected_mid_ladder_nonroot_score() {
    let mut board = Board::new();
    for &(x, y, side) in MID_LADDER {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    board.play(xy_to_move(7, 5).unwrap(), Some(1)).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let side = board.side_to_move();
    let mut stats = SearchStats::default();
    let (score, move_) = searcher.search(
        &mut board,
        &mut caches,
        side,
        3.0,
        -20_002,
        20_002,
        8,
        &mut stats,
        SearchOptions {
            opo: 1,
            ply: 1,
            downf: 1,
            ..SearchOptions::default()
        },
    );
    assert_eq!(
        move_to_xy(move_.expect("move should exist")).unwrap(),
        (12, 12)
    );
    assert_eq!(score, -43);
}
