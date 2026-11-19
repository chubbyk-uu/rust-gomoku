//! Move ordering helpers aligned with the classic reference.

use crate::board::{move_to_xy, Board};
use crate::constants::BOARD_AREA;
use crate::search::movegen::Candidate;
use crate::types::{Move, Side};

pub fn ordering_backend_name() -> &'static str {
    "python"
}

pub fn getmi(board: &Board, x: usize, y: usize, side: Side) -> i32 {
    let mut ret = 1_i32;
    let opponent = -side;
    let grid = board.grid_rows();
    let size = board.size();

    let mut ii = x + 1;
    while ii <= x + 4 && ii < size {
        if grid[y][ii] == opponent {
            break;
        }
        ret += 1;
        ii += 1;
    }

    let mut ii = x as isize - 1;
    while ii >= x as isize - 4 && ii >= 0 {
        if grid[y][ii as usize] == opponent {
            break;
        }
        ret += 1;
        ii -= 1;
    }

    let mut jj = y + 1;
    while jj <= y + 4 && jj < size {
        if grid[jj][x] == opponent {
            break;
        }
        ret += 1;
        jj += 1;
    }

    let mut jj = y as isize - 1;
    while jj >= y as isize - 4 && jj >= 0 {
        if grid[jj as usize][x] == opponent {
            break;
        }
        ret += 1;
        jj -= 1;
    }

    let mut ii = x + 1;
    let mut jj = y + 1;
    while ii <= x + 4 && ii < size && jj < size {
        if grid[jj][ii] == opponent {
            break;
        }
        ret += 1;
        ii += 1;
        jj += 1;
    }

    let mut ii = x as isize - 1;
    let mut jj = y as isize - 1;
    while ii >= x as isize - 4 && ii >= 0 && jj >= 0 {
        if grid[jj as usize][ii as usize] == opponent {
            break;
        }
        ret += 1;
        ii -= 1;
        jj -= 1;
    }

    let mut ii = x as isize - 1;
    let mut jj = y + 1;
    while ii >= x as isize - 4 && ii >= 0 && jj < size {
        if grid[jj][ii as usize] == opponent {
            break;
        }
        ret += 1;
        ii -= 1;
        jj += 1;
    }

    let mut ii = x + 1;
    let mut jj = y as isize - 1;
    while ii <= x + 4 && ii < size && jj >= 0 {
        if grid[jj as usize][ii] == opponent {
            break;
        }
        ret += 1;
        ii += 1;
        jj -= 1;
    }

    ret
}

pub fn order_candidates(
    board: &Board,
    candidates: &[Candidate],
    side: Side,
    tt_best_move: Option<Move>,
) -> Vec<Candidate> {
    let mut result = candidates.to_vec();
    order_candidates_in_place(board, &mut result, side, tt_best_move);
    result
}

pub(crate) fn order_candidates_owned(
    board: &Board,
    mut candidates: Vec<Candidate>,
    side: Side,
    tt_best_move: Option<Move>,
) -> Vec<Candidate> {
    order_candidates_in_place(board, &mut candidates, side, tt_best_move);
    candidates
}

pub(crate) fn order_candidates_primary_slice(candidates: &mut [Candidate]) {
    candidates.sort_unstable_by(|a, b| {
        b.order_score
            .partial_cmp(&a.order_score)
            .expect("candidate scores are finite")
    });
}

pub(crate) fn order_candidates_primary_owned(
    mut candidates: Vec<Candidate>,
    tt_best_move: Option<Move>,
) -> Vec<Candidate> {
    candidates.sort_unstable_by(|a, b| {
        let a_tt = tt_best_move == Some(a.move_);
        let b_tt = tt_best_move == Some(b.move_);
        b_tt.cmp(&a_tt).then_with(|| {
            b.order_score
                .partial_cmp(&a.order_score)
                .expect("candidate scores are finite")
        })
    });
    candidates
}

pub(crate) fn resolve_candidate_tie_group(
    board: &Board,
    ordered: &mut [Candidate],
    start: usize,
    side: Side,
    tt_best_move: Option<Move>,
) -> usize {
    let mut end = start + 1;
    while end < ordered.len() && same_primary_key(ordered[start], ordered[end], tt_best_move) {
        end += 1;
    }
    let group_len = end - start;
    if group_len > 1 {
        if group_len <= 16 {
            let mut keyed = [(0_i32, ordered[start]); 16];
            for (slot, &candidate) in keyed[..group_len].iter_mut().zip(&ordered[start..end]) {
                let (x, y) = move_to_xy(candidate.move_).expect("candidate move is in range");
                *slot = (getmi(board, x, y, side), candidate);
            }
            keyed[..group_len]
                .sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.move_.cmp(&b.1.move_)));
            for (dst, &(_, candidate)) in ordered[start..end].iter_mut().zip(&keyed[..group_len]) {
                *dst = candidate;
            }
        } else {
            let mut mi_cache = [0_i32; BOARD_AREA];
            ordered[start..end].sort_unstable_by(|a, b| {
                let a_mi = cached_getmi(board, a.move_, side, &mut mi_cache);
                let b_mi = cached_getmi(board, b.move_, side, &mut mi_cache);
                b_mi.cmp(&a_mi).then_with(|| a.move_.cmp(&b.move_))
            });
        }
    }
    end
}

