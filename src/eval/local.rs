//! Local point evaluation and cache maintenance.

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{BLACK, BOARD_SIZE, EMPTY, WHITE};
use crate::eval::caches::EvalCaches;
use crate::patterns::buckets::bucket_for_lines;
use crate::patterns::line::shape_raw_from_board_python;
use crate::patterns::shapes::{ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};

const FOUR_DIRECTIONS: [i32; 4] = [HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP];

pub fn local_backend_name() -> &'static str {
    "python"
}

pub fn compute_direction_shape(
    board: &mut Board,
    x: usize,
    y: usize,
    direction: i32,
    side: i8,
) -> i32 {
    if board.grid_rows()[y][x] != EMPTY {
        return 0;
    }

    board.grid_rows_mut()[y][x] = side;
    let result = {
        let (pivot, point_index) = pivot_and_point_index(x, y, direction);
        shape_raw_from_board_python(board, pivot, direction, point_index, true)
            .expect("direction checked in pivot_and_point_index")
    };
    board.grid_rows_mut()[y][x] = EMPTY;
    result
}

pub fn compute_bucket_and_attack(direction_shapes: (i32, i32, i32, i32)) -> (i32, i32) {
    let mut attack = 0;
    let mut lines = [0_i32; 4];

    for (idx, shape) in [
        direction_shapes.0,
        direction_shapes.1,
        direction_shapes.2,
        direction_shapes.3,
    ]
    .into_iter()
    .enumerate()
    {
        let label = (shape >> 16) & 0xF;
        let aux = shape & 0xF;
        lines[idx] = label % ShapeLabel::L6 as i32;

        if label == ShapeLabel::L3 as i32 || label == ShapeLabel::L3B as i32 {
            attack = attack.max(3);
        } else if label == ShapeLabel::L4S as i32 {
            attack = attack.max(4);
            if aux >= 2 {
                lines[idx] = 8;
            }
        } else if label == ShapeLabel::L5 as i32 {
            attack = 6;
        } else if label == ShapeLabel::L4 as i32 {
            attack = attack.max(5);
        }
    }

    if lines[0] < lines[1] {
        lines.swap(0, 1);
    }
    if lines[2] < lines[3] {
        lines.swap(2, 3);
    }

    let (top1, top2) = if lines[1] >= lines[2] {
        (lines[0], lines[1])
    } else if lines[3] >= lines[0] {
        (lines[2], lines[3])
    } else if lines[0] >= lines[2] {
        (lines[0], lines[2])
    } else {
        (lines[2], lines[0])
    };

    let bucket = bucket_for_lines(top1, top2).expect("normalized line strengths are valid");
    (bucket, attack)
}

pub fn recompute_point_caches(board: &mut Board, caches: &mut EvalCaches, x: usize, y: usize) {
    let occupied = board.grid_rows()[y][x] != EMPTY;

    if occupied {
        for player in 0..2 {
            if caches.active_snapshot_count > 0 {
                let old_bucket = caches.value_cache[player][x][y];
                let old_attack = caches.attack_cache[player][x][y];
                if old_bucket != 0 || old_attack != 0 {
                    caches
                        .value_log
                        .push((player, x, y, old_bucket, old_attack));
                }
            }

            caches.value_cache[player][x][y] = 0;
            caches.attack_cache[player][x][y] = 0;

            for direction in 0..4 {
                let old = caches.shape_cache[player][x][y][direction];
                if old != 0 {
                    if caches.active_snapshot_count > 0 {
                        caches.shape_log.push((player, x, y, direction, old));
                    }
                    caches.shape_cache[player][x][y][direction] = 0;
                }
            }
        }
        return;
    }

    for (side, player) in [(BLACK, 0_usize), (WHITE, 1_usize)] {
        let mut shapes = [0_i32; 4];
        for direction in FOUR_DIRECTIONS {
            let shape = compute_direction_shape(board, x, y, direction, side);
            let direction_index = direction as usize;
            let old = caches.shape_cache[player][x][y][direction_index];
            if old != shape {
                if caches.active_snapshot_count > 0 {
                    caches.shape_log.push((player, x, y, direction_index, old));
                }
                caches.shape_cache[player][x][y][direction_index] = shape;
            }
            shapes[direction_index] = shape;
        }

        let (bucket, attack) =
            compute_bucket_and_attack((shapes[0], shapes[1], shapes[2], shapes[3]));
        let old_bucket = caches.value_cache[player][x][y];
        let old_attack = caches.attack_cache[player][x][y];
        if caches.active_snapshot_count > 0 && (old_bucket != bucket || old_attack != attack) {
            caches
                .value_log
                .push((player, x, y, old_bucket, old_attack));
        }
        caches.value_cache[player][x][y] = bucket;
        caches.attack_cache[player][x][y] = attack;
    }
}

