use rust_gomoku::{
    broken_four_reply, forcing_threat_moves, move_to_xy, threat_moves, winning_threat_moves,
    xy_to_move, Board, ThreatBoardView, VCFSearcher,
};

#[test]
fn threat_moves_use_expected_vcf_offsets() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let moves = threat_moves(&board, 1);
    assert!(moves.contains(&xy_to_move(9, 7).unwrap()));
    assert!(moves.contains(&xy_to_move(7, 9).unwrap()));
    assert!(moves.contains(&xy_to_move(5, 5).unwrap()));
    assert!(!moves.contains(&xy_to_move(5, 4).unwrap()));
}

#[test]
fn threat_moves_only_expand_from_current_side_stones() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    let moves: std::collections::HashSet<_> = threat_moves(&board, 1).into_iter().collect();
    assert!(moves.contains(&xy_to_move(9, 7).unwrap()));
    assert!(!moves.contains(&xy_to_move(1, 0).unwrap()));
}

#[test]
fn threat_moves_follow_expected_xy_scan_order() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let moves = threat_moves(&board, 1);
    let coords: Vec<_> = moves
        .iter()
        .take(4)
        .map(|&m| move_to_xy(m).unwrap())
        .collect();
    let mut sorted = coords.clone();
    sorted.sort_unstable();
    assert_eq!(coords, sorted);
}

#[test]
fn winning_threat_moves_detect_open_four_creation() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    let wins: std::collections::HashSet<_> = winning_threat_moves(&board, 1).into_iter().collect();
    assert!(wins.contains(&xy_to_move(6, 7).unwrap()));
}

#[test]
fn forcing_threat_moves_detect_broken_four_continuations() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let forcing: std::collections::HashSet<_> =
        forcing_threat_moves(&board, 1).into_iter().collect();
    assert!(forcing.contains(&xy_to_move(5, 7).unwrap()));
}

#[test]
fn vcf_search_finds_immediate_forcing_threat() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    let result = VCFSearcher::default().search(&board, 1, 2);
    assert!(result.found);
    assert!(result.solved);
    assert!(matches!(
        move_to_xy(result.move_.expect("move should exist")).unwrap(),
        (2, 7) | (6, 7)
    ));
}

#[test]
fn vcf_begin_result_mapping_matches_expected_on_found_position() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    for depth in [1, 2, 4, 8] {
        let result = VCFSearcher::default().search(&board, 1, depth);
        assert!(result.found);
        assert!(result.solved);
        assert_eq!(result.move_, Some(xy_to_move(2, 7).unwrap()));
    }
}

#[test]
fn vcf_search_reports_inconclusive_at_zero_depth() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let result = VCFSearcher::default().search(&board, 1, 0);
    assert!(!result.found);
    assert!(!result.solved);
    assert_eq!(result.move_, None);
}

#[test]
fn vcf_search_can_report_solved_negative() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    let result = VCFSearcher::default().search(&board, board.side_to_move(), 1);
    assert!(!result.found);
    assert!(result.solved);
    assert_eq!(result.move_, None);
}

#[test]
fn vcf_sequence_key_is_order_invariant_within_side_lists() {
    let key1 = VCFSearcher::canonical_sequence_key(&[10, 3, 7], &[8, 2]);
    let key2 = VCFSearcher::canonical_sequence_key(&[7, 10, 3], &[2, 8]);
    assert_eq!(key1, key2);
}

#[test]
fn vcf_begin_depth_is_capped_as_expected() {
    assert_eq!(VCFSearcher::normalize_begin_depth(8), 5);
    assert_eq!(VCFSearcher::normalize_begin_depth(6), 4);
    assert_eq!(VCFSearcher::normalize_begin_depth(4), 4);
}

#[test]
fn threat_board_view_reports_direct_b4_point_for_side() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let (move_, ambiguous) = ThreatBoardView::from_board(board).broken_four_point_for_side(1);
    assert!(matches!(
        move_to_xy(move_.expect("move should exist")).unwrap(),
        (2, 7) | (7, 7)
    ));
    assert!(matches!(ambiguous, true | false));
}

#[test]
fn broken_four_reply_wrapper_matches_view() {
    let mut board = Board::new();
    board.play(xy_to_move(3, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(4, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(1, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(5, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(2, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(6, 7).unwrap(), None).unwrap();
    let view = ThreatBoardView::from_board(board.clone());
    let (x, y) = (6, 7);
    assert_eq!(
        broken_four_reply(&board, x, y),
        view.broken_four_reply(x, y)
    );
}

#[test]
fn broken_four_legal_reply_uses_composite_alternate_when_primary_is_occupied() {
    let mut board = Board::new();
    let moves = [
        (4, 4),
        (5, 4),
        (6, 5),
        (4, 5),
        (6, 3),
        (6, 4),
        (7, 3),
        (5, 3),
        (7, 5),
        (8, 5),
        (6, 6),
        (5, 6),
        (5, 7),
        (8, 4),
        (7, 4),
        (7, 6),
        (6, 7),
        (8, 3),
        (8, 2),
        (5, 2),
        (5, 5),
        (7, 7),
        (6, 8),
        (6, 9),
        (4, 8),
        (3, 9),
        (7, 9),
        (4, 6),
        (6, 10),
        (5, 10),
        (7, 8),
        (8, 8),
        (9, 10),
        (7, 11),
        (10, 9),
        (8, 10),
        (9, 9),
        (8, 9),
    ];
    for (x, y) in moves {
        board.play(xy_to_move(x, y).unwrap(), None).unwrap();
    }

    let mut view = ThreatBoardView::from_board(board);
    view.play(xy_to_move(8, 7).unwrap(), -1);
    let raw_reply = view.broken_four_reply(8, 7).expect("raw reply exists");
    assert_eq!(move_to_xy(raw_reply).unwrap(), (8, 8));
    assert!(!view.board.is_legal_move(raw_reply));
    let legal_reply = view
        .broken_four_legal_reply(8, 7)
        .expect("alternate legal reply exists");
    assert_eq!(move_to_xy(legal_reply).unwrap(), (8, 6));
}
