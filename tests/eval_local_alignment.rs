use rust_gomoku::{
    attack_level, compute_bucket_and_attack, compute_direction_shape, load_default_config,
    local_backend_name, move_to_xy, move_value, recompute_all, recompute_point_caches,
    value_wide_compute, xy_to_move, Board, EvalCaches, ShapeLabel, BLACK,
};

#[test]
fn local_backend_name_is_supported() {
    assert!(matches!(local_backend_name(), "python" | "cython"));
}

#[test]
fn recompute_point_caches_finds_black_five_threat() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();

    let mut caches = EvalCaches::new();
    recompute_point_caches(&mut board, &mut caches, 7, 7);
    assert_eq!(attack_level(&caches, 7, 7, BLACK), 6);
    assert!(caches.value_cache[0][7][7] > 0);
}

#[test]
fn recompute_all_populates_board_shadow_from_board() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    assert_eq!(caches.board_shadow[7][7], 1);
    assert_eq!(caches.board_shadow[8][7], -1);
}

#[test]
fn incremental_value_wide_matches_full_recompute() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 8).unwrap(), None).unwrap();

    let mut incremental = EvalCaches::new();
    let mut full = EvalCaches::new();
    value_wide_compute(&mut board, &mut incremental, (8, 8));
    recompute_all(&mut board, &mut full);

    assert_eq!(incremental.board_shadow, full.board_shadow);
    assert_eq!(incremental.shape_cache, full.shape_cache);
    assert_eq!(incremental.value_cache, full.value_cache);
    assert_eq!(incremental.attack_cache, full.attack_cache);
}

#[test]
fn value_wide_compute_roundtrip_after_play_and_undo() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let snapshot = caches.snapshot();

    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (7, 8));
    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));

    let mut expected = EvalCaches::new();
    recompute_all(&mut board, &mut expected);
    assert_eq!(caches.board_shadow, expected.board_shadow);
    assert_eq!(caches.shape_cache, expected.shape_cache);
    assert_eq!(caches.value_cache, expected.value_cache);
    assert_eq!(caches.attack_cache, expected.attack_cache);
    caches.restore_snapshot(&snapshot);
}

#[test]
fn nested_snapshots_restore_shape_cache_in_lifo_order() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let outer = caches.snapshot();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (7, 8));
    let after_outer = caches.copy();

    let inner = caches.snapshot();
    board.play(xy_to_move(8, 8).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (8, 8));

    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));
    caches.restore_snapshot(&inner);
    assert_eq!(caches.board_shadow, after_outer.board_shadow);
    assert_eq!(caches.shape_cache, after_outer.shape_cache);
    assert_eq!(caches.value_cache, after_outer.value_cache);
    assert_eq!(caches.attack_cache, after_outer.attack_cache);

    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));
    caches.restore_snapshot(&outer);

    let mut full = EvalCaches::new();
    recompute_all(&mut board, &mut full);
    assert_eq!(caches.board_shadow, full.board_shadow);
    assert_eq!(caches.shape_cache, full.shape_cache);
    assert_eq!(caches.value_cache, full.value_cache);
    assert_eq!(caches.attack_cache, full.attack_cache);
}

#[test]
fn value_wide_compute_matches_full_recompute_after_multiple_steps() {
    let mut board = Board::new();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let sequence = [
        xy_to_move(7, 7).unwrap(),
        xy_to_move(8, 7).unwrap(),
        xy_to_move(7, 8).unwrap(),
        xy_to_move(8, 8).unwrap(),
        xy_to_move(6, 7).unwrap(),
    ];
    for move_ in sequence {
        board.play(move_, None).unwrap();
        let (mx, my) = move_to_xy(move_).unwrap();
        value_wide_compute(&mut board, &mut caches, (mx, my));
        let mut full = EvalCaches::new();
        recompute_all(&mut board, &mut full);
        assert_eq!(caches.board_shadow, full.board_shadow);
        assert_eq!(caches.shape_cache, full.shape_cache);
        assert_eq!(caches.value_cache, full.value_cache);
        assert_eq!(caches.attack_cache, full.attack_cache);
    }
}

#[test]
fn move_value_uses_attack_and_defend_tables() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();

    let mut caches = EvalCaches::new();
    recompute_point_caches(&mut board, &mut caches, 7, 7);
    let config = load_default_config();
    let expected = config.eval_tables.attack_value[caches.value_cache[0][7][7] as usize]
        + config.eval_tables.defend_value[caches.value_cache[1][7][7] as usize];
    assert_eq!(move_value(&caches, 7, 7, BLACK, &config), expected);
}