pub fn recompute_all(board: &mut Board, caches: &mut EvalCaches) {
    let size = board.size();
    for x in 0..size {
        for y in 0..size {
            recompute_point_caches(board, caches, x, y);
        }
    }
    copy_board_into_shadow(board, caches);
    caches.initialized = true;
}

pub fn value_wide_compute(board: &mut Board, caches: &mut EvalCaches) {
    let size = board.size();
    if !caches.initialized {
        let mut has_stone = false;
        for x in 0..size {
            for y in 0..size {
                if board.grid_rows()[y][x] != EMPTY {
                    has_stone = true;
                    break;
                }
            }
            if has_stone {
                break;
            }
        }
        if has_stone {
            recompute_all(board, caches);
            return;
        }
        caches.initialized = true;
    }

    let ar = 4_isize;
    let horizontal_flag = 1_u8;
    let vertical_flag = 2_u8;
    let diag_down_flag = 4_u8;
    let diag_up_flag = 8_u8;
    let mut comp = vec![vec![0_u8; size]; size];

    {
        let grid = board.grid_rows();
        let shadow = &caches.board_shadow;
        for x in 0..size {
            for y in 0..size {
                if shadow[x][y] != grid[y][x] {
                    comp[x][y] = 15;

                    let fixed = x;
                    let mut seen = 0_i8;
                    for yy in (y + 1)..usize::min(size, y + ar as usize + 1) {
                        let value = grid[yy][fixed];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[fixed][yy] |= horizontal_flag;
                    }
                    seen = 0;
                    for yy in (0..y).rev().take(ar as usize) {
                        let value = grid[yy][fixed];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[fixed][yy] |= horizontal_flag;
                    }

                    let fixed = y;
                    seen = 0;
                    for xx in (x + 1)..usize::min(size, x + ar as usize + 1) {
                        let value = grid[fixed][xx];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[xx][fixed] |= vertical_flag;
                    }
                    seen = 0;
                    for xx in (0..x).rev().take(ar as usize) {
                        let value = grid[fixed][xx];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[xx][fixed] |= vertical_flag;
                    }

                    let mut seen = 0_i8;
                    let mut xx = x as isize - 1;
                    let mut yy = y as isize + 1;
                    while xx >= 0
                        && yy < size as isize
                        && xx >= x as isize - ar
                        && yy <= y as isize + ar
                    {
                        let value = grid[yy as usize][xx as usize];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[xx as usize][yy as usize] |= diag_down_flag;
                        xx -= 1;
                        yy += 1;
                    }

                    seen = 0;
                    xx = x as isize + 1;
                    yy = y as isize - 1;
                    while xx < size as isize
                        && yy >= 0
                        && xx <= x as isize + ar
                        && yy >= y as isize - ar
                    {
                        let value = grid[yy as usize][xx as usize];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[xx as usize][yy as usize] |= diag_down_flag;
                        xx += 1;
                        yy -= 1;
                    }

                    seen = 0;
                    xx = x as isize + 1;
                    yy = y as isize + 1;
                    while xx < size as isize
                        && yy < size as isize
                        && xx <= x as isize + ar
                        && yy <= y as isize + ar
                    {
                        let value = grid[yy as usize][xx as usize];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[xx as usize][yy as usize] |= diag_up_flag;
                        xx += 1;
                        yy += 1;
                    }

                    seen = 0;
                    xx = x as isize - 1;
                    yy = y as isize - 1;
                    while xx >= 0 && yy >= 0 && xx >= x as isize - ar && yy >= y as isize - ar {
                        let value = grid[yy as usize][xx as usize];
                        if seen == 0 {
                            seen = value;
                        } else if value != EMPTY && value != seen {
                            break;
                        }
                        comp[xx as usize][yy as usize] |= diag_up_flag;
                        xx -= 1;
                        yy -= 1;
                    }
                }
            }
        }
    }

    for x in 0..size {
        for y in 0..size {
            let cell = board.grid_rows()[y][x];
            caches.board_shadow[x][y] = cell;
            let flags = comp[x][y];

            if flags != 0 && cell == EMPTY {
                if flags & horizontal_flag != 0 {
                    update_direction_cache(board, caches, x, y, HORIZONTAL);
                }
                if flags & vertical_flag != 0 {
                    update_direction_cache(board, caches, x, y, VERTICAL);
                }
                if flags & diag_down_flag != 0 {
                    update_direction_cache(board, caches, x, y, DIAGONAL_DOWN);
                }
                if flags & diag_up_flag != 0 {
                    update_direction_cache(board, caches, x, y, DIAGONAL_UP);
                }

                update_bucket_attack(board, caches, x, y, 0);
                update_bucket_attack(board, caches, x, y, 1);
            } else if cell != EMPTY {
                clear_occupied_point(caches, x, y);
            }
        }
    }
}

