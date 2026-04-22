use rust_gomoku::{has_vct_trigger, move_to_xy, xy_to_move, Board, VCFSearcher, VCTSearcher};

fn make_board(moves: &[(usize, usize, i8)]) -> Board {
    let mut board = Board::new();
    for &(x, y, side) in moves {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    board
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
