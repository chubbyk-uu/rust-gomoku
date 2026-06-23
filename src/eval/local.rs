//! Local point evaluation and cache maintenance.

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, DSHAPE_SIZE, EMPTY, WHITE};
use crate::eval::caches::EvalCaches;
use crate::patterns::buckets::bucket_for_lines;
use crate::patterns::line::shape_raw_from_board_point_hypothetical;
use crate::patterns::shapes::{ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};
use crate::rules::RuleSet;
use crate::types::Move;

const FOUR_DIRECTIONS: [i32; 4] = [HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP];

pub fn local_backend_name() -> &'static str {
    "python"
}

pub fn compute_direction_shape(board: &Board, x: usize, y: usize, direction: i32, side: i8) -> i32 {
    compute_direction_shape_for_rule(board, x, y, direction, side, RuleSet::Freestyle)
}

pub fn compute_direction_shape_for_rule(
    board: &Board,
    x: usize,
    y: usize,
    direction: i32,
    side: i8,
    rule: RuleSet,
) -> i32 {
    if board.grid_rows()[y][x] != EMPTY {
        return 0;
    }

    let freestyle_shape = rule == RuleSet::Freestyle || side != BLACK;
    shape_raw_from_board_point_hypothetical(
        board.grid_rows(),
        x,
        y,
        direction,
        side,
        freestyle_shape,
    )
    .expect("direction checked by caller")
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
    recompute_point_caches_for_rule(board, caches, x, y, RuleSet::Freestyle)
}

pub fn recompute_point_caches_for_rule(
    board: &mut Board,
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    rule: RuleSet,
) {
    let occupied = board.grid_rows()[y][x] != EMPTY;
    caches.rule_set = rule;

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
            let shape = compute_direction_shape_for_rule(board, x, y, direction, side, rule);
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
    recompute_all_for_rule(board, caches, RuleSet::Freestyle)
}

pub fn recompute_all_for_rule(board: &mut Board, caches: &mut EvalCaches, rule: RuleSet) {
    caches.rule_set = rule;
    let size = board.size();
    for x in 0..size {
        for y in 0..size {
            recompute_point_caches_for_rule(board, caches, x, y, rule);
        }
    }
    rebuild_global_eval_state(board, caches);
    caches.initialized = true;
}

pub fn value_wide_compute(board: &mut Board, caches: &mut EvalCaches, changed: (usize, usize)) {
    value_wide_compute_for_rule(board, caches, changed, RuleSet::Freestyle)
}

pub fn value_wide_compute_for_rule(
    board: &mut Board,
    caches: &mut EvalCaches,
    changed: (usize, usize),
    rule: RuleSet,
) {
    let size = board.size();
    if !caches.initialized || caches.rule_set != rule {
        recompute_all_for_rule(board, caches, rule);
        return;
    }
    caches.rule_set = rule;

    let (cx, cy) = changed;
    let horizontal_flag = 1_u8;
    let vertical_flag = 2_u8;
    let diag_down_flag = 4_u8;
    let diag_up_flag = 8_u8;
    let mut comp = [[0_u8; BOARD_SIZE]; BOARD_SIZE];
    let mut dirty = [0 as Move; BOARD_SIZE * BOARD_SIZE];
    let mut dirty_len = 0_usize;

    {
        let grid = board.grid_rows();
        mark_dirty_cell(&mut comp, &mut dirty, &mut dirty_len, cx, cy, 15);
        let radius = if rule == RuleSet::Renju { 5 } else { 4 };
        mark_cell_neighbors(
            &mut comp,
            &mut dirty,
            &mut dirty_len,
            grid,
            cx,
            cy,
            size,
            radius,
        );
    }

    let old_cell = caches.board_shadow[cx][cy];
    let new_cell = board.grid_rows()[cy][cx];
    update_occupied_transition(caches, cx, cy, old_cell, new_cell);

    for &move_ in &dirty[..dirty_len] {
        let index = move_ as usize;
        let x = index % BOARD_SIZE;
        let y = index / BOARD_SIZE;
        let flags = comp[x][y];
        let cell = board.grid_rows()[y][x];
        remove_empty_bucket_contribution(caches, x, y);
        if cell == EMPTY {
            let mut changed_player = [false; 2];
            if flags & horizontal_flag != 0 {
                merge_changed_player(
                    &mut changed_player,
                    update_direction_cache(board, caches, x, y, HORIZONTAL, rule),
                );
            }
            if flags & vertical_flag != 0 {
                merge_changed_player(
                    &mut changed_player,
                    update_direction_cache(board, caches, x, y, VERTICAL, rule),
                );
            }
            if flags & diag_down_flag != 0 {
                merge_changed_player(
                    &mut changed_player,
                    update_direction_cache(board, caches, x, y, DIAGONAL_DOWN, rule),
                );
            }
            if flags & diag_up_flag != 0 {
                merge_changed_player(
                    &mut changed_player,
                    update_direction_cache(board, caches, x, y, DIAGONAL_UP, rule),
                );
            }
            for (player, changed) in changed_player.into_iter().enumerate() {
                if changed {
                    update_bucket_attack(board, caches, x, y, player);
                }
            }
        } else {
            clear_occupied_point(caches, x, y);
        }
        add_empty_bucket_contribution(board, caches, x, y);
        caches.board_shadow[x][y] = cell;
    }
}

