//! Global board evaluation.

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{
    BLACK, BOARD_SIZE, DSHAPE_SIZE, EMPTY, LAST5, NEXT4, NEXT43, NEXT5, WHITE, WIN,
};
use crate::eval::caches::EvalCaches;
use crate::eval::local::value_wide_compute_for_rule;
use crate::patterns::line::Line;
use crate::patterns::{DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};
use crate::rules::{classify_forbidden_move, RuleSet};
use crate::types::Move;

pub(crate) const NEIGHBOR_CHECKS: [(isize, isize, usize); 8] = [
    (-1, -1, 3),
    (-1, 0, 1),
    (-1, 1, 2),
    (0, -1, 0),
    (0, 1, 0),
    (1, -1, 2),
    (1, 0, 1),
    (1, 1, 3),
];

pub fn global_eval_backend_name() -> &'static str {
    "python"
}

pub fn evaluate_board(
    board: &mut Board,
    caches: &mut EvalCaches,
    side: i8,
    opo: usize,
    config: &EngineConfig,
) -> f64 {
    let (total, dgn) = evaluate_board_main(board, caches, side, config);
    evaluate_board_from_main(board, caches, side, opo, config, total, dgn)
}

fn evaluate_board_from_main(
    board: &mut Board,
    caches: &mut EvalCaches,
    side: i8,
    opo: usize,
    config: &EngineConfig,
    total: f64,
    dgn: i32,
) -> f64 {
    if -32_768.0 < total && total < 32_768.0 {
        return total - config.search.drift + f64::from(dgn) * config.search.dgn;
    }

    let winv = ((total + 32_768.0) / 65_536.0).floor();
    if winv <= -f64::from(NEXT5) / 2.0 {
        return -f64::from(WIN);
    }
    if winv >= f64::from(LAST5) / 2.0 {
        return evaluate_last5_branch(board, caches, side, opo, config);
    }
    if winv <= -f64::from(NEXT4) / 2.0 {
        return -f64::from(WIN);
    }
    if winv <= -f64::from(NEXT43) && evaluate_next43_branch(board, caches, side, config) {
        return -f64::from(WIN);
    }
    total - 65_536.0 * winv - config.search.drift + f64::from(dgn) * config.search.dgn
}

pub fn evaluate_board_main(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
) -> (f64, i32) {
    evaluate_board_main_cached(board, caches, side, config)
}

pub fn evaluate_board_main_scan(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
) -> (f64, i32) {
    let player = if side == BLACK { 0 } else { 1 };
    let opponent = 1 - player;
    let opponent_side = -side;
    let grid = board.grid_rows();
    let shape_cache_player = &caches.shape_cache[player];
    let shape_cache_opponent = &caches.shape_cache[opponent];
    let player_values = &caches.value_cache[player];
    let opponent_values = &caches.value_cache[opponent];
    let last_eval = &config.eval_tables.last_eval;
    let next_eval = &config.eval_tables.next_eval;
    let size = board.size();

    let mut offensive = 0.0;
    let mut defensive = 0.0;
    let mut dgn = 0_i32;

    for x in 0..size {
        let player_value_col = &player_values[x];
        let opponent_value_col = &opponent_values[x];
        for y in 0..size {
            let stone = grid[y][x];
            if stone == EMPTY {
                offensive += last_eval[player_value_col[y] as usize];
                defensive += next_eval[opponent_value_col[y] as usize];
                continue;
            }
            if stone == side {
                let cc = blocked_neighbor_count(grid, shape_cache_player, x, y, size);
                if cc <= 1 {
                    dgn -= 5;
                } else if cc - 1 >= 5 {
                    dgn -= cc - 1 - 3;
                }
            } else if stone == opponent_side {
                let cc = blocked_neighbor_count(grid, shape_cache_opponent, x, y, size);
                if cc <= 1 {
                    dgn += 5;
                } else if cc - 1 >= 5 {
                    dgn += cc - 1 - 3;
                }
            }
        }
    }

    (offensive - defensive, dgn)
}

