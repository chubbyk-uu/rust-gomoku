use rust_gomoku::{
    getmi, move_to_xy, order_candidates, order_candidates_root_classic, ordering_backend_name,
    xy_to_move, Board, Candidate, Move, BOARD_AREA,
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

fn eager_order_candidates_reference(
    board: &Board,
    candidates: &[Candidate],
    side: i8,
    tt_best_move: Option<Move>,
) -> Vec<Move> {
    let mut mi_cache = [0_i32; BOARD_AREA];
    for candidate in candidates {
        let (x, y) = move_to_xy(candidate.move_).expect("candidate move is in range");
        mi_cache[candidate.move_ as usize] = getmi(board, x, y, side);
    }
    let mut result = candidates.to_vec();
    result.sort_unstable_by(|a, b| {
        let a_tt = tt_best_move == Some(a.move_);
        let b_tt = tt_best_move == Some(b.move_);
        b_tt.cmp(&a_tt)
            .then_with(|| {
                b.order_score
                    .partial_cmp(&a.order_score)
                    .expect("candidate scores are finite")
            })
            .then_with(|| mi_cache[b.move_ as usize].cmp(&mi_cache[a.move_ as usize]))
            .then_with(|| a.move_.cmp(&b.move_))
    });
    result.iter().map(|candidate| candidate.move_).collect()
}

#[test]
fn order_candidates_matches_eager_getmi_reference_across_ties() {
    let mut board = Board::new();
    for (side, (x, y)) in [
        (1, (7, 7)),
        (-1, (8, 7)),
        (1, (6, 8)),
        (-1, (9, 8)),
        (1, (5, 9)),
        (-1, (10, 9)),
        (1, (8, 6)),
        (-1, (4, 10)),
    ] {
        board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
    }

    let candidates = [
        candidate(6, 7, 12.0),
        candidate(7, 6, 12.0),
        candidate(9, 7, 12.0),
        candidate(8, 8, 11.0),
        candidate(5, 8, 11.0),
        candidate(10, 8, 10.0),
        candidate(4, 9, 10.0),
        candidate(11, 9, 10.0),
    ];
    let tt_best = Some(xy_to_move(5, 8).unwrap());
    let expected = eager_order_candidates_reference(&board, &candidates, 1, tt_best);
    let actual: Vec<_> = order_candidates(&board, &candidates, 1, tt_best)
        .iter()
        .map(|candidate| candidate.move_)
        .collect();
    assert_eq!(actual, expected);
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
