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
    let mut ordered: Vec<(Candidate, bool, i32)> = candidates
        .iter()
        .map(|candidate| {
            let (x, y) = move_to_xy(candidate.move_).expect("candidate move is in range");
            (
                *candidate,
                tt_best_move == Some(candidate.move_),
                getmi(board, x, y, side),
            )
        })
        .collect();
    ordered.sort_by(|a, b| {
        let (a_candidate, a_tt, a_mi) = a;
        let (b_candidate, b_tt, b_mi) = b;
        b_tt.cmp(&a_tt)
            .then_with(|| {
                b_candidate
                    .order_score
                    .partial_cmp(&a_candidate.order_score)
                    .expect("candidate scores are finite")
            })
            .then_with(|| b_mi.cmp(a_mi))
            .then_with(|| a_candidate.move_.cmp(&b_candidate.move_))
    });
    ordered
        .into_iter()
        .map(|(candidate, _, _)| candidate)
        .collect()
}

pub fn order_candidates_root_classic(
    board: &Board,
    candidates: &[Candidate],
    side: Side,
) -> Vec<Candidate> {
    let mut ordered = candidates.to_vec();
    let mut mis = vec![0_i32; BOARD_AREA];
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