#[test]
fn shape_cache_contains_valid_labels_for_empty_point() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_point_caches(&mut board, &mut caches, 7, 8);
    for direction in 0..4 {
        let label = (caches.shape_cache[0][7][8][direction] >> 16) & 0xF;
        assert!((ShapeLabel::L0 as i32..=ShapeLabel::L6 as i32).contains(&label));
    }
}

#[test]
fn compute_direction_shape_matches_cached_horizontal_shape() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_point_caches(&mut board, &mut caches, 8, 8);

    let shape = compute_direction_shape(&mut board, 8, 8, 0, BLACK);
    assert_eq!(shape, caches.shape_cache[0][8][8][0]);
}

#[test]
fn compute_bucket_and_attack_returns_expected_attack_for_five() {
    let shapes = (786_433, 131_073, 131_073, 131_073);
    let (_, attack) = compute_bucket_and_attack(shapes);
    assert_eq!(attack, 6);
}

#[test]
fn value_log_empty_on_fresh_caches() {
    let caches = EvalCaches::new();
    assert_eq!(caches.value_log_len(), 0);
}

#[test]
fn value_log_not_written_without_active_snapshot() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (8, 7));
    assert_eq!(caches.value_log_len(), 0);
}

#[test]
fn value_log_grows_after_snapshot_and_play() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let snapshot = caches.snapshot();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (8, 7));
    assert!(caches.value_log_len() > 0);
    board.undo().unwrap();
    caches.restore_snapshot(&snapshot);
}

#[test]
fn snapshot_value_log_len_field_is_recorded() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let snap1 = caches.snapshot();
    assert_eq!(snap1.value_log_len, 0);

    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (8, 7));
    let snap2 = caches.snapshot();
    assert!(snap2.value_log_len > 0);

    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));
    caches.restore_snapshot(&snap2);
    board.undo().unwrap();
    caches.restore_snapshot(&snap1);
}

#[test]
fn restore_snapshot_reverts_value_and_attack_caches() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let before_value = caches.value_cache[0].clone();
    let before_attack = caches.attack_cache[0].clone();

    let snap = caches.snapshot();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (7, 8));

    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));
    caches.restore_snapshot(&snap);

    assert_eq!(caches.value_cache[0], before_value);
    assert_eq!(caches.attack_cache[0], before_attack);
    assert_eq!(caches.value_log_len(), 0);
}

#[test]
fn double_snapshot_restores_independently() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let initial_value = caches.value_cache[0].clone();
    let initial_attack = caches.attack_cache[0].clone();

    let outer = caches.snapshot();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (8, 7));
    let state_after_outer = caches.value_cache[0].clone();

    let inner = caches.snapshot();
    board.play(xy_to_move(9, 7).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (9, 7));

    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));
    caches.restore_snapshot(&inner);
    assert_eq!(caches.value_cache[0], state_after_outer);

    let undone = board.undo().unwrap();
    let (ux, uy) = move_to_xy(undone.move_).unwrap();
    value_wide_compute(&mut board, &mut caches, (ux, uy));
    caches.restore_snapshot(&outer);
    assert_eq!(caches.value_cache[0], initial_value);
    assert_eq!(caches.attack_cache[0], initial_attack);
    assert_eq!(caches.value_log_len(), 0);
}

#[test]
fn value_log_cleared_on_reset() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let snap = caches.snapshot();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    value_wide_compute(&mut board, &mut caches, (8, 7));
    assert!(caches.value_log_len() > 0);
    caches.restore_snapshot(&snap);
    caches.reset();
    assert_eq!(caches.value_log_len(), 0);
}

#[test]
fn recompute_all_does_not_write_value_log() {
    let mut board = Board::new();
    for move_ in [
        xy_to_move(7, 7).unwrap(),
        xy_to_move(8, 7).unwrap(),
        xy_to_move(7, 8).unwrap(),
        xy_to_move(8, 8).unwrap(),
    ] {
        board.play(move_, None).unwrap();
    }
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    assert_eq!(caches.value_log_len(), 0);
}

