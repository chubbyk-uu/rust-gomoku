//! Global board evaluation.

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{BLACK, LAST5, NEXT4, NEXT43, NEXT5, WHITE, WIN};
use crate::eval::caches::EvalCaches;
use crate::eval::local::value_wide_compute;
use crate::patterns::line::Line;
use crate::patterns::{DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};

const DIR_MAP: [usize; 9] = [3, 1, 2, 0, 0, 0, 2, 1, 3];

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
    let player = if side == BLACK { 0 } else { 1 };
    let opponent = 1 - player;
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
            if stone == side {
                let mut cc = 1_i32;
                for k in 0..9 {
                    if k == 4 {
                        continue;
                    }
                    let xx = x as isize - 1 + (k / 3) as isize;
                    let yy = y as isize - 1 + (k % 3) as isize;
                    if xx < 0
                        || yy < 0
                        || xx >= size as isize
                        || yy >= size as isize
                        || grid[yy as usize][xx as usize] != 0
                    {
                        cc += 1;
                    } else if ((shape_cache_player[xx as usize][yy as usize][DIR_MAP[k]] >> 16)
                        & 15)
                        == 0
                    {
                        cc += 1;
                    }
                }
                if cc <= 1 {
                    dgn -= 5;
                } else if cc - 1 >= 5 {
                    dgn -= cc - 1 - 3;
                }
            } else if stone == -side {
                let mut cc = 1_i32;
                for k in 0..9 {
                    if k == 4 {
                        continue;
                    }
                    let xx = x as isize - 1 + (k / 3) as isize;
                    let yy = y as isize - 1 + (k % 3) as isize;
                    if xx < 0
                        || yy < 0
                        || xx >= size as isize
                        || yy >= size as isize
                        || grid[yy as usize][xx as usize] != 0
                    {
                        cc += 1;
                    } else if ((shape_cache_opponent[xx as usize][yy as usize][DIR_MAP[k]] >> 16)
                        & 15)
                        == 0
                    {
                        cc += 1;
                    }
                }
                if cc <= 1 {
                    dgn += 5;
                } else if cc - 1 >= 5 {
                    dgn += cc - 1 - 3;
                }
            } else {
                offensive += last_eval[player_value_col[y] as usize];
                defensive += next_eval[opponent_value_col[y] as usize];
            }
        }
    }

    (offensive - defensive, dgn)
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

    let snapshot = caches.snapshot();
    board.grid_rows_mut()[y][x] = -side;
    let result = {
        value_wide_compute(board, caches);
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
            let mut encoded = 0_i32;
            let mut direction = 0_usize;
            for (pivot, direction_value, point_index, direction_id) in line_specs {
                encoded = Line::from_board(board, pivot, direction_value)
                    .expect("direction id is valid")
                    .b4p(point_index);
                if encoded > 0 {
                    direction = direction_id;
                    break;
                }
            }
            if direction == 0 {
                board.grid_rows_mut()[y][x] = 0;
                continue;
            }
            let Some((rx, ry)) = decode_b4_reply(size, x, y, direction, encoded) else {
                board.grid_rows_mut()[y][x] = 0;
                continue;
            };
            if rx >= size || ry >= size || board.grid_rows()[ry][rx] != 0 {
                board.grid_rows_mut()[y][x] = 0;
                continue;
            }
            board.grid_rows_mut()[ry][rx] = side;
            let has_followup = has_b4p_after_move(board, rx, ry);
            board.grid_rows_mut()[ry][rx] = 0;
            board.grid_rows_mut()[y][x] = 0;
            if !has_followup {
                return true;
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

fn decode_b4_reply(
    board_size: usize,
    x: usize,
    y: usize,
    direction_index: usize,
    encoded: i32,
) -> Option<(usize, usize)> {
    let r1 = ga(encoded) as usize;
    match direction_index {
        1 => Some((x, r1)),
        2 => Some((r1, y)),
        3 => Some((x + y - r1, r1)),
        4 => Some((board_size - 1 + x - y - r1, board_size - 1 - r1)),
        _ => None,
    }
}

fn ga(value: i32) -> i32 {
    value & 0xFF
}