pub fn move_value(caches: &EvalCaches, x: usize, y: usize, side: i8, config: &EngineConfig) -> f64 {
    let player = side_index(side);
    let opponent = 1 - player;
    config.eval_tables.attack_value[caches.value_cache[player][x][y] as usize]
        + config.eval_tables.defend_value[caches.value_cache[opponent][x][y] as usize]
}

pub fn eval_value_next(
    caches: &EvalCaches,
    x: usize,
    y: usize,
    side: i8,
    config: &EngineConfig,
) -> f64 {
    let player = side_index(side);
    config.eval_tables.next_eval[caches.value_cache[player][x][y] as usize]
}

pub fn eval_value_last(
    caches: &EvalCaches,
    x: usize,
    y: usize,
    side: i8,
    config: &EngineConfig,
) -> f64 {
    let player = side_index(side);
    config.eval_tables.last_eval[caches.value_cache[player][x][y] as usize]
}

pub fn attack_level(caches: &EvalCaches, x: usize, y: usize, side: i8) -> i32 {
    caches.attack_cache[side_index(side)][x][y]
}

fn side_index(side: i8) -> usize {
    match side {
        BLACK => 0,
        WHITE => 1,
        _ => panic!("invalid side: {side}"),
    }
}

fn pivot_and_point_index(x: usize, y: usize, direction: i32) -> (usize, usize) {
    match direction {
        HORIZONTAL => (x, y),
        VERTICAL => (y, x),
        DIAGONAL_DOWN => (x + y, y),
        DIAGONAL_UP => (BOARD_SIZE - 1 - y + x, BOARD_SIZE - 1 - y),
        _ => panic!("invalid direction: {direction}"),
    }
}

fn copy_board_into_shadow(board: &Board, caches: &mut EvalCaches) {
    for x in 0..board.size() {
        for y in 0..board.size() {
            caches.board_shadow[x][y] = board.grid_rows()[y][x];
        }
    }
}

fn update_direction_cache(
    board: &mut Board,
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    direction: i32,
) {
    for (side, player) in [(BLACK, 0_usize), (WHITE, 1_usize)] {
        let new_shape = compute_direction_shape(board, x, y, direction, side);
        let direction_index = direction as usize;
        let old = caches.shape_cache[player][x][y][direction_index];
        if old != new_shape {
            if caches.active_snapshot_count > 0 {
                caches.shape_log.push((player, x, y, direction_index, old));
            }
            caches.shape_cache[player][x][y][direction_index] = new_shape;
        }
    }
}

fn update_bucket_attack(board: &Board, caches: &mut EvalCaches, x: usize, y: usize, player: usize) {
    if board.grid_rows()[y][x] != EMPTY {
        return;
    }
    let shapes = &caches.shape_cache[player][x][y];
    let (bucket, attack) = compute_bucket_and_attack((shapes[0], shapes[1], shapes[2], shapes[3]));
    let old_bucket = caches.value_cache[player][x][y];
    let old_attack = caches.attack_cache[player][x][y];
    if caches.active_snapshot_count > 0 && (old_bucket != bucket || old_attack != attack) {
        caches
            .value_log
            .push((player, x, y, old_bucket, old_attack));
    }
    caches.value_cache[player][x][y] = bucket;
    caches.attack_cache[player][x][y] = attack;
}

fn clear_occupied_point(caches: &mut EvalCaches, x: usize, y: usize) {
    for player in 0..2 {
        let old_bucket = caches.value_cache[player][x][y];
        let old_attack = caches.attack_cache[player][x][y];
        if caches.active_snapshot_count > 0 && (old_bucket != 0 || old_attack != 0) {
            caches
                .value_log
                .push((player, x, y, old_bucket, old_attack));
        }
        caches.value_cache[player][x][y] = 0;
        caches.attack_cache[player][x][y] = 0;

        for direction in 0..4 {
            let old = caches.shape_cache[player][x][y][direction];
            if old != 0 {
                if caches.active_snapshot_count > 0 {
                    caches.shape_log.push((player, x, y, direction, old));
                }
                caches.shape_cache[player][x][y][direction] = 0;
            }
        }
    }
}