fn cached_getmi(board: &Board, move_: Move, side: Side, mi_cache: &mut [i32; BOARD_AREA]) -> i32 {
    let index = move_ as usize;
    let cached = mi_cache[index];
    if cached != 0 {
        return cached;
    }
    let (x, y) = move_to_xy(move_).expect("candidate move is in range");
    let mi = getmi(board, x, y, side);
    mi_cache[index] = mi;
    mi
}

fn same_primary_key(a: Candidate, b: Candidate, tt_best_move: Option<Move>) -> bool {
    (tt_best_move == Some(a.move_)) == (tt_best_move == Some(b.move_))
        && a.order_score == b.order_score
}

fn order_candidates_in_place(
    board: &Board,
    result: &mut [Candidate],
    side: Side,
    tt_best_move: Option<Move>,
) {
    result.sort_unstable_by(|a, b| {
        let a_tt = tt_best_move == Some(a.move_);
        let b_tt = tt_best_move == Some(b.move_);
        b_tt.cmp(&a_tt).then_with(|| {
            b.order_score
                .partial_cmp(&a.order_score)
                .expect("candidate scores are finite")
        })
    });

    let mut mi_cache = [0_i32; BOARD_AREA];
    let mut start = 0_usize;
    while start < result.len() {
        let mut end = start + 1;
        while end < result.len() && same_primary_key(result[start], result[end], tt_best_move) {
            end += 1;
        }
        if end - start > 1 {
            result[start..end].sort_unstable_by(|a, b| {
                let a_mi = cached_getmi(board, a.move_, side, &mut mi_cache);
                let b_mi = cached_getmi(board, b.move_, side, &mut mi_cache);
                b_mi.cmp(&a_mi).then_with(|| a.move_.cmp(&b.move_))
            });
        }
        start = end;
    }
}

pub fn order_candidates_root_classic(
    board: &Board,
    candidates: &[Candidate],
    side: Side,
) -> Vec<Candidate> {
    let mut ordered = candidates.to_vec();
    let mut mis = [0_i32; BOARD_AREA];
    let limit = ordered.len();
    for i in 0..limit {
        let mut best_index = i;
        let mut best = ordered[best_index];
        let mut best_mi = 0_i32;
        for j in (i + 1)..limit {
            let candidate = ordered[j];
            if candidate.order_score > best.order_score {
                best_index = j;
                best = candidate;
                best_mi = 0;
                continue;
            }
            if candidate.order_score < best.order_score {
                continue;
            }
            if best_mi == 0 {
                let (bx, by) = move_to_xy(best.move_).expect("candidate move is in range");
                best_mi = getmi(board, bx, by, side);
                mis[usize::from(best.move_)] = best_mi;
            }
            let mut candidate_mi = mis[usize::from(candidate.move_)];
            if candidate_mi == 0 {
                let (cx, cy) = move_to_xy(candidate.move_).expect("candidate move is in range");
                candidate_mi = getmi(board, cx, cy, side);
                mis[usize::from(candidate.move_)] = candidate_mi;
            }
            if candidate_mi > best_mi {
                best_index = j;
                best = candidate;
                best_mi = candidate_mi;
            }
        }
        ordered.swap(i, best_index);
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{xy_to_move, Board};

    fn candidate(x: usize, y: usize, order_score: f64, self_attack: i32) -> Candidate {
        Candidate {
            move_: xy_to_move(x, y).unwrap(),
            order_score,
            self_attack,
            opp_attack: 0,
        }
    }

    #[test]
    fn lazy_tie_group_ordering_matches_full_order_candidates_owned() {
        let mut board = Board::new();
        board.play(xy_to_move(7, 7).unwrap(), Some(1)).unwrap();
        board.play(xy_to_move(8, 7).unwrap(), Some(-1)).unwrap();
        board.play(xy_to_move(6, 7).unwrap(), Some(1)).unwrap();
        let tt_move = xy_to_move(5, 5).unwrap();
        let raw = vec![
            candidate(3, 3, 10.0, 0),
            candidate(7, 6, 10.0, 0),
            candidate(5, 5, 1.0, 0),
            candidate(8, 8, 10.0, 0),
            candidate(2, 2, 5.0, 0),
            candidate(6, 6, 5.0, 0),
            candidate(9, 9, 20.0, 0),
        ];

        let expected = order_candidates_owned(&board, raw.clone(), 1, Some(tt_move));
        let mut lazy = order_candidates_primary_owned(raw, Some(tt_move));
        let mut sorted_until = 0;
        for idx in 0..lazy.len() {
            if idx >= sorted_until {
                sorted_until =
                    resolve_candidate_tie_group(&board, &mut lazy, idx, 1, Some(tt_move));
            }
        }
        assert_eq!(lazy, expected);
    }
}
