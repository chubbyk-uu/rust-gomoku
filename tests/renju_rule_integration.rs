use std::collections::HashSet;

use rust_gomoku::{
    generate_candidates, load_default_config, recompute_all_for_rule, xy_to_move, Board,
    EvalCaches, GameController, GameMode, GomocupProtocol, RuleSet, SearchClockMode, SearchLimits,
    BLACK, WHITE,
};

#[test]
fn forced_black_forbidden_loss_ends_gui_game_and_returns_classical_protocol_move() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../cases/renju/no_legal_move_after_224.json")).unwrap();
    let moves: Vec<[usize; 2]> = serde_json::from_value(fixture["moves"].clone()).unwrap();
    let config = load_default_config();
    let mut gui = GameController::new(config.clone());
    gui.new_game_with_clock(
        BLACK,
        RuleSet::Renju,
        GameMode::TwoPlayer,
        SearchClockMode::Untimed,
    );
    let limits = SearchLimits {
        max_depth: 2,
        root_width: 8,
        ..SearchLimits::default()
    };
    let mut protocol = GomocupProtocol::new(Some(config), Some(limits));
    protocol.handle_line("INFO rule 4");
    protocol.handle_line("BOARD");

    for (i, [x, y]) in moves.iter().copied().enumerate() {
        gui.play_human(x, y);
        let side = if i % 2 == 0 { 1 } else { 2 };
        protocol.handle_line(&format!("{x},{y},{side}"));
    }

    let state = gui.snapshot();
    assert_eq!(state.move_count, 224);
    assert_eq!(state.winner, WHITE);
    assert!(!state.can_play);
    assert!(!state.draw);
    gui.undo_turn();
    assert_eq!(gui.snapshot().move_count, 223);
    assert_eq!(gui.snapshot().winner, 0);
    assert!(gui.snapshot().can_play);

    assert_eq!(protocol.handle_line("DONE").last().unwrap(), "9,12");
    assert_eq!(protocol.board.winner(), WHITE);
    assert_eq!(protocol.handle_line("TAKEBACK 9,12"), vec!["OK"]);
    assert_eq!(protocol.board.winner(), 0);
}

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
    let mut freestyle = load_default_config();
    freestyle.rule_set = RuleSet::Freestyle;
    let mut freestyle_caches = EvalCaches::new();
    recompute_all_for_rule(&mut board, &mut freestyle_caches, RuleSet::Freestyle);
    let freestyle_result = generate_candidates(
        &board,
        &freestyle_caches,
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
    let mut renju_caches = EvalCaches::new();
    recompute_all_for_rule(&mut board, &mut renju_caches, RuleSet::Renju);
    let renju_result = generate_candidates(
        &board,
        &renju_caches,
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
