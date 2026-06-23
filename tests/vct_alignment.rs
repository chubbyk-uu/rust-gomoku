use rust_gomoku::{
    classify_forbidden_move, forcing_threat_moves, forcing_threat_moves_for_rule, has_vct_trigger,
    move_to_xy, xy_to_move, AttackMove, Board, ForbiddenKind, RuleSet, ThreatBoardView,
    ThreatLevel, VCFSearcher, VCTSearcher,
};

fn make_board(moves: &[(usize, usize, i8)]) -> Board {
    let mut board = Board::new();
    for &(x, y, side) in moves {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    board
}

fn classify_after_play(
    board: &Board,
    x: usize,
    y: usize,
    attacker: i8,
    rule: RuleSet,
) -> Option<AttackMove> {
    let mut view = ThreatBoardView::from_board_with_rule(board.clone(), rule);
    let move_ = xy_to_move(x, y).unwrap();
    view.play(move_, attacker);
    view.classify_attack_at(x, y, attacker, move_)
}

fn is_four(attack: &Option<AttackMove>) -> bool {
    matches!(
        attack.as_ref().map(|a| a.level),
        Some(ThreatLevel::B4) | Some(ThreatLevel::A4) | Some(ThreatLevel::WIN5)
    )
}

// Black plays (9,7) onto row 7 black stones 4,5,7,8. Freestyle sees a broken
// four whose gap is (6,7); in Renju filling (6,7) makes 4..9 a six (overline),
// so the only "winning" completion is illegal and the move is not a real four.
#[test]
fn renju_classify_demotes_black_overline_four() {
    let board = make_board(&[
        (4, 7, 1),
        (0, 0, -1),
        (5, 7, 1),
        (1, 0, -1),
        (7, 7, 1),
        (2, 0, -1),
        (8, 7, 1),
    ]);
    // Confirm the geometry: with the move played, the gap fill is an overline.
    let after = make_board(&[
        (4, 7, 1),
        (0, 0, -1),
        (5, 7, 1),
        (1, 0, -1),
        (7, 7, 1),
        (2, 0, -1),
        (8, 7, 1),
        (3, 0, -1),
        (9, 7, 1),
    ]);
    assert_eq!(
        classify_forbidden_move(&after, xy_to_move(6, 7).unwrap(), 1, RuleSet::Renju).unwrap(),
        ForbiddenKind::Overline
    );

    let freestyle = classify_after_play(&board, 9, 7, 1, RuleSet::Freestyle);
    assert!(is_four(&freestyle), "freestyle treats it as a four");

    let renju = classify_after_play(&board, 9, 7, 1, RuleSet::Renju);
    assert!(
        !is_four(&renju),
        "renju must not treat an overline-four as a four"
    );
}

// Black open three 5,6,7 flanked by anchors at (2,7) and (10,7). Both open-four
// extensions ((4,7) and (8,7)) only reach a simple four because the far
// completion would be an overline, so this is a fake open three in Renju.
#[test]
fn renju_classify_rejects_fake_open_three() {
    let board = make_board(&[
        (5, 7, 1),
        (0, 0, -1),
        (6, 7, 1),
        (1, 0, -1),
        (2, 7, 1),
        (3, 0, -1),
        (10, 7, 1),
    ]);
    let freestyle = classify_after_play(&board, 7, 7, 1, RuleSet::Freestyle);
    assert_eq!(
        freestyle.as_ref().map(|a| a.level),
        Some(ThreatLevel::A3),
        "freestyle sees an open three"
    );

    let renju = classify_after_play(&board, 7, 7, 1, RuleSet::Renju);
    assert!(
        renju.is_none(),
        "renju rejects the fake open three, got {renju:?}"
    );
}

// White broken four at row 7 (white 5,6,8 + move 9, gap 7); black's only block
// (7,7) is a double-three forbidden point, so the four is unstoppable (A4).
#[test]
fn renju_classify_promotes_white_four_when_black_block_forbidden() {
    let board = make_board(&[
        (6, 6, 1),
        (5, 7, -1),
        (8, 8, 1),
        (6, 7, -1),
        (6, 8, 1),
        (8, 7, -1),
        (8, 6, 1),
    ]);
    assert_eq!(
        classify_forbidden_move(&board, xy_to_move(7, 7).unwrap(), 1, RuleSet::Renju).unwrap(),
        ForbiddenKind::DoubleThree
    );

    let freestyle = classify_after_play(&board, 9, 7, -1, RuleSet::Freestyle);
    assert_eq!(
        freestyle.as_ref().map(|a| a.level),
        Some(ThreatLevel::B4),
        "freestyle lets black block the broken four"
    );

    let renju = classify_after_play(&board, 9, 7, -1, RuleSet::Renju);
    assert_eq!(
        renju.as_ref().map(|a| a.level),
        Some(ThreatLevel::A4),
        "renju makes white's four unstoppable when black cannot block"
    );
}

// End-to-end: a black move that is a double-four win in freestyle VCT must not
// be returned by Renju VCT because the move is forbidden.
#[test]
fn renju_vct_does_not_return_forbidden_double_four() {
    let board = make_board(&[
        (5, 7, 1),
        (0, 0, -1),
        (6, 7, 1),
        (2, 0, -1),
        (8, 7, 1),
        (4, 0, -1),
        (7, 5, 1),
        (6, 0, -1),
        (7, 6, 1),
        (8, 0, -1),
        (7, 8, 1),
    ]);
    let forbidden = xy_to_move(7, 7).unwrap();
    assert_eq!(
        classify_forbidden_move(&board, forbidden, 1, RuleSet::Renju).unwrap(),
        ForbiddenKind::DoubleFour
    );

    let renju = VCTSearcher::default().search_for_rule(&board, 1, 4, RuleSet::Renju);
    assert_ne!(renju.move_, Some(forbidden));
}

#[test]
fn renju_forcing_threats_filter_forbidden_black_moves() {
    let board = make_board(&[
        (5, 7, 1),
        (0, 0, -1),
        (6, 7, 1),
        (2, 0, -1),
        (8, 7, 1),
        (4, 0, -1),
        (7, 5, 1),
        (6, 0, -1),
        (7, 6, 1),
        (8, 0, -1),
        (7, 8, 1),
    ]);
    let forbidden = xy_to_move(7, 7).unwrap();
    assert_eq!(
        classify_forbidden_move(&board, forbidden, 1, RuleSet::Renju).unwrap(),
        ForbiddenKind::DoubleFour
    );

    let freestyle = forcing_threat_moves(&board, 1);
    assert!(
        freestyle.contains(&forbidden),
        "freestyle helper sees the double-four forcing point"
    );

    let renju = forcing_threat_moves_for_rule(&board, 1, RuleSet::Renju);
    assert!(
        !renju.contains(&forbidden),
        "renju helper must filter the forbidden forcing point"
    );
}

// End-to-end: white wins by VCT because black's forced block is forbidden.
#[test]
fn renju_vct_white_wins_when_black_block_forbidden() {
    let board = make_board(&[
        (6, 6, 1),
        (5, 7, -1),
        (8, 8, 1),
        (6, 7, -1),
        (6, 8, 1),
        (8, 7, -1),
        (8, 6, 1),
    ]);
    assert_eq!(board.side_to_move(), -1);
    let renju = VCTSearcher::default().search_for_rule(&board, -1, 3, RuleSet::Renju);
    assert!(renju.found, "white should find a continuous-threat win");
}

#[test]
fn has_vct_trigger_is_false_on_quiet_position() {
    let board = make_board(&[(7, 7, 1), (0, 0, -1)]);
    assert!(!has_vct_trigger(&board, 1));
}

#[test]
fn has_vct_trigger_is_true_on_b4_position() {
    let board = make_board(&[
        (5, 7, 1),
        (4, 7, -1),
        (6, 7, 1),
        (0, 0, -1),
        (7, 7, 1),
        (1, 0, -1),
    ]);
    assert!(has_vct_trigger(&board, 1));
}

#[test]
fn has_vct_trigger_is_true_on_dual_a3() {
    let board = make_board(&[
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
        (14, 14, -1),
    ]);
    assert!(has_vct_trigger(&board, 1));
}

#[test]
fn nested_a3r_count_restores_threat_board_view_state() {
    let board = make_board(&[
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
    ]);
    assert_eq!(board.side_to_move(), -1);

    let mut view = ThreatBoardView::from_board(board);
    let before_board = view.board.clone();
    let before_x1 = view.x1.clone();
    let before_x2 = view.x2.clone();
    let before_x3 = view.x3.clone();
    let before_x4 = view.x4.clone();

    let move_ = xy_to_move(7, 7).unwrap();
    view.play(move_, 1);
    let (x, y) = move_to_xy(move_).unwrap();
    assert!(view.a3r_count(x, y) >= 2);
    view.undo();

    assert_eq!(view.board, before_board);
    assert_eq!(view.x1, before_x1);
    assert_eq!(view.x2, before_x2);
    assert_eq!(view.x3, before_x3);
    assert_eq!(view.x4, before_x4);
}

#[test]
fn vct_returns_not_found_on_depth_zero() {
    let board = make_board(&[
        (5, 7, 1),
        (4, 7, -1),
        (6, 7, 1),
        (0, 0, -1),
        (7, 7, 1),
        (1, 0, -1),
    ]);
    let r = VCTSearcher::default().search(&board, 1, 0);
    assert!(!r.found);
}

#[test]
fn vct_returns_solved_on_terminal_board() {
    let board = make_board(&[
        (5, 7, 1),
        (0, 0, -1),
        (6, 7, 1),
        (1, 0, -1),
        (7, 7, 1),
        (2, 0, -1),
        (8, 7, 1),
        (3, 0, -1),
        (9, 7, 1),
    ]);
    let r = VCTSearcher::default().search(&board, 1, 4);
    assert!(r.solved);
}

#[test]
fn dual_a3_win_is_found_by_vct_but_not_vcf() {
    let board = make_board(&[
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
        (14, 14, -1),
    ]);
    assert!(!VCFSearcher::default().search(&board, 1, 8).found);
    let r = VCTSearcher::default().search(&board, 1, 4);
    assert!(r.found);
    assert_eq!(
        move_to_xy(r.move_.expect("move should exist")).unwrap(),
        (7, 7)
    );
}

#[test]
fn vct_reports_no_win_when_defender_survives() {
    let board = make_board(&[
        (5, 7, 1),
        (4, 7, -1),
        (6, 7, 1),
        (0, 1, -1),
        (7, 7, 1),
        (1, 1, -1),
    ]);
    let r = VCTSearcher::default().search(&board, 1, 8);
    assert!(!r.found);
    assert!(r.solved);
}

#[test]
fn vct_iterative_deepening_returns_early_on_find() {
    let board = make_board(&[
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
        (14, 14, -1),
    ]);
    let r = VCTSearcher::default().search(&board, 1, 6);
    assert!(r.found);
}

#[test]
fn vct_memo_is_cleared_between_calls() {
    let board = make_board(&[
        (6, 7, 1),
        (0, 0, -1),
        (8, 7, 1),
        (1, 3, -1),
        (7, 6, 1),
        (3, 1, -1),
        (7, 8, 1),
        (14, 14, -1),
    ]);
    let mut searcher = VCTSearcher::default();
    let r1 = searcher.search(&board, 1, 4);
    let r2 = searcher.search(&board, 1, 4);
    assert_eq!(r1.found, r2.found);
    assert_eq!(r1.move_, r2.move_);
}
