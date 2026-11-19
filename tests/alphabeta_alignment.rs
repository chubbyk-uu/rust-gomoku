use std::collections::HashSet;
use std::time::{Duration, Instant};

use rust_gomoku::constants::{BLACK, HASHF_EXACT, INF, WHITE, WIN};
use rust_gomoku::{
    classify_forbidden_move, compute_corner_state, load_default_config, move_to_xy, recompute_all,
    recompute_all_for_rule, rootbonus, terminal_score, xy_to_move, AlphaBetaSearcher, Board,
    EvalCaches, ForbiddenKind, RuleSet, SearchOptions, SearchStats, TTEntry,
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
        (7, 6)
    );
    assert_eq!(score, -19990);
}

#[test]
fn fractional_depth_expands_after_a_cached_leaf() {
    let mut board = Board::new();
    for (x, y) in [(7, 7), (8, 6), (7, 5), (7, 4), (6, 6), (1, 1)] {
        board.play(xy_to_move(x, y).unwrap(), None).unwrap();
    }
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let mut search = |engine: &mut AlphaBetaSearcher, depth| {
        let side = board.side_to_move();
        engine.search(
            &mut board,
            &mut caches,
            side,
            depth,
            -INF,
            INF,
            20,
            &mut SearchStats::default(),
            SearchOptions::default(),
        )
    };
    let mut engine = AlphaBetaSearcher::new(load_default_config());
    let cold = search(&mut engine, 0.8);
    let mut engine = AlphaBetaSearcher::new(load_default_config());
    search(&mut engine, 0.0);
    assert!(cold.1.is_some());
    assert_eq!(search(&mut engine, 0.8), cold);
}

#[test]
fn last_free_point_draw_scores_zero() {
    let mut board = Board::new();
    let (black, white): (Vec<u16>, Vec<u16>) =
        (0..225).partition(|m| ((m % 15 + 2 * (m / 15)) % 4) < 2);
    for i in 0..224 {
        let stones = if i % 2 == 0 { &black } else { &white };
        board.play(stones[i / 2], None).unwrap();
    }
    let result = rust_gomoku::RootSearcher::new(load_default_config()).search(&mut board, None);
    board.play(result.move_, None).unwrap();
    assert!(board.is_draw());
    assert_eq!(terminal_score(&board, board.side_to_move(), 1), Some(0));
    assert_eq!(result.score, 0);
}

#[test]
fn restricted_root_and_depth0_tt_hint_do_not_claim_fake_wins() {
    let mut board = Board::new();
    for (x, y) in [
        (5, 7),
        (0, 0),
        (6, 7),
        (1, 0),
        (7, 7),
        (2, 0),
        (8, 7),
        (9, 7),
    ] {
        board.play(xy_to_move(x, y).unwrap(), None).unwrap();
    }
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let winning = xy_to_move(4, 7).unwrap();
    let quiet_a = xy_to_move(5, 8).unwrap();
    let quiet_b = xy_to_move(6, 8).unwrap();

    // Depth-0 TT hint on a quiet move must not override the immediate five at (4, 7).
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    searcher.tt.store(TTEntry {
        key: board.zobrist_key(),
        value: 0,
        flag: HASHF_EXACT,
        depth: 0,
        priority: 80,
        best_move: Some(quiet_b),
        generation: 0,
    });
    let mut stats = SearchStats::default();
    let (score, best) = searcher.search(
        &mut board,
        &mut caches,
        BLACK,
        1.0,
        -INF,
        INF,
        20,
        &mut stats,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );
    assert_eq!(best, Some(winning));
    assert_eq!(score, INF);

    // Excluding (4, 7) via root_allowed_moves must not return a fake +INF for a quiet move.
    let allowed = HashSet::from([quiet_a, quiet_b]);
    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let mut stats = SearchStats::default();
    let (score, best) = searcher.search(
        &mut board,
        &mut caches,
        BLACK,
        1.0,
        -INF,
        INF,
        20,
        &mut stats,
        SearchOptions {
            opo: 1,
            root: true,
            root_allowed_moves: Some(&allowed),
            ..SearchOptions::default()
        },
    );
    assert!(best.is_some_and(|m| allowed.contains(&m)));
    assert!(score < WIN, "quiet move must not claim fake win: {score}");
    assert!(
        stats.nodes > 1,
        "restricted root must actually search candidates"
    );
}

#[test]
fn restricted_root_ignores_unfiltered_tt_cutoff_and_does_not_pollute_tt() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let outside = xy_to_move(6, 6).unwrap();
    let allowed_a = xy_to_move(7, 8).unwrap();
    let allowed_b = xy_to_move(8, 8).unwrap();
    let allowed = HashSet::from([allowed_a, allowed_b]);

    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    searcher.tt.store(TTEntry {
        key: board.zobrist_key(),
        value: 1234,
        flag: HASHF_EXACT,
        depth: 6,
        priority: 100,
        best_move: Some(outside),
        generation: 0,
    });

    let mut stats = SearchStats::default();
    let (score, best) = searcher.search(
        &mut board,
        &mut caches,
        BLACK,
        2.0,
        -INF,
        INF,
        20,
        &mut stats,
        SearchOptions {
            opo: 1,
            root: true,
            root_allowed_moves: Some(&allowed),
            ..SearchOptions::default()
        },
    );
    assert!(best.is_some_and(|m| allowed.contains(&m)));
    assert_ne!(score, 1234);
    assert!(stats.nodes > 1);

    // Restricted root search must not overwrite the unrestricted TT entry.
    let probe = searcher.tt.probe(board.zobrist_key(), 6, -INF, INF);
    assert!(probe.hit);
    assert_eq!(probe.value, Some(1234));
    assert_eq!(probe.best_move, Some(outside));
}