pub fn evaluate_board_main_cached(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
) -> (f64, i32) {
    if !caches.initialized {
        return evaluate_board_main_scan(board, caches, side, config);
    }

    let player = if side == BLACK { 0 } else { 1 };
    let opponent = 1 - player;
    let offensive = bucket_dot_sparse(
        &caches.empty_bucket_counts[player],
        &caches.nonzero_bucket_mask[player],
        &config.eval_tables.last_eval,
    );
    let defensive = bucket_dot_sparse(
        &caches.empty_bucket_counts[opponent],
        &caches.nonzero_bucket_mask[opponent],
        &config.eval_tables.next_eval,
    );
    let dgn = caches.dgn_sum[opponent] - caches.dgn_sum[player];
    #[cfg(debug_assertions)]
    debug_assert_eq!(dgn, evaluate_dgn_from_occupied(board, caches, side));
    (offensive - defensive, dgn)
}

#[inline(always)]
fn bucket_dot_sparse(counts: &[u16; DSHAPE_SIZE], mask: &[u64; 3], values: &[f64]) -> f64 {
    let mut total = 0.0;
    for (word_idx, &word) in mask.iter().enumerate() {
        let mut bits = word;
        let base = word_idx << 6;
        while bits != 0 {
            let bucket = base + bits.trailing_zeros() as usize;
            total += f64::from(counts[bucket]) * values[bucket];
            bits &= bits - 1;
        }
    }
    total
}

#[cfg(debug_assertions)]
fn evaluate_dgn_from_occupied(board: &Board, caches: &EvalCaches, side: i8) -> i32 {
    let player = if side == BLACK { 0 } else { 1 };
    let opponent = 1 - player;
    let opponent_side = -side;
    let grid = board.grid_rows();
    let shape_cache_player = &caches.shape_cache[player];
    let shape_cache_opponent = &caches.shape_cache[opponent];
    let size = board.size();
    let mut dgn = 0_i32;

    for y in 0..size {
        for x in 0..size {
            let stone = grid[y][x];
            if stone == EMPTY {
                continue;
            }
            if stone == side {
                let cc = blocked_neighbor_count(grid, shape_cache_player, x, y, size);
                if cc <= 1 {
                    dgn -= 5;
                } else if cc - 1 >= 5 {
                    dgn -= cc - 1 - 3;
                }
            } else if stone == opponent_side {
                let cc = blocked_neighbor_count(grid, shape_cache_opponent, x, y, size);
                if cc <= 1 {
                    dgn += 5;
                } else if cc - 1 >= 5 {
                    dgn += cc - 1 - 3;
                }
            }
        }
    }

    dgn
}

