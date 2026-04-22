use rust_gomoku::{
    getmi, order_candidates, order_candidates_root_classic, ordering_backend_name, xy_to_move,
    Board, Candidate,
};

fn candidate(x: usize, y: usize, order_score: f64) -> Candidate {
    Candidate {
        move_: xy_to_move(x, y).unwrap(),
        order_score,
        self_attack: 0,
        opp_attack: 0,
    }
}

#[test]
fn ordering_backend_name_is_supported() {
    assert!(matches!(ordering_backend_name(), "python" | "cython"));
}

#[test]
fn getmi_matches_reference_blocking_pattern() {
    let mut board = Board::new();
    let sequence = [
        (1, (6, 7)),
        (-1, (9, 7)),
        (1, (7, 8)),
        (-1, (7, 5)),
        (1, (6, 6)),
        (-1, (8, 8)),
        (1, (6, 8)),
        (-1, (5, 9)),
    ];
    for (side, (x, y)) in sequence {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }
    assert_eq!(getmi(&board, 7, 7, 1), 20);
}

#[test]
fn order_candidates_matches_reference_tuple_sort() {
    let board = Board::new();
    let ordered = order_candidates(
        &board,
        &[
            candidate(13, 13, 10.0),
            candidate(1, 1, 10.0),
            candidate(7, 7, 10.0),
            candidate(3, 3, 12.0),
        ],
        1,
        Some(xy_to_move(13, 13).unwrap()),
    );
    let moves: Vec<_> = ordered.iter().map(|candidate| candidate.move_).collect();
    assert_eq!(
        moves,
        vec![
            xy_to_move(13, 13).unwrap(),
            xy_to_move(3, 3).unwrap(),
            xy_to_move(7, 7).unwrap(),
            xy_to_move(1, 1).unwrap(),
        ]
    );
}

#[test]
fn order_candidates_root_classic_matches_reference_selection_sort() {
    let board = Board::new();
    let ordered = order_candidates_root_classic(
        &board,
        &[
            candidate(13, 13, 10.0),
            candidate(1, 1, 10.0),
            candidate(7, 7, 10.0),
            candidate(3, 3, 12.0),
        ],
        1,
    );
    let moves: Vec<_> = ordered.iter().map(|candidate| candidate.move_).collect();
    assert_eq!(
        moves,
        vec![
            xy_to_move(3, 3).unwrap(),
            xy_to_move(7, 7).unwrap(),
            xy_to_move(1, 1).unwrap(),
            xy_to_move(13, 13).unwrap(),
        ]
    );
}