#[test]
fn root_search_returns_best_candidate_even_when_atdown_scores_below_neg_inf() {
    // White has an immediate five-in-a-row threat at (7, 7) and (2, 7) (open four on row 7).
    // Two restricted black moves near the edge both lose immediately with different rootbonus
    // penalties, pushing both scores strictly below -INF.
    let mut board = Board::new();
    for (x, y) in [
        (0, 0),
        (3, 7),
        (1, 0),
        (4, 7),
        (2, 0),
        (5, 7),
        (14, 14),
        (6, 7),
    ] {
        board.play(xy_to_move(x, y).unwrap(), None).unwrap();
    }
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let edge_move = xy_to_move(1, 1).unwrap(); // higher rootbonus penalty
    let inner_move = xy_to_move(3, 3).unwrap(); // lower rootbonus penalty
    let allowed = HashSet::from([edge_move, inner_move]);

    let mut searcher = AlphaBetaSearcher::new(load_default_config());
    let mut stats = SearchStats::default();
    let (score, best) = searcher.search(
        &mut board,
        &mut caches,
        BLACK,
        1.0,
        -INF,
        INF,
        20,
        &mut stats,
        SearchOptions {
            opo: 1,
            root: true,
            root_allowed_moves: Some(&allowed),
            ..SearchOptions::default()
        },
    );
    assert_eq!(best, Some(inner_move));
    assert!(score < -INF);
}

#[test]
fn last_free_point_forbidden_in_renju_scores_as_loss_at_leaf_and_search_depths() {
    // Fill 224 cells with a draw-safe pattern, leaving only (5, 7) empty where
    // placing Black creates a 6-in-a-row overline (cols 2..=7 on row 7).
    let mut board = Board::new();
    let target = xy_to_move(5, 7).unwrap();
    let overline_neighbors = [
        xy_to_move(2, 7).unwrap(),
        xy_to_move(3, 7).unwrap(),
        xy_to_move(4, 7).unwrap(),
        xy_to_move(6, 7).unwrap(),
        xy_to_move(7, 7).unwrap(),
    ];
    let guards = [xy_to_move(1, 7).unwrap(), xy_to_move(8, 7).unwrap()];

    // Start from the safe 2x2 striped fill, then place the 5 black stones of the
    // overline and 2 white guards at the ends, balancing to 112 black + 112 white.
    let mut grid = [[0_i8; 15]; 15];
    for m in 0..225_u16 {
        if m == target {
            continue;
        }
        let (x, y) = move_to_xy(m).unwrap();
        grid[y][x] = if ((x + 2 * y) % 4) < 2 { BLACK } else { WHITE };
    }
    for m in overline_neighbors {
        let (x, y) = move_to_xy(m).unwrap();
        grid[y][x] = BLACK;
    }
    for m in guards {
        let (x, y) = move_to_xy(m).unwrap();
        grid[y][x] = WHITE;
    }
    // Rebalance black/white counts away from row 7 so we can replay 224 alternating moves.
    let mut blacks = Vec::new();
    let mut whites = Vec::new();
    for m in 0..225_u16 {
        if m == target {
            continue;
        }
        let (x, y) = move_to_xy(m).unwrap();
        if grid[y][x] == BLACK {
            blacks.push(m);
        } else {
            whites.push(m);
        }
    }
    while blacks.len() > 112 {
        let swap = blacks
            .iter()
            .position(|&m| {
                let (x, y) = move_to_xy(m).unwrap();
                y != 7 && y != 6 && y != 8 && x < 14
            })
            .unwrap();
        let m = blacks.remove(swap);
        whites.push(m);
    }
    while whites.len() > 112 {
        let swap = whites
            .iter()
            .position(|&m| {
                let (x, y) = move_to_xy(m).unwrap();
                y != 7 && y != 6 && y != 8 && x < 14
            })
            .unwrap();
        let m = whites.remove(swap);
        blacks.push(m);
    }
    // Replay on board using alternating sides via freestyle setup, verifying no winner.
    for i in 0..224 {
        let m = if i % 2 == 0 {
            blacks[i / 2]
        } else {
            whites[i / 2]
        };
        board.play(m, None).unwrap();
    }
    assert_eq!(board.move_count(), 224);
    assert_eq!(board.winner(), 0);
    assert_eq!(board.side_to_move(), BLACK);
    assert_eq!(
        classify_forbidden_move(&board, target, BLACK, RuleSet::Renju).unwrap(),
        ForbiddenKind::Overline
    );

    let mut config = load_default_config();
    config.rule_set = RuleSet::Renju;
    let mut caches = EvalCaches::new();
    recompute_all_for_rule(&mut board, &mut caches, RuleSet::Renju);

    for depth in [0.0, 1.0] {
        let mut searcher = AlphaBetaSearcher::new(config.clone());
        let mut stats = SearchStats::default();
        let (score, best) = searcher.search(
            &mut board,
            &mut caches,
            BLACK,
            depth,
            -INF,
            INF,
            20,
            &mut stats,
            SearchOptions {
                ply: 3,
                ..SearchOptions::default()
            },
        );
        assert_eq!(
            best, None,
            "depth={depth}: forbidden last move must not be played"
        );
        assert_eq!(
            score,
            -INF + 3,
            "depth={depth}: no legal move must score as loss"
        );
    }
}