#[inline(always)]
pub(crate) fn blocked_neighbor_count(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    shape_cache: &[[[i32; 4]; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    size: usize,
) -> i32 {
    let mut cc = 1_i32;
    for &(dx, dy, direction) in &NEIGHBOR_CHECKS {
        let xx = x as isize + dx;
        let yy = y as isize + dy;
        if xx < 0 || yy < 0 || xx >= size as isize || yy >= size as isize {
            cc += 1;
            continue;
        }
        let nx = xx as usize;
        let ny = yy as usize;
        if grid[ny][nx] != EMPTY || ((shape_cache[nx][ny][direction] >> 16) & 15) == 0 {
            cc += 1;
        }
    }
    cc
}

pub fn find_last5_target(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
) -> Option<(usize, usize)> {
    let value_cache = &caches.value_cache[if side == BLACK { 0 } else { 1 }];
    let last_eval = &config.eval_tables.last_eval;
    let threshold = f64::from(LAST5) * 65_536.0 / 2.0;

    for x in 0..board.size() {
        let value_col = &value_cache[x];
        for y in 0..board.size() {
            if board.grid_rows()[y][x] == 0 && last_eval[value_col[y] as usize] >= threshold {
                return Some((x, y));
            }
        }
    }
    None
}

pub fn evaluate_last5_branch(
    board: &mut Board,
    caches: &mut EvalCaches,
    side: i8,
    opo: usize,
    config: &EngineConfig,
) -> f64 {
    let Some((x, y)) = find_last5_target(board, caches, side, config) else {
        return f64::from(WIN);
    };

    // SlowRenju ValueW: if `side` (white) has a five-threat whose only block is
    // a forbidden black move, black cannot block and `side` wins outright. This
    // mirrors `fflag & moveValue1bWide(ci,cj,-c) < 0 -> WIN`. Freestyle and the
    // black-perspective evaluation are unaffected.
    if config.rule_set == RuleSet::Renju && -side == BLACK {
        let block = (y * BOARD_SIZE + x) as Move;
        if classify_forbidden_move(board, block, BLACK, RuleSet::Renju)
            .is_ok_and(|kind| kind.is_forbidden())
        {
            return f64::from(WIN);
        }
    }

    let snapshot = caches.snapshot();
    board.grid_rows_mut()[y][x] = -side;
    let result = {
        value_wide_compute_for_rule(board, caches, (x, y), config.rule_set);
        -evaluate_board(board, caches, -side, 1 - opo, config)
    };
    board.grid_rows_mut()[y][x] = 0;
    caches.restore_snapshot(&snapshot);
    result
}

pub fn evaluate_next43_branch(
    board: &mut Board,
    caches: &mut EvalCaches,
    side: i8,
    config: &EngineConfig,
) -> bool {
    let size = board.size();
    let next_eval = &config.eval_tables.next_eval;
    let opponent_cache = &caches.value_cache[if side == WHITE { 0 } else { 1 }];
    let threshold = f64::from(NEXT43) * 65_536.0 / 2.0;

    for x in 0..size {
        let opponent_value_col = &opponent_cache[x];
        for y in 0..size {
            if board.grid_rows()[y][x] != 0 || next_eval[opponent_value_col[y] as usize] < threshold
            {
                continue;
            }
            let line_specs = [
                (x, HORIZONTAL, y, 1_usize),
                (y, VERTICAL, x, 2_usize),
                (x + y, DIAGONAL_DOWN, y, 3_usize),
                (size - 1 - y + x, DIAGONAL_UP, size - 1 - y, 4_usize),
            ];

            board.grid_rows_mut()[y][x] = -side;
            let mut reply = None;
            for (pivot, direction_value, point_index, direction_id) in line_specs {
                let encoded = Line::from_board(board, pivot, direction_value)
                    .expect("direction id is valid")
                    .b4p(point_index);
                if encoded <= 0 {
                    continue;
                }
                let raw_replies = [ga(encoded), gb(encoded)];
                let reply_count = if encoded >= (1 << 16) { 2 } else { 1 };
                for &raw in &raw_replies[..reply_count] {
                    let Some((rx, ry)) = decode_b4_reply_raw(size, x, y, direction_id, raw) else {
                        continue;
                    };
                    if rx >= size || ry >= size || board.grid_rows()[ry][rx] != 0 {
                        continue;
                    }
                    if config.rule_set == RuleSet::Renju
                        && -side == BLACK
                        && !board.is_winning_move_for_rule(rx, ry, BLACK, RuleSet::Renju)
                    {
                        continue;
                    }
                    reply = Some((rx, ry));
                    break;
                }
                if reply.is_some() {
                    break;
                }
            }
            let Some((rx, ry)) = reply else {
                board.grid_rows_mut()[y][x] = 0;
                continue;
            };
            if config.rule_set == RuleSet::Renju && side == BLACK {
                let block = (ry * BOARD_SIZE + rx) as Move;
                if classify_forbidden_move(board, block, BLACK, RuleSet::Renju)
                    .is_ok_and(|kind| kind.is_forbidden())
                {
                    board.grid_rows_mut()[y][x] = 0;
                    return true;
                }
            }
            board.grid_rows_mut()[ry][rx] = side;
            let has_followup = has_b4p_after_move_for_rule(board, rx, ry, side, config.rule_set);
            board.grid_rows_mut()[ry][rx] = 0;
            board.grid_rows_mut()[y][x] = 0;
            if !has_followup {
                return true;
            }
        }
    }
    false
}

fn has_b4p_after_move_for_rule(board: &Board, x: usize, y: usize, side: i8, rule: RuleSet) -> bool {
    if !has_b4p_after_move(board, x, y) {
        return false;
    }
    if rule != RuleSet::Renju || side != BLACK {
        return true;
    }
    has_renju_black_four_through(board, x, y)
}

fn has_renju_black_four_through(board: &Board, x: usize, y: usize) -> bool {
    const AXES: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];
    let size = board.size() as isize;
    let grid = board.grid_rows();
    for (dx, dy) in AXES {
        for step in -4_isize..=4 {
            if step == 0 {
                continue;
            }
            let cx = x as isize + dx * step;
            let cy = y as isize + dy * step;
            if cx < 0 || cy < 0 || cx >= size || cy >= size {
                continue;
            }
            if grid[cy as usize][cx as usize] != EMPTY {
                continue;
            }
            let mut neg = 0_isize;
            loop {
                let nx = cx - dx * (neg + 1);
                let ny = cy - dy * (neg + 1);
                if nx < 0 || ny < 0 || nx >= size || ny >= size {
                    break;
                }
                if grid[ny as usize][nx as usize] != BLACK {
                    break;
                }
                neg += 1;
            }
            let mut pos = 0_isize;
            loop {
                let nx = cx + dx * (pos + 1);
                let ny = cy + dy * (pos + 1);
                if nx < 0 || ny < 0 || nx >= size || ny >= size {
                    break;
                }
                if grid[ny as usize][nx as usize] != BLACK {
                    break;
                }
                pos += 1;
            }
            if neg + 1 + pos == 5 {
                let anchor_offset = -step;
                if anchor_offset >= -neg && anchor_offset <= pos {
                    return true;
                }
            }
        }
    }
    false
}

fn has_b4p_after_move(board: &Board, x: usize, y: usize) -> bool {
    Line::from_board(board, x, HORIZONTAL)
        .expect("direction id is valid")
        .b4p(y)
        > 0
        || Line::from_board(board, y, VERTICAL)
            .expect("direction id is valid")
            .b4p(x)
            > 0
        || Line::from_board(board, x + y, DIAGONAL_DOWN)
            .expect("direction id is valid")
            .b4p(y)
            > 0
        || Line::from_board(board, board.size() - 1 - y + x, DIAGONAL_UP)
            .expect("direction id is valid")
            .b4p(board.size() - 1 - y)
            > 0
}

fn decode_b4_reply_raw(
    board_size: usize,
    x: usize,
    y: usize,
    direction_index: usize,
    raw: i32,
) -> Option<(usize, usize)> {
    let r1 = usize::try_from(raw).ok()?;
    match direction_index {
        1 => Some((x, r1)),
        2 => Some((r1, y)),
        3 => Some((x.checked_add(y)?.checked_sub(r1)?, r1)),
        4 => {
            let rx = (board_size.checked_sub(1)? + x)
                .checked_sub(y)?
                .checked_sub(r1)?;
            let ry = board_size.checked_sub(1)?.checked_sub(r1)?;
            Some((rx, ry))
        }
        _ => None,
    }
}

fn ga(value: i32) -> i32 {
    value & 0xFF
}

fn gb(value: i32) -> i32 {
    (value >> 8) & 0xFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::config::load_default_config;
    use crate::eval::local::{recompute_all, recompute_all_for_rule, value_wide_compute};
    use crate::rules::RuleSet;

    #[test]
    fn cached_matches_scan_after_direct_grid_mutation_and_snapshot_restore() {
        let config = load_default_config();
        let mut board = Board::new();
        for (idx, (x, y)) in [(7, 7), (8, 7), (7, 8), (8, 8), (6, 7), (9, 8)]
            .into_iter()
            .enumerate()
        {
            board
                .play(
                    xy_to_move(x, y).unwrap(),
                    Some(if idx % 2 == 0 { BLACK } else { WHITE }),
                )
                .unwrap();
        }
        let mut caches = EvalCaches::new();
        recompute_all(&mut board, &mut caches);
        assert_scan_cached_equivalent(&board, &caches, BLACK, &config);
        assert_scan_cached_equivalent(&board, &caches, WHITE, &config);

        let snapshot = caches.snapshot();
        board.grid_rows_mut()[6][6] = WHITE;
        value_wide_compute(&mut board, &mut caches, (6, 6));
        assert_scan_cached_equivalent(&board, &caches, BLACK, &config);
        assert_scan_cached_equivalent(&board, &caches, WHITE, &config);

        board.grid_rows_mut()[6][6] = EMPTY;
        caches.restore_snapshot(&snapshot);
        assert_scan_cached_equivalent(&board, &caches, BLACK, &config);
        assert_scan_cached_equivalent(&board, &caches, WHITE, &config);
    }

    #[test]
    fn renju_cached_eval_matches_scan_with_forbidden_suppression() {
        let mut config = load_default_config();
        config.rule_set = RuleSet::Renju;
        let mut board = Board::new();
        for (x, y, side) in [
            (6, 7, BLACK),
            (8, 7, BLACK),
            (7, 6, BLACK),
            (7, 8, BLACK),
            (0, 0, WHITE),
            (1, 0, WHITE),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }

        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut caches, RuleSet::Renju);
        assert_eq!(
            (caches.value_cache[0][7][7], caches.attack_cache[0][7][7]),
            (0, 0)
        );
        assert_scan_cached_equivalent(&board, &caches, BLACK, &config);
        assert_scan_cached_equivalent(&board, &caches, WHITE, &config);
    }

    #[test]
    fn renju_white_wins_when_black_block_is_forbidden() {
        // White has a broken four on row 7 (5,6,_,8,9); the only block (7,7) is a
        // black double-three forbidden point, so white wins outright in Renju.
        let stones = [
            (5, 7, WHITE),
            (6, 7, WHITE),
            (8, 7, WHITE),
            (9, 7, WHITE),
            (6, 6, BLACK),
            (8, 8, BLACK),
            (6, 8, BLACK),
            (8, 6, BLACK),
        ];
        let block = xy_to_move(7, 7).unwrap();

        let mut renju = load_default_config();
        renju.rule_set = RuleSet::Renju;
        assert!(
            classify_forbidden_move(
                &{
                    let mut b = Board::new();
                    for (x, y, s) in stones {
                        b.grid_rows_mut()[y][x] = s;
                    }
                    b
                },
                block,
                BLACK,
                RuleSet::Renju,
            )
            .unwrap()
            .is_forbidden(),
            "the block point must be forbidden for black"
        );

        let mut board = Board::new();
        for (x, y, s) in stones {
            board.grid_rows_mut()[y][x] = s;
        }
        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut caches, RuleSet::Renju);
        let renju_score = evaluate_board(&mut board, &mut caches, WHITE, 1, &renju);
        assert_eq!(renju_score, f64::from(WIN), "white should win in Renju");

        // In freestyle the block is legal, so white does not win outright.
        let freestyle = load_default_config();
        let mut board = Board::new();
        for (x, y, s) in stones {
            board.grid_rows_mut()[y][x] = s;
        }
        let mut caches = EvalCaches::new();
        recompute_all(&mut board, &mut caches);
        let freestyle_score = evaluate_board(&mut board, &mut caches, WHITE, 1, &freestyle);
        assert!(
            freestyle_score < f64::from(WIN),
            "freestyle white must not win outright, got {freestyle_score}"
        );
    }

    #[test]
    fn renju_next43_wins_when_black_block_is_forbidden() {
        // White to move next (side = BLACK) has a 4-3 at (6, 7):
        //   row 7: B(2,7), W(3,7), W(4,7), W(5,7) -> W(6,7) forces block at (7,7)
        //   col 6: W(6,5), W(6,6) -> W(6,7) forms an open three.
        // Black's only block (7, 7) creates a double-four (col 7 + main diag),
        // which is a legal counter-four in freestyle but forbidden in Renju.
        let stones = [
            (2, 7, BLACK),
            (3, 7, WHITE),
            (4, 7, WHITE),
            (5, 7, WHITE),
            (6, 5, WHITE),
            (6, 6, WHITE),
            (7, 4, BLACK),
            (7, 5, BLACK),
            (7, 6, BLACK),
            (8, 8, BLACK),
            (9, 9, BLACK),
            (10, 10, BLACK),
        ];

        let mut renju = load_default_config();
        renju.rule_set = RuleSet::Renju;
        let mut board_r = Board::new();
        for (x, y, s) in stones {
            board_r.grid_rows_mut()[y][x] = s;
        }
        let mut caches_r = EvalCaches::new();
        recompute_all_for_rule(&mut board_r, &mut caches_r, RuleSet::Renju);
        assert!(
            evaluate_next43_branch(&mut board_r, &mut caches_r, BLACK, &renju),
            "Renju: black cannot play forbidden double-four block (7,7)"
        );
        assert_eq!(
            evaluate_board(&mut board_r, &mut caches_r, BLACK, 0, &renju),
            -f64::from(WIN)
        );

        let freestyle = load_default_config();
        let mut board_f = Board::new();
        for (x, y, s) in stones {
            board_f.grid_rows_mut()[y][x] = s;
        }
        let mut caches_f = EvalCaches::new();
        recompute_all(&mut board_f, &mut caches_f);
        assert!(
            !evaluate_next43_branch(&mut board_f, &mut caches_f, BLACK, &freestyle),
            "Freestyle: black legally counter-fours at (7,7)"
        );
    }

    #[test]
    fn renju_next43_ignores_fake_black_overline_counter_four() {
        // White has a 4-3 at (6, 7): B(2,7), W(3,7), W(4,7), W(5,7) + open-3 W(6,5), W(6,6).
        // Forced block is (7, 7). On col 7, Black has W(7,2), B(7,3), B(7,4), B(7,5), _(7,6),
        // _(7,7), B(7,8), B(7,9)? Wait: B(7,3..=5) + B(7,7) + B(7,8) makes 6 when filling (7,6)!
        // Check: W(7,2), B(7,3), B(7,4), B(7,5), _(7,6), [B(7,7)], B(7,8), W(7,9).
        // Before (7,7): col 7 has 3 blacks (3,4,5) + gap (6,7) + 1 black (8), not a four.
        // After Black plays (7,7): col 7 has W(2), B(3), B(4), B(5), _(6), B(7), B(8), W(9).
        // Raw b4p sees B(4), B(5), _(6), B(7), B(8) -> four!
        // Filling the only gap (7,6) gives B(3..=8) = 6 stones (overline!), so NOT a real Renju four!
        let stones = [
            (2, 7, BLACK),
            (3, 7, WHITE),
            (4, 7, WHITE),
            (5, 7, WHITE),
            (6, 5, WHITE),
            (6, 6, WHITE),
            (7, 2, WHITE),
            (7, 3, BLACK),
            (7, 4, BLACK),
            (7, 5, BLACK),
            (7, 8, BLACK),
            (7, 9, WHITE),
        ];

        let mut renju = load_default_config();
        renju.rule_set = RuleSet::Renju;
        let mut board_r = Board::new();
        for (x, y, s) in stones {
            board_r.grid_rows_mut()[y][x] = s;
        }
        assert_eq!(
            classify_forbidden_move(&board_r, xy_to_move(7, 7).unwrap(), BLACK, RuleSet::Renju)
                .unwrap(),
            crate::rules::ForbiddenKind::None,
            "(7,7) itself must be legal for black"
        );
        let mut caches_r = EvalCaches::new();
        recompute_all_for_rule(&mut board_r, &mut caches_r, RuleSet::Renju);
        assert!(
            evaluate_next43_branch(&mut board_r, &mut caches_r, BLACK, &renju),
            "Renju: fake overline counter-four through (7,7) must not refute white's 4-3"
        );

        let freestyle = load_default_config();
        let mut board_f = Board::new();
        for (x, y, s) in stones {
            board_f.grid_rows_mut()[y][x] = s;
        }
        let mut caches_f = EvalCaches::new();
        recompute_all(&mut board_f, &mut caches_f);
        assert!(
            !evaluate_next43_branch(&mut board_f, &mut caches_f, BLACK, &freestyle),
            "Freestyle: overline completion at (7,6) is a valid counter-four"
        );
    }

    fn assert_scan_cached_equivalent(
        board: &Board,
        caches: &EvalCaches,
        side: i8,
        config: &EngineConfig,
    ) {
        let (scan_total, scan_dgn) = evaluate_board_main_scan(board, caches, side, config);
        let (cached_total, cached_dgn) = evaluate_board_main_cached(board, caches, side, config);
        assert_eq!(cached_dgn, scan_dgn);
        let tolerance = 1e-9_f64.max(scan_total.abs() * 1e-14);
        assert!((cached_total - scan_total).abs() <= tolerance);

        let mut scan_board = board.clone();
        let mut scan_caches = caches.clone();
        let scan_score = evaluate_board_from_main(
            &mut scan_board,
            &mut scan_caches,
            side,
            0,
            config,
            scan_total,
            scan_dgn,
        );
        let mut cached_board = board.clone();
        let mut cached_caches = caches.clone();
        let cached_score = evaluate_board_from_main(
            &mut cached_board,
            &mut cached_caches,
            side,
            0,
            config,
            cached_total,
            cached_dgn,
        );
        assert_eq!(cached_score as i32, scan_score as i32);
    }
}