fn mark_dirty_cell(
    comp: &mut [[u8; BOARD_SIZE]; BOARD_SIZE],
    dirty: &mut [Move; BOARD_SIZE * BOARD_SIZE],
    dirty_len: &mut usize,
    x: usize,
    y: usize,
    flags: u8,
) {
    if comp[x][y] == 0 {
        dirty[*dirty_len] = (y * BOARD_SIZE + x) as Move;
        *dirty_len += 1;
    }
    comp[x][y] |= flags;
}

fn mark_cell_neighbors(
    comp: &mut [[u8; BOARD_SIZE]; BOARD_SIZE],
    dirty: &mut [Move; BOARD_SIZE * BOARD_SIZE],
    dirty_len: &mut usize,
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    size: usize,
    radius: isize,
) {
    const H: u8 = 1;
    const V: u8 = 2;
    const DD: u8 = 4;
    const DU: u8 = 8;

    let fixed = x;
    let mut seen = 0_i8;
    for yy in (y + 1)..usize::min(size, y + radius as usize + 1) {
        let value = grid[yy][fixed];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, fixed, yy, H);
    }
    seen = 0;
    for yy in (0..y).rev().take(radius as usize) {
        let value = grid[yy][fixed];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, fixed, yy, H);
    }

    let fixed = y;
    seen = 0;
    for xx in (x + 1)..usize::min(size, x + radius as usize + 1) {
        let value = grid[fixed][xx];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, xx, fixed, V);
    }
    seen = 0;
    for xx in (0..x).rev().take(radius as usize) {
        let value = grid[fixed][xx];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, xx, fixed, V);
    }

    let mut seen = 0_i8;
    let mut xx = x as isize - 1;
    let mut yy = y as isize + 1;
    while xx >= 0 && yy < size as isize && xx >= x as isize - radius && yy <= y as isize + radius {
        let value = grid[yy as usize][xx as usize];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, xx as usize, yy as usize, DD);
        xx -= 1;
        yy += 1;
    }
    seen = 0;
    xx = x as isize + 1;
    yy = y as isize - 1;
    while xx < size as isize && yy >= 0 && xx <= x as isize + radius && yy >= y as isize - radius {
        let value = grid[yy as usize][xx as usize];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, xx as usize, yy as usize, DD);
        xx += 1;
        yy -= 1;
    }

    seen = 0;
    xx = x as isize + 1;
    yy = y as isize + 1;
    while xx < size as isize
        && yy < size as isize
        && xx <= x as isize + radius
        && yy <= y as isize + radius
    {
        let value = grid[yy as usize][xx as usize];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, xx as usize, yy as usize, DU);
        xx += 1;
        yy += 1;
    }
    seen = 0;
    xx = x as isize - 1;
    yy = y as isize - 1;
    while xx >= 0 && yy >= 0 && xx >= x as isize - radius && yy >= y as isize - radius {
        let value = grid[yy as usize][xx as usize];
        if seen == 0 {
            seen = value;
        } else if value != EMPTY && value != seen {
            break;
        }
        mark_dirty_cell(comp, dirty, dirty_len, xx as usize, yy as usize, DU);
        xx -= 1;
        yy -= 1;
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

fn rebuild_global_eval_state(board: &Board, caches: &mut EvalCaches) {
    caches.empty_bucket_counts = [[0; DSHAPE_SIZE]; 2];
    caches.occupied_moves = [0; BOARD_AREA];
    caches.occupied_len = 0;

    for x in 0..board.size() {
        for y in 0..board.size() {
            let cell = board.grid_rows()[y][x];
            caches.board_shadow[x][y] = cell;
            if cell == EMPTY {
                for player in 0..2 {
                    let bucket = caches.value_cache[player][x][y] as usize;
                    debug_assert!(bucket < DSHAPE_SIZE);
                    caches.empty_bucket_counts[player][bucket] += 1;
                }
            } else {
                let move_ = (y * BOARD_SIZE + x) as Move;
                caches.occupied_moves[caches.occupied_len] = move_;
                caches.occupied_len += 1;
            }
        }
    }
}

fn remove_empty_bucket_contribution(caches: &mut EvalCaches, x: usize, y: usize) {
    if caches.board_shadow[x][y] != EMPTY {
        return;
    }
    for player in 0..2 {
        let bucket = caches.value_cache[player][x][y] as usize;
        debug_assert!(bucket < DSHAPE_SIZE);
        debug_assert!(caches.empty_bucket_counts[player][bucket] > 0);
        caches.empty_bucket_counts[player][bucket] -= 1;
    }
}

fn add_empty_bucket_contribution(board: &Board, caches: &mut EvalCaches, x: usize, y: usize) {
    if board.grid_rows()[y][x] != EMPTY {
        return;
    }
    for player in 0..2 {
        let bucket = caches.value_cache[player][x][y] as usize;
        debug_assert!(bucket < DSHAPE_SIZE);
        caches.empty_bucket_counts[player][bucket] += 1;
    }
}

fn update_occupied_transition(
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    old_cell: i8,
    new_cell: i8,
) {
    if old_cell == EMPTY && new_cell != EMPTY {
        add_occupied_move(caches, (y * BOARD_SIZE + x) as Move);
    } else if old_cell != EMPTY && new_cell == EMPTY {
        remove_occupied_move(caches, (y * BOARD_SIZE + x) as Move);
    }
}

fn add_occupied_move(caches: &mut EvalCaches, move_: Move) {
    debug_assert!(caches.occupied_len < BOARD_AREA);
    debug_assert!(!caches.occupied_moves[..caches.occupied_len].contains(&move_));
    caches.occupied_moves[caches.occupied_len] = move_;
    caches.occupied_len += 1;
}

fn remove_occupied_move(caches: &mut EvalCaches, move_: Move) {
    if caches.occupied_len == 0 {
        debug_assert!(false, "occupied move list underflow");
        return;
    }
    let last_index = caches.occupied_len - 1;
    if caches.occupied_moves[last_index] == move_ {
        caches.occupied_len -= 1;
        return;
    }
    let Some(index) = caches.occupied_moves[..last_index]
        .iter()
        .position(|&candidate| candidate == move_)
    else {
        debug_assert!(false, "occupied move missing from list");
        return;
    };
    caches.occupied_moves[index] = caches.occupied_moves[last_index];
    caches.occupied_len -= 1;
}

fn merge_changed_player(accumulator: &mut [bool; 2], changed: [bool; 2]) {
    accumulator[0] |= changed[0];
    accumulator[1] |= changed[1];
}

fn update_direction_cache(
    board: &Board,
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    direction: i32,
    rule: RuleSet,
) -> [bool; 2] {
    let mut changed = [false; 2];
    for (side, player) in [(BLACK, 0_usize), (WHITE, 1_usize)] {
        let new_shape = compute_direction_shape_for_rule(board, x, y, direction, side, rule);
        let direction_index = direction as usize;
        let old = caches.shape_cache[player][x][y][direction_index];
        if old != new_shape {
            if caches.active_snapshot_count > 0 {
                caches.shape_log.push((player, x, y, direction_index, old));
            }
            caches.shape_cache[player][x][y][direction_index] = new_shape;
            changed[player] = true;
        }
    }
    changed
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
