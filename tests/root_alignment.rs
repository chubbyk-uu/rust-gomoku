use rust_gomoku::constants::{HASHF_ALPHA, INF};
use rust_gomoku::{
    fallback_ai_move, load_default_config, move_to_xy, new_classic_fallback_rng, recompute_all,
    xy_to_move, Board, EvalCaches, RootSearcher, SearchLimits, TTEntry,
};

#[test]
fn root_search_returns_center_on_empty_board() {
    let mut board = Board::new();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(&mut board, None);
    assert_eq!(result.move_, xy_to_move(7, 7).unwrap());
}

#[test]
fn root_search_finds_immediate_winning_completion() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 2,
            root_width: 10,
            ..SearchLimits::default()
        }),
    );
    assert!(matches!(move_to_xy(result.move_).unwrap(), (2, 7) | (7, 7)));
}

#[test]
fn root_search_returns_legal_move_under_node_limit() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 2,
            root_width: 8,
            node_limit: Some(10),
            ..SearchLimits::default()
        }),
    );
    assert!(board.is_legal_move(result.move_));
}

#[test]
fn root_search_prefers_vcf_first_when_available() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 2,
            root_width: 8,
            ..SearchLimits::default()
        }),
    );
    assert!(matches!(move_to_xy(result.move_).unwrap(), (2, 7) | (6, 7)));
    let trace = searcher.last_trace.as_ref().expect("trace is recorded");
    assert!(trace.vcf_found);
    assert!(!trace.used_vct);
    assert_eq!(trace.tactical_path, "vcf");
}

#[test]
fn root_vct_not_triggered_when_disabled() {
    let mut board = Board::new();
    let sequence = [
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
        (14, 14, -1),
    ];
    for (x, y, side) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut config = load_default_config();
    config.runtime.compute_vct = false;
    let mut searcher = RootSearcher::new(config);
    searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 3,
            root_width: 10,
            ..SearchLimits::default()
        }),
    );
    let trace = searcher.last_trace.as_ref().expect("trace is recorded");
    assert!(!trace.used_vct);
    assert!(!trace.vct_triggered);
}

#[test]
fn root_vct_not_triggered_on_quiet_position() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 3,
            root_width: 10,
            ..SearchLimits::default()
        }),
    );
    let trace = searcher.last_trace.as_ref().expect("trace is recorded");
    assert!(trace.used_vct);
    assert!(!trace.vct_triggered);
}

#[test]
fn root_vct_triggered_and_accepted_on_dual_a3_win() {
    let mut board = Board::new();
    let sequence = [
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
        (14, 14, -1),
    ];
    for (x, y, side) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 4,
            root_width: 20,
            ..SearchLimits::default()
        }),
    );
    let trace = searcher.last_trace.as_ref().expect("trace is recorded");
    assert!(trace.vct_triggered);
    assert!(trace.vct_found);
    assert!(trace.vct_accepted);
    assert_eq!(trace.tactical_path, "vct");
    assert_eq!(move_to_xy(result.move_).unwrap(), (7, 7));
    assert_eq!(result.score, INF);
}

#[test]
fn root_vct_triggered_but_finds_no_win() {
    let mut board = Board::new();
    let sequence = [
        (5, 7, 1),
        (4, 7, -1),
        (6, 7, 1),
        (0, 1, -1),
        (7, 7, 1),
        (1, 1, -1),
    ];
    for (x, y, side) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut searcher = RootSearcher::new(load_default_config());
    searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 4,
            root_width: 20,
            ..SearchLimits::default()
        }),
    );
    let trace = searcher.last_trace.as_ref().expect("trace is recorded");
    assert!(trace.vct_triggered);
    assert!(!trace.vct_found);
    assert!(!trace.vct_accepted);
    assert_eq!(trace.tactical_path, "alphabeta");
}

#[test]
fn root_vct_verification_rejects_opponent_counter() {
    let mut board = Board::new();
    let sequence = [
        (5, 7, 1),
        (10, 5, -1),
        (6, 7, 1),
        (11, 5, -1),
        (7, 7, 1),
        (12, 5, -1),
        (0, 1, 1),
        (13, 5, -1),
        (0, 2, 1),
        (4, 7, -1),
    ];
    for (x, y, side) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut searcher = RootSearcher::new(load_default_config());
    let (accepted, reason) =
        searcher.verify_root_vct_move(&board, board.side_to_move(), xy_to_move(2, 14).unwrap());
    assert!(!accepted);
    assert!(matches!(reason, Some("opponent_forcing" | "opponent_vcf")));
}

#[test]
fn root_search_matches_expected_one_move_reply() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 4,
            root_width: 8,
            ..SearchLimits::default()
        }),
    );
    assert_eq!(move_to_xy(result.move_).unwrap(), (7, 4));
    assert_eq!(result.score, -12);
}

#[test]
fn root_search_matches_classic_opening_10_4_depth6_width15() {
    let mut board = Board::new();
    board.play(xy_to_move(10, 4).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 6,
            root_width: 15,
            ..SearchLimits::default()
        }),
    );
    assert_eq!(move_to_xy(result.move_).unwrap(), (9, 4));
    assert_eq!(result.score, -10);
}

