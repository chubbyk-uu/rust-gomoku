//! Move ordering helpers aligned with the classic reference.

use crate::board::{move_to_xy, Board};
use crate::constants::BOARD_AREA;
use crate::search::movegen::Candidate;
use crate::types::{Move, Side};

pub(crate) const ORDERING_MAX_PLY: usize = BOARD_AREA + 1;
pub(crate) const NO_KILLER_MOVE: Move = Move::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FastOrderingStats {
    pub killer_hits: usize,
    pub history_hits: usize,
}

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

pub(crate) fn order_candidates_fast_history_owned(
    board: &Board,
    mut candidates: Vec<Candidate>,
    side: Side,
    tt_best_move: Option<Move>,
    ply: usize,
    hostile_threat: bool,
    killer_moves: &[[Move; 2]; ORDERING_MAX_PLY],
    history_scores: &[[i32; BOARD_AREA]; 2],
    killer_bonus: i32,
    history_bonus_cap: i32,
) -> (Vec<Candidate>, FastOrderingStats) {
    let stats = order_candidates_fast_history_in_place(
        board,
        &mut candidates,
        side,
        tt_best_move,
        ply,
        hostile_threat,
        killer_moves,
        history_scores,
        killer_bonus,
        history_bonus_cap,
    );
    (candidates, stats)
}

pub(crate) fn is_quiet_ordering_candidate(candidate: Candidate, hostile_threat: bool) -> bool {
    !hostile_threat && candidate.self_attack < 4 && candidate.opp_attack < 4
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

fn side_index(side: Side) -> usize {
    usize::from(side != 1)
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

fn killer_rank(killers: [Move; 2], move_: Move) -> i32 {
    if killers[0] == move_ {
        2
    } else if killers[1] == move_ {
        1
    } else {
        0
    }
}

fn fast_primary_equal(a: Candidate, b: Candidate, tt_best_move: Option<Move>) -> bool {
    (tt_best_move == Some(a.move_)) == (tt_best_move == Some(b.move_))
        && a.order_score == b.order_score
}

#[allow(clippy::too_many_arguments)]
fn order_candidates_fast_history_in_place(
    board: &Board,
    result: &mut [Candidate],
    side: Side,
    tt_best_move: Option<Move>,
    ply: usize,
    hostile_threat: bool,
    killer_moves: &[[Move; 2]; ORDERING_MAX_PLY],
    history_scores: &[[i32; BOARD_AREA]; 2],
    killer_bonus: i32,
    history_bonus_cap: i32,
) -> FastOrderingStats {
    let ply_index = ply.min(ORDERING_MAX_PLY - 1);
    let killers = killer_moves[ply_index];
    let history = &history_scores[side_index(side)];
    let mut killer_ranks = [0_i32; BOARD_AREA];
    let mut history_values = [0_i32; BOARD_AREA];
    let mut stats = FastOrderingStats::default();
    for candidate in result.iter().copied() {
        let index = candidate.move_ as usize;
        let quiet = is_quiet_ordering_candidate(candidate, hostile_threat);
        if quiet {
            let rank = killer_rank(killers, candidate.move_);
            if rank > 0 {
                stats.killer_hits += 1;
            }
            let history_score = history[index].clamp(0, history_bonus_cap.max(0));
            if history_score > 0 {
                stats.history_hits += 1;
            }
            killer_ranks[index] = rank * killer_bonus.max(0);
            history_values[index] = history_score;
        }
    }

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
        while end < result.len() && fast_primary_equal(result[start], result[end], tt_best_move) {
            end += 1;
        }
        if end - start > 1 {
            result[start..end].sort_unstable_by(|a, b| {
                let a_index = a.move_ as usize;
                let b_index = b.move_ as usize;
                let a_mi = cached_getmi(board, a.move_, side, &mut mi_cache);
                let b_mi = cached_getmi(board, b.move_, side, &mut mi_cache);
                killer_ranks[b_index]
                    .cmp(&killer_ranks[a_index])
                    .then_with(|| history_values[b_index].cmp(&history_values[a_index]))
                    .then_with(|| b_mi.cmp(&a_mi))
                    .then_with(|| a.move_.cmp(&b.move_))
            });
        }
        start = end;
    }

    stats
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
    fn fast_history_ordering_uses_quiet_bonus_inside_static_ties() {
        let board = Board::new();
        let tt_move = xy_to_move(11, 11).unwrap();
        let killer_move = xy_to_move(4, 4).unwrap();
        let history_move = xy_to_move(5, 5).unwrap();
        let tactical_move = xy_to_move(6, 6).unwrap();
        let quiet_static_move = xy_to_move(7, 7).unwrap();
        let mut killers = [[NO_KILLER_MOVE; 2]; ORDERING_MAX_PLY];
        killers[3][0] = killer_move;
        let mut history = [[0_i32; BOARD_AREA]; 2];
        history[0][history_move as usize] = 9_000;

        let (ordered, stats) = order_candidates_fast_history_owned(
            &board,
            vec![
                Candidate {
                    move_: quiet_static_move,
                    order_score: 1.0,
                    self_attack: 0,
                    opp_attack: 0,
                },
                Candidate {
                    move_: history_move,
                    order_score: 1.0,
                    self_attack: 0,
                    opp_attack: 0,
                },
                Candidate {
                    move_: killer_move,
                    order_score: 1.0,
                    self_attack: 0,
                    opp_attack: 0,
                },
                Candidate {
                    move_: tt_move,
                    order_score: 0.0,
                    self_attack: 0,
                    opp_attack: 0,
                },
                Candidate {
                    move_: tactical_move,
                    order_score: 100.0,
                    self_attack: 4,
                    opp_attack: 0,
                },
            ],
            1,
            Some(tt_move),
            3,
            false,
            &killers,
            &history,
            20_000,
            1_024,
        );
        let moves: Vec<_> = ordered.iter().map(|candidate| candidate.move_).collect();
        assert_eq!(
            moves,
            vec![
                tt_move,
                tactical_move,
                killer_move,
                history_move,
                quiet_static_move,
            ]
        );
        assert_eq!(stats.killer_hits, 1);
        assert_eq!(stats.history_hits, 1);
    }

    #[test]
    fn hostile_threat_disables_quiet_history_reordering() {
        let board = Board::new();
        let killer_move = xy_to_move(4, 4).unwrap();
        let static_move = xy_to_move(5, 5).unwrap();
        let mut killers = [[NO_KILLER_MOVE; 2]; ORDERING_MAX_PLY];
        killers[2][0] = killer_move;
        let history = [[0_i32; BOARD_AREA]; 2];

        let (ordered, stats) = order_candidates_fast_history_owned(
            &board,
            vec![candidate(4, 4, 1.0, 0), candidate(5, 5, 10.0, 0)],
            1,
            None,
            2,
            true,
            &killers,
            &history,
            20_000,
            1_024,
        );
        let moves: Vec<_> = ordered.iter().map(|candidate| candidate.move_).collect();
        assert_eq!(moves, vec![static_move, killer_move]);
        assert_eq!(stats.killer_hits, 0);
        assert_eq!(stats.history_hits, 0);
    }
}