#[test]
fn value_wide_matches_expected_on_handpicked_points() {
    let mut board = Board::new();
    let moves = [
        (7, 7),
        (7, 6),
        (8, 7),
        (6, 6),
        (9, 7),
        (5, 5),
        (6, 7),
        (8, 6),
    ];
    for (idx, (x, y)) in moves.into_iter().enumerate() {
        board
            .play(
                xy_to_move(x, y).unwrap(),
                Some(if idx % 2 == 0 { 1 } else { -1 }),
            )
            .unwrap();
    }

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let expected = [
        (
            (6, 8),
            (
                10,
                0,
                6,
                0,
                [196609, 131073, 196609, 131073],
                [65537, 131073, 65537, 131073],
            ),
        ),
        (
            (8, 8),
            (
                25,
                0,
                5,
                0,
                [196609, 131073, 393217, 196609],
                [65537, 131073, 65537, 65537],
            ),
        ),
        (
            (5, 7),
            (
                81,
                6,
                27,
                0,
                [131073, 786433, 65537, 131073],
                [327681, 65537, 393217, 131073],
            ),
        ),
        (
            (10, 7),
            (
                81,
                6,
                6,
                0,
                [131073, 786433, 131073, 131073],
                [131073, 65537, 131073, 131073],
            ),
        ),
        (
            (4, 4),
            (
                6,
                0,
                31,
                0,
                [131073, 131073, 131073, 65537],
                [131073, 131073, 131073, 458753],
            ),
        ),
    ];

    for (
        (x, y),
        (bucket_black, attack_black, bucket_white, attack_white, shape_black, shape_white),
    ) in expected
    {
        assert_eq!(caches.value_cache[0][x][y], bucket_black);
        assert_eq!(caches.attack_cache[0][x][y], attack_black);
        assert_eq!(caches.value_cache[1][x][y], bucket_white);
        assert_eq!(caches.attack_cache[1][x][y], attack_white);
        assert_eq!(caches.shape_cache[0][x][y], shape_black);
        assert_eq!(caches.shape_cache[1][x][y], shape_white);
    }
}

#[test]
fn value_wide_incremental_snapshots_match_expected_sequence() {
    let mut board = Board::new();
    let mut caches = EvalCaches::new();
    let sequence = [
        (7, 7),
        (7, 6),
        (8, 7),
        (6, 6),
        (9, 7),
        (5, 5),
        (6, 7),
        (8, 6),
    ];
    let expected = [
        (
            1,
            [
                ((6, 8), (24, 0, 6, 0)),
                ((8, 8), (24, 0, 6, 0)),
                ((5, 7), (18, 0, 6, 0)),
            ],
        ),
        (
            2,
            [
                ((6, 8), (24, 0, 6, 0)),
                ((8, 8), (24, 0, 6, 0)),
                ((5, 7), (18, 0, 6, 0)),
            ],
        ),
        (
            3,
            [
                ((6, 8), (24, 0, 6, 0)),
                ((8, 8), (28, 0, 6, 0)),
                ((5, 7), (39, 3, 6, 0)),
            ],
        ),
        (
            4,
            [
                ((6, 8), (24, 0, 18, 0)),
                ((8, 8), (25, 0, 6, 0)),
                ((5, 7), (39, 3, 24, 0)),
            ],
        ),
        (
            5,
            [
                ((6, 8), (24, 0, 18, 0)),
                ((8, 8), (28, 0, 5, 0)),
                ((5, 7), (58, 4, 24, 0)),
            ],
        ),
        (
            6,
            [
                ((6, 8), (24, 0, 18, 0)),
                ((8, 8), (28, 0, 5, 0)),
                ((5, 7), (58, 4, 27, 0)),
            ],
        ),
        (
            7,
            [
                ((6, 8), (25, 0, 6, 0)),
                ((8, 8), (28, 0, 5, 0)),
                ((5, 7), (81, 6, 27, 0)),
            ],
        ),
        (
            8,
            [
                ((6, 8), (10, 0, 6, 0)),
                ((8, 8), (25, 0, 5, 0)),
                ((5, 7), (81, 6, 27, 0)),
            ],
        ),
    ];

    for ((ply, points), (x, y)) in expected.into_iter().zip(sequence.into_iter()) {
        board
            .play(
                xy_to_move(x, y).unwrap(),
                Some(if ply % 2 == 1 { 1 } else { -1 }),
            )
            .unwrap();
        value_wide_compute(&mut board, &mut caches, (x, y));
        for ((px, py), values) in points {
            assert_eq!(
                (
                    caches.value_cache[0][px][py],
                    caches.attack_cache[0][px][py],
                    caches.value_cache[1][px][py],
                    caches.attack_cache[1][px][py],
                ),
                values
            );
        }
    }
}