#[test]
fn classic_fallback_rng_matches_white_10_10_sequence() {
    let prefix_to_30 = [
        (10, 10, 1),
        (10, 9, -1),
        (9, 11, 1),
        (9, 10, -1),
        (11, 9, 1),
        (12, 8, -1),
        (8, 12, 1),
        (7, 13, -1),
        (8, 11, 1),
        (10, 11, -1),
        (11, 12, 1),
        (8, 9, -1),
        (10, 12, 1),
        (9, 12, -1),
        (8, 10, 1),
        (7, 9, -1),
        (8, 13, 1),
        (8, 14, -1),
        (9, 9, 1),
        (8, 8, -1),
        (11, 13, 1),
        (12, 14, -1),
        (11, 11, 1),
        (11, 10, -1),
        (12, 12, 1),
        (13, 13, -1),
        (13, 12, 1),
        (14, 12, -1),
        (12, 10, 1),
    ];
    let prefix_to_32 = [prefix_to_30.as_slice(), &[(13, 9, -1), (10, 8, 1)]].concat();

    let mut rng = new_classic_fallback_rng();

    let mut board = Board::new();
    for (x, y, side) in prefix_to_30 {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    assert_eq!(
        move_to_xy(fallback_ai_move(&board, &caches, board.side_to_move(), &mut rng).unwrap())
            .unwrap(),
        (13, 9)
    );

    let mut board = Board::new();
    for (x, y, side) in prefix_to_32 {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    assert_eq!(
        move_to_xy(fallback_ai_move(&board, &caches, board.side_to_move(), &mut rng).unwrap())
            .unwrap(),
        (9, 7)
    );
}

#[test]
fn root_allowed_moves_use_dynamic_board_margin_when_enabled() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    let mut config = load_default_config();
    config.runtime.static_board = false;
    config.runtime.dynamic_board_margin = 1;
    let searcher = RootSearcher::new(config);
    let allowed = searcher.root_allowed_moves(&board).unwrap();
    assert!(allowed.contains(&xy_to_move(6, 6).unwrap()));
    assert!(!allowed.contains(&xy_to_move(0, 0).unwrap()));
}

#[test]
fn root_allowed_moves_expand_to_square_window_like_current_engine() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 9).unwrap(), None).unwrap();
    board.play(xy_to_move(10, 9).unwrap(), None).unwrap();
    let mut config = load_default_config();
    config.runtime.static_board = false;
    config.runtime.dynamic_board_margin = 1;
    let searcher = RootSearcher::new(config);
    let allowed = searcher.root_allowed_moves(&board).unwrap();
    let xs: Vec<_> = allowed
        .iter()
        .map(|&move_| move_to_xy(move_).unwrap().0)
        .collect();
    let ys: Vec<_> = allowed
        .iter()
        .map(|&move_| move_to_xy(move_).unwrap().1)
        .collect();
    assert_eq!(
        xs.iter().max().unwrap() - xs.iter().min().unwrap(),
        ys.iter().max().unwrap() - ys.iter().min().unwrap()
    );
}

#[test]
fn root_search_reports_completed_depth_without_overshooting_limit() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 4).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 3,
            root_width: 8,
            ..SearchLimits::default()
        }),
    );
    assert_eq!(result.depth, 3);
}

#[test]
fn root_search_uses_classic_ais_fallback_when_root_move_is_missing() {
    let mut board = Board::new();
    let sequence = [
        (1, (7, 7)),
        (-1, (7, 6)),
        (1, (7, 5)),
        (-1, (6, 5)),
        (1, (8, 7)),
        (-1, (6, 7)),
        (1, (6, 6)),
        (-1, (5, 8)),
        (1, (8, 5)),
        (-1, (5, 4)),
        (1, (8, 6)),
        (-1, (4, 9)),
        (1, (3, 10)),
        (-1, (4, 3)),
        (1, (3, 2)),
    ];
    for (side, (x, y)) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    let mut searcher = RootSearcher::new(load_default_config());
    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 3,
            root_width: 10,
            ..SearchLimits::default()
        }),
    );
    assert!(board.is_legal_move(result.move_));
    assert_eq!(move_to_xy(result.move_).unwrap(), (8, 8));
    assert_eq!(result.score, -INF);
    assert_eq!(result.depth, 0);
}

#[test]
fn root_search_matches_expected_value_on_simple_tt_alpha_seed() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();

    let mut searcher = RootSearcher::new(load_default_config());
    searcher.tt.store(TTEntry {
        key: board.zobrist_key(),
        value: 123,
        flag: HASHF_ALPHA,
        depth: 4,
        priority: board.move_count() as i32 * 10 + 4,
        best_move: Some(xy_to_move(7, 8).unwrap()),
    });

    let result = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 4,
            root_width: 8,
            ..SearchLimits::default()
        }),
    );
    assert_eq!(move_to_xy(result.move_).unwrap(), (6, 6));
    assert_eq!(result.score, 13);
    assert_eq!(result.depth, 4);
}

#[test]
fn root_search_with_none_time_limit_matches_limit_free_search() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 4).unwrap(), None).unwrap();
    let mut searcher = RootSearcher::new(load_default_config());
    let without_time_limit = searcher.search(
        &mut board.clone(),
        Some(SearchLimits {
            max_depth: 3,
            root_width: 8,
            ..SearchLimits::default()
        }),
    );
    let explicit_none = searcher.search(
        &mut board,
        Some(SearchLimits {
            max_depth: 3,
            root_width: 8,
            time_limit_ms: None,
            ..SearchLimits::default()
        }),
    );
    assert_eq!(explicit_none.move_, without_time_limit.move_);
    assert_eq!(explicit_none.score, without_time_limit.score);
    assert_eq!(explicit_none.depth, without_time_limit.depth);
}
