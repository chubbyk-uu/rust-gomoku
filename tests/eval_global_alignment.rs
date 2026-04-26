use rust_gomoku::{
    evaluate_board, evaluate_board_main_cached, evaluate_board_main_scan, evaluate_last5_branch,
    evaluate_next43_branch, find_last5_target, global_eval_backend_name, load_default_config,
    recompute_all, value_wide_compute, xy_to_move, Board, EvalCaches, BLACK, WHITE,
};

#[test]
fn global_eval_backend_name_is_supported() {
    assert!(matches!(global_eval_backend_name(), "python" | "cython"));
}

#[test]
fn global_evaluation_prefers_side_with_immediate_five() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let score = evaluate_board(&mut board, &mut caches, BLACK, 0, &load_default_config());
    assert!(score > 0.0);
}

#[test]
fn find_last5_target_preserves_expected_scan_order() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let target = find_last5_target(&board, &caches, BLACK, &load_default_config());
    assert_eq!(target, Some((2, 7)));
}

#[test]
fn last5_branch_returns_positive_value_for_open_four() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let value = evaluate_last5_branch(&mut board, &mut caches, BLACK, 0, &load_default_config());
    assert!(value > 0.0);
}

#[test]
fn next43_branch_is_false_on_non_forcing_shape() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    assert!(!evaluate_next43_branch(
        &mut board,
        &mut caches,
        BLACK,
        &load_default_config()
    ));
}

#[test]
fn global_eval_matches_expected_on_handpicked_positions() {
    let config = load_default_config();
    let positions: &[(&[(usize, usize)], f64, f64)] = &[
        (
            &[
                (7, 7),
                (7, 6),
                (8, 7),
                (6, 6),
                (9, 7),
                (5, 5),
                (6, 7),
                (8, 6),
            ],
            15000.0,
            -15000.0,
        ),
        (
            &[(7, 7), (0, 0), (8, 7), (1, 0), (9, 7), (2, 0), (10, 7)],
            15000.0,
            -15000.0,
        ),
        (
            &[(7, 7), (6, 7), (8, 8), (7, 8), (10, 10), (8, 7)],
            -23.692430307108392_f64,
            -15000.0,
        ),
    ];

    for &(moves, black_expected, white_expected) in positions {
        let mut board = Board::new();
        for (idx, &(x, y)) in moves.iter().enumerate() {
            board
                .play(
                    xy_to_move(x, y).unwrap(),
                    Some(if idx % 2 == 0 { 1 } else { -1 }),
                )
                .unwrap();
        }
        let mut caches = EvalCaches::new();
        recompute_all(&mut board, &mut caches);
        assert_main_scan_and_cached_match(&board, &caches, BLACK);
        assert_main_scan_and_cached_match(&board, &caches, WHITE);
        let black_score = evaluate_board(&mut board, &mut caches, 1, 0, &config);
        let white_score = evaluate_board(&mut board, &mut caches, -1, 0, &config);
        assert_score_close(black_score, black_expected);
        assert_score_close(white_score, white_expected);
    }
}

#[test]
fn cached_global_eval_matches_scan_after_incremental_updates() {
    let mut board = Board::new();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let sequence = [
        (7, 7),
        (8, 7),
        (7, 8),
        (8, 8),
        (6, 7),
        (9, 8),
        (5, 7),
        (10, 8),
        (6, 8),
        (4, 7),
        (11, 8),
        (3, 7),
    ];

    for (ply, (x, y)) in sequence.into_iter().enumerate() {
        let move_ = xy_to_move(x, y).unwrap();
        board
            .play(move_, Some(if ply % 2 == 0 { BLACK } else { WHITE }))
            .unwrap();
        value_wide_compute(&mut board, &mut caches, (x, y));
        assert_main_scan_and_cached_match(&board, &caches, BLACK);
        assert_main_scan_and_cached_match(&board, &caches, WHITE);
    }
}

fn assert_main_scan_and_cached_match(board: &Board, caches: &EvalCaches, side: i8) {
    let config = load_default_config();
    let (scan_total, scan_dgn) = evaluate_board_main_scan(board, caches, side, &config);
    let (cached_total, cached_dgn) = evaluate_board_main_cached(board, caches, side, &config);
    assert_eq!(cached_dgn, scan_dgn);
    let tolerance = 1e-9_f64.max(scan_total.abs() * 1e-14);
    assert!(
        (cached_total - scan_total).abs() <= tolerance,
        "cached total {cached_total} differs from scan total {scan_total}"
    );
    assert_eq!(cached_total as i32, scan_total as i32);
    if (-32_768.0 < scan_total && scan_total < 32_768.0)
        && (-32_768.0 < cached_total && cached_total < 32_768.0)
    {
        let scan_score = scan_total - config.search.drift + f64::from(scan_dgn) * config.search.dgn;
        let cached_score =
            cached_total - config.search.drift + f64::from(cached_dgn) * config.search.dgn;
        assert_eq!(cached_score as i32, scan_score as i32);
    }
}

fn assert_score_close(actual: f64, expected: f64) {
    let tolerance = 1e-9_f64.max(expected.abs() * 1e-14);
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual score {actual} differs from expected {expected}"
    );
    assert_eq!(actual as i32, expected as i32);
}
