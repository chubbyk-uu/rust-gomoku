use std::collections::{HashMap, HashSet};

use rust_gomoku::constants::BOARD_AREA;
use rust_gomoku::{
    apply_hostile_three_extension, attack_level, covered_moves, generate_candidates,
    load_default_config, move_to_xy, move_value, movegen_backend_name, recompute_all, xy_to_move,
    Board, EvalCaches,
};

#[test]
fn movegen_backend_name_is_supported() {
    assert!(matches!(movegen_backend_name(), "python" | "cython"));
}

#[test]
fn covered_moves_uses_expected_template() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let moves = covered_moves(&board);
    assert_eq!(moves.len(), 32);
    assert!(moves.contains(&xy_to_move(7, 4).unwrap()));
    assert!(moves.contains(&xy_to_move(4, 4).unwrap()));
    assert!(!moves.contains(&xy_to_move(5, 4).unwrap()));
}

#[test]
fn covered_moves_returns_center_on_empty_board() {
    let board = Board::new();
    let moves = covered_moves(&board);
    assert_eq!(moves, vec![xy_to_move(7, 7).unwrap()]);
}

#[test]
fn generate_candidates_collapses_single_forcing_class() {
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
    let result = generate_candidates(
        &board,
        &caches,
        1,
        &load_default_config(),
        None,
        None,
        None,
        false,
    );
    assert_eq!(result.candidates.len(), 1);
    let (x, y) = move_to_xy(result.candidates[0].move_).unwrap();
    assert!(matches!((x, y), (2, 7) | (7, 7)));
}

#[test]
fn generate_candidates_hard_filters_root_allowed_moves() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let allowed = HashSet::from([xy_to_move(7, 8).unwrap()]);
    let result = generate_candidates(
        &board,
        &caches,
        1,
        &load_default_config(),
        Some(5),
        Some(&allowed),
        None,
        false,
    );
    assert!(!result.candidates.is_empty());
    assert_eq!(result.candidates[0].move_, xy_to_move(7, 8).unwrap());
    for c in &result.candidates {
        assert!(allowed.contains(&c.move_));
    }
}

#[test]
fn generate_candidates_injects_preferred_move_score() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let preferred = xy_to_move(7, 8).unwrap();
    let result = generate_candidates(
        &board,
        &caches,
        1,
        &load_default_config(),
        Some(8),
        None,
        Some(preferred),
        false,
    );
    assert!(!result.candidates.is_empty());
    assert!(result
        .candidates
        .iter()
        .any(|candidate| candidate.move_ == preferred && candidate.order_score == 100.0));
}

#[test]
fn hostile_three_extension_matches_expected_bonus_on_known_fallback_position() {
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

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let cfg = load_default_config();
    let side = board.side_to_move();
    let mut vbw_map = [0.0_f64; BOARD_AREA];
    let mut hsflag = None::<u16>;
    let mut sglflag = 0_i32;
    for move_ in covered_moves(&board) {
        let (x, y) = move_to_xy(move_).unwrap();
        let value = move_value(&caches, x, y, side, &cfg);
        let att1 = attack_level(&caches, x, y, side);
        let att2 = attack_level(&caches, x, y, -side);
        vbw_map[move_ as usize] = value;
        if value <= 0.0 {
            continue;
        }
        if att2 == 6 || att1 >= 5 {
            sglflag += 1;
        } else if att2 == 5 {
            hsflag = Some(move_);
        }
    }

    let before = vbw_map.clone();
    assert_eq!(sglflag, 0);
    assert_eq!(hsflag, Some(xy_to_move(8, 8).unwrap()));
    apply_hostile_three_extension(&board, hsflag.unwrap(), side, &mut vbw_map);
    let changed: HashMap<_, _> = vbw_map
        .iter()
        .enumerate()
        .filter_map(|(move_index, value)| {
            let old = before[move_index];
            if (*value - old).abs() > f64::EPSILON {
                Some((move_to_xy(move_index as u16).unwrap(), *value - old))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        changed,
        HashMap::from([((8, 4), 10_000.0), ((8, 8), 10_000.0)])
    );
}

#[test]
fn generate_candidates_matches_expected_casen_on_known_fallback_position() {
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

    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);
    let result = generate_candidates(
        &board,
        &caches,
        board.side_to_move(),
        &load_default_config(),
        Some(10),
        None,
        None,
        false,
    );
    assert_eq!(result.candidates.len(), 2);
    let coords: HashSet<_> = result
        .candidates
        .iter()
        .map(|candidate| move_to_xy(candidate.move_).unwrap())
        .collect();
    assert_eq!(coords, HashSet::from([(8, 4), (8, 8)]));
}
