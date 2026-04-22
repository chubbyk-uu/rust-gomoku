use rust_gomoku::{
    evaluate_board, evaluate_last5_branch, evaluate_next43_branch, find_last5_target,
    global_eval_backend_name, load_default_config, recompute_all, xy_to_move, Board, EvalCaches,
    BLACK,
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
        assert_eq!(
            evaluate_board(&mut board, &mut caches, 1, 0, &config),
            black_expected
        );
        assert_eq!(
            evaluate_board(&mut board, &mut caches, -1, 0, &config),
            white_expected
        );
    }
}
