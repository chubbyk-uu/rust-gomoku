use std::collections::HashSet;

use rust_gomoku::{
    generate_candidates, load_default_config, recompute_all, xy_to_move, Board, EvalCaches,
    RuleSet, BLACK, WHITE,
};

fn double_three_board_black_to_move() -> Board {
    let mut board = Board::new();
    for (x, y, side) in [
        (6, 7, BLACK),
        (0, 0, WHITE),
        (8, 7, BLACK),
        (0, 1, WHITE),
        (7, 6, BLACK),
        (0, 2, WHITE),
        (7, 8, BLACK),
        (0, 3, WHITE),
    ] {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    assert_eq!(board.side_to_move(), BLACK);
    board
}

#[test]
fn rule_aware_legal_move_keeps_freestyle_default_and_filters_renju_black() {
    let board = double_three_board_black_to_move();
    let forbidden = xy_to_move(7, 7).unwrap();

    assert!(board.is_legal_move(forbidden));
    assert!(board.is_legal_move_for_rule(forbidden, BLACK, RuleSet::Freestyle));
    assert!(!board.is_legal_move_for_rule(forbidden, BLACK, RuleSet::Renju));
    assert!(board.is_legal_move_for_rule(forbidden, WHITE, RuleSet::Renju));
}

#[test]
fn movegen_filters_renju_black_forbidden_moves() {
    let mut board = double_three_board_black_to_move();
    let forbidden = xy_to_move(7, 7).unwrap();
    let allowed: HashSet<_> = [forbidden].into_iter().collect();
    let mut caches = EvalCaches::new();
    recompute_all(&mut board, &mut caches);

    let mut freestyle = load_default_config();
    freestyle.rule_set = RuleSet::Freestyle;
    let freestyle_result = generate_candidates(
        &board,
        &caches,
        BLACK,
        &freestyle,
        None,
        Some(&allowed),
        None,
        false,
    );
    assert!(
        freestyle_result
            .candidates
            .iter()
            .any(|candidate| candidate.move_ == forbidden),
        "freestyle should keep the root-allowed candidate"
    );

    let mut renju = load_default_config();
    renju.rule_set = RuleSet::Renju;
    let renju_result = generate_candidates(
        &board,
        &caches,
        BLACK,
        &renju,
        None,
        Some(&allowed),
        None,
        false,
    );
    assert!(
        renju_result
            .candidates
            .iter()
            .all(|candidate| candidate.move_ != forbidden),
        "renju black movegen must not emit forbidden moves"
    );
}
