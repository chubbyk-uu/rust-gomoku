use rust_gomoku::{
    move_to_rc, move_to_xy, rc_to_move, xy_to_move, Board, BoardError, PlayedMove, BLACK,
    BOARD_SIZE, EMPTY, WHITE,
};

#[test]
fn coordinate_conversion_round_trip() {
    let move_ = xy_to_move(7, 11).unwrap();
    assert_eq!(move_to_xy(move_).unwrap(), (7, 11));
}

#[test]
fn row_col_helpers_match_xy_helpers() {
    let move_ = xy_to_move(7, 11).unwrap();
    assert_eq!(rc_to_move(11, 7).unwrap(), move_);
    assert_eq!(move_to_xy(move_).unwrap(), (7, 11));
    assert_eq!(move_to_rc(move_).unwrap(), (11, 7));
}

#[test]
fn board_starts_empty() {
    let board = Board::new();
    assert_eq!(board.move_count(), 0);
    assert_eq!(board.side_to_move(), BLACK);
    assert_eq!(board.winner(), EMPTY);
    assert_eq!(board.at(0, 0).unwrap(), EMPTY);
}

#[test]
fn play_and_undo_restore_state() {
    let mut board = Board::new();
    let first = board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let second = board.play(xy_to_move(7, 8).unwrap(), None).unwrap();

    assert_eq!(
        first,
        PlayedMove {
            move_: xy_to_move(7, 7).unwrap(),
            side: BLACK
        }
    );
    assert_eq!(
        second,
        PlayedMove {
            move_: xy_to_move(7, 8).unwrap(),
            side: WHITE
        }
    );
    assert_eq!(board.move_count(), 2);
    assert_eq!(board.side_to_move(), BLACK);

    let undone = board.undo().unwrap();
    assert_eq!(undone, second);
    assert_eq!(board.move_count(), 1);
    assert_eq!(board.side_to_move(), WHITE);
    assert_eq!(board.at(7, 8).unwrap(), EMPTY);
}

#[test]
fn play_rejects_occupied_point() {
    let mut board = Board::new();
    let move_ = xy_to_move(7, 7).unwrap();
    board.play(move_, None).unwrap();
    assert_eq!(board.play(move_, None), Err(BoardError::IllegalMove(move_)));
}

#[test]
fn play_rejects_wrong_side() {
    let mut board = Board::new();
    let move_ = xy_to_move(7, 7).unwrap();
    assert_eq!(
        board.play(move_, Some(WHITE)),
        Err(BoardError::WrongSideToMove {
            expected: BLACK,
            got: WHITE
        })
    );
}

#[test]
fn horizontal_win_detection() {
    let mut board = Board::new();
    let moves = [
        xy_to_move(3, 7).unwrap(),
        xy_to_move(0, 0).unwrap(),
        xy_to_move(4, 7).unwrap(),
        xy_to_move(0, 1).unwrap(),
        xy_to_move(5, 7).unwrap(),
        xy_to_move(0, 2).unwrap(),
        xy_to_move(6, 7).unwrap(),
        xy_to_move(0, 3).unwrap(),
        xy_to_move(7, 7).unwrap(),
    ];
    for move_ in moves {
        board.play(move_, None).unwrap();
    }
    assert_eq!(board.winner(), BLACK);
}

#[test]
fn diagonal_win_detection() {
    let mut board = Board::new();
    let moves = [
        xy_to_move(2, 2).unwrap(),
        xy_to_move(0, 0).unwrap(),
        xy_to_move(3, 3).unwrap(),
        xy_to_move(0, 1).unwrap(),
        xy_to_move(4, 4).unwrap(),
        xy_to_move(0, 2).unwrap(),
        xy_to_move(5, 5).unwrap(),
        xy_to_move(0, 3).unwrap(),
        xy_to_move(6, 6).unwrap(),
    ];
    for move_ in moves {
        board.play(move_, None).unwrap();
    }
    assert_eq!(board.winner(), BLACK);
}

#[test]
fn board_replay_reconstructs_position() {
    let moves = [
        xy_to_move(7, 7).unwrap(),
        xy_to_move(8, 7).unwrap(),
        xy_to_move(7, 8).unwrap(),
    ];
    let mut board = Board::new();
    board.replay(&moves, BLACK).unwrap();
    assert_eq!(board.occupied_moves(), moves);
    assert_eq!(board.at(7, 7).unwrap(), BLACK);
    assert_eq!(board.at_rc(7, 7).unwrap(), BLACK);
    assert_eq!(board.at(8, 7).unwrap(), WHITE);
    assert_eq!(board.at_rc(7, 8).unwrap(), WHITE);
    assert_eq!(board.at(7, 8).unwrap(), BLACK);
    assert_eq!(board.at_rc(8, 7).unwrap(), BLACK);
}

#[test]
fn zobrist_changes_after_move() {
    let mut board = Board::new();
    let start_key = board.zobrist_key();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    assert_ne!(board.zobrist_key(), start_key);
}

#[test]
fn zobrist_restores_after_undo() {
    let mut board = Board::new();
    let start_key = board.zobrist_key();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.undo().unwrap();
    board.undo().unwrap();
    assert_eq!(board.zobrist_key(), start_key);
}

#[test]
fn same_position_same_hash() {
    let moves = [
        xy_to_move(7, 7).unwrap(),
        xy_to_move(8, 7).unwrap(),
        xy_to_move(7, 8).unwrap(),
        xy_to_move(8, 8).unwrap(),
    ];
    let mut board_a = Board::new();
    let mut board_b = Board::new();
    for move_ in moves {
        board_a.play(move_, None).unwrap();
    }
    board_b.replay(&moves, BLACK).unwrap();
    assert_eq!(board_a.zobrist_key(), board_b.zobrist_key());
}

#[test]
fn hash_tracks_partial_undo_correctly() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let key_after_first = board.zobrist_key();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    board.undo().unwrap();
    board.undo().unwrap();
    assert_eq!(board.zobrist_key(), key_after_first);
}

#[test]
fn side_to_move_does_not_change_hash_on_same_grid() {
    let black_to_move = Board::new();
    let white_to_move = Board::with_side_to_move(WHITE).unwrap();
    assert_eq!(black_to_move.zobrist_key(), white_to_move.zobrist_key());
}

#[test]
fn move_encoding_rejects_out_of_range_values() {
    assert_eq!(
        xy_to_move(BOARD_SIZE, 0),
        Err(BoardError::CoordinatesOutOfRange {
            x: BOARD_SIZE,
            y: 0
        })
    );
}

#[test]
fn full_board_without_five_is_a_draw() {
    let mut black_moves = Vec::new();
    let mut white_moves = Vec::new();
    for y in 0..BOARD_SIZE {
        for x in 0..BOARD_SIZE {
            let move_ = xy_to_move(x, y).unwrap();
            if (x + 2 * y) % 4 < 2 {
                black_moves.push(move_);
            } else {
                white_moves.push(move_);
            }
        }
    }

    let mut board = Board::new();
    for index in 0..black_moves.len() {
        board.play(black_moves[index], None).unwrap();
        if let Some(&move_) = white_moves.get(index) {
            board.play(move_, None).unwrap();
        }
    }

    assert!(board.is_full());
    assert!(board.is_draw());
    assert!(board.is_terminal());
    assert_eq!(board.winner(), 0);
}
