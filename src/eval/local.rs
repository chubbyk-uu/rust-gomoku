//! Local point evaluation and cache maintenance.

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, DSHAPE_SIZE, EMPTY, WHITE};
use crate::eval::caches::EvalCaches;
use crate::patterns::buckets::bucket_for_lines;
use crate::patterns::line::shape_raw_from_board_point_hypothetical;
use crate::patterns::shapes::{ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};
use crate::rules::{classify_forbidden_move, RuleSet};
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
    compute_bucket_and_attack_raw(direction_shapes)
}

fn compute_bucket_and_attack_for_rule(
    board: &Board,
    x: usize,
    y: usize,
    side: i8,
    rule: RuleSet,
    direction_shapes: (i32, i32, i32, i32),
) -> (i32, i32) {
    let counts = compute_bucket_attack_and_counts(direction_shapes);
    if rule == RuleSet::Renju && side == BLACK && counts.exact_fives == 0 {
        if counts.fours >= 2 || counts.overlines > 0 {
            return (0, 0);
        }
        if counts.open_threes >= 2
            && classify_forbidden_move(board, (y * BOARD_SIZE + x) as Move, side, rule)
                .is_ok_and(|kind| kind.is_forbidden())
        {
            return (0, 0);
        }
    }
    (counts.bucket, counts.attack)
}

fn compute_bucket_and_attack_raw(direction_shapes: (i32, i32, i32, i32)) -> (i32, i32) {
    let counts = compute_bucket_attack_and_counts(direction_shapes);
    (counts.bucket, counts.attack)
}

struct ShapeCounts {
    bucket: i32,
    attack: i32,
    exact_fives: i32,
    open_threes: i32,
    fours: i32,
    overlines: i32,
}

fn compute_bucket_attack_and_counts(direction_shapes: (i32, i32, i32, i32)) -> ShapeCounts {
    let mut attack = 0;
    let mut exact_fives = 0;
    let mut open_threes = 0;
    let mut fours = 0;
    let mut overlines = 0;
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
            open_threes += 1;
            attack = attack.max(3);
        } else if label == ShapeLabel::L4S as i32 {
            fours += aux;
            attack = attack.max(4);
            if aux >= 2 {
                lines[idx] = 8;
            }
        } else if label == ShapeLabel::L5 as i32 {
            attack = 6;
            exact_fives += 1;
        } else if label == ShapeLabel::L4 as i32 {
            fours += 1;
            attack = attack.max(5);
        } else if label == ShapeLabel::L6 as i32 {
            overlines += 1;
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
    ShapeCounts {
        bucket,
        attack,
        exact_fives,
        open_threes,
        fours,
        overlines,
    }
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

        let (bucket, attack) = compute_bucket_and_attack_for_rule(
            board,
            x,
            y,
            side,
            rule,
            (shapes[0], shapes[1], shapes[2], shapes[3]),
        );
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
                    update_bucket_attack(board, caches, x, y, player, rule);
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

fn update_bucket_attack(
    board: &Board,
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    player: usize,
    rule: RuleSet,
) {
    if board.grid_rows()[y][x] != EMPTY {
        return;
    }
    let shapes = &caches.shape_cache[player][x][y];
    let side = if player == 0 { BLACK } else { WHITE };
    let (bucket, attack) = compute_bucket_and_attack_for_rule(
        board,
        x,
        y,
        side,
        rule,
        (shapes[0], shapes[1], shapes[2], shapes[3]),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn board_from_stones(stones: &[(usize, usize, i8)]) -> Board {
        let mut board = Board::new();
        for &(x, y, side) in stones {
            board.grid_rows_mut()[y][x] = side;
        }
        board
    }

    fn recompute_point_for_rule(
        stones: &[(usize, usize, i8)],
        point: (usize, usize),
        rule: RuleSet,
    ) -> EvalCaches {
        let mut board = board_from_stones(stones);
        let mut caches = EvalCaches::new();
        recompute_point_caches_for_rule(&mut board, &mut caches, point.0, point.1, rule);
        caches
    }

    #[test]
    fn renju_black_eval_suppresses_forbidden_points() {
        let cases = [
            (
                "double_four",
                &[
                    (5, 7, BLACK),
                    (6, 7, BLACK),
                    (8, 7, BLACK),
                    (7, 5, BLACK),
                    (7, 6, BLACK),
                    (7, 8, BLACK),
                ][..],
                (7, 7),
            ),
            (
                "double_three",
                &[(6, 7, BLACK), (8, 7, BLACK), (7, 6, BLACK), (7, 8, BLACK)][..],
                (7, 7),
            ),
            (
                "overline",
                &[
                    (4, 7, BLACK),
                    (5, 7, BLACK),
                    (6, 7, BLACK),
                    (7, 7, BLACK),
                    (8, 7, BLACK),
                ][..],
                (9, 7),
            ),
        ];

        for (name, stones, (x, y)) in cases {
            let freestyle = recompute_point_for_rule(stones, (x, y), RuleSet::Freestyle);
            let renju = recompute_point_for_rule(stones, (x, y), RuleSet::Renju);
            assert!(
                freestyle.value_cache[0][x][y] > 0 || freestyle.attack_cache[0][x][y] > 0,
                "{name}: freestyle should still value the tactical point"
            );
            assert_eq!(
                (renju.value_cache[0][x][y], renju.attack_cache[0][x][y]),
                (0, 0),
                "{name}: Renju black forbidden point must be suppressed"
            );
        }
    }

    #[test]
    fn renju_black_eval_keeps_exact_five_priority() {
        let stones = [
            (3, 7, BLACK),
            (4, 7, BLACK),
            (5, 7, BLACK),
            (6, 7, BLACK),
            (7, 2, BLACK),
            (7, 3, BLACK),
            (7, 4, BLACK),
            (7, 5, BLACK),
            (7, 6, BLACK),
        ];
        let renju = recompute_point_for_rule(&stones, (7, 7), RuleSet::Renju);
        assert_eq!(renju.attack_cache[0][7][7], 6);
        assert!(renju.value_cache[0][7][7] > 0);
    }

    #[test]
    fn renju_eval_does_not_suppress_white_forbidden_like_shapes() {
        let stones = [
            (4, 7, WHITE),
            (5, 7, WHITE),
            (6, 7, WHITE),
            (7, 7, WHITE),
            (8, 7, WHITE),
        ];
        let freestyle = recompute_point_for_rule(&stones, (9, 7), RuleSet::Freestyle);
        let renju = recompute_point_for_rule(&stones, (9, 7), RuleSet::Renju);
        assert_eq!(renju.value_cache[1][9][7], freestyle.value_cache[1][9][7]);
        assert_eq!(renju.attack_cache[1][9][7], freestyle.attack_cache[1][9][7]);
        assert!(
            renju.value_cache[1][9][7] > 0 || renju.attack_cache[1][9][7] > 0,
            "white has no forbidden-point suppression"
        );
    }

    #[test]
    fn renju_incremental_eval_matches_full_recompute_with_suppression() {
        let mut board =
            board_from_stones(&[(6, 7, BLACK), (8, 7, BLACK), (7, 6, BLACK), (0, 0, WHITE)]);
        let mut incremental = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut incremental, RuleSet::Renju);

        board.grid_rows_mut()[8][7] = BLACK;
        value_wide_compute_for_rule(&mut board, &mut incremental, (7, 8), RuleSet::Renju);

        let mut full = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut full, RuleSet::Renju);
        assert_eq!(incremental.shape_cache, full.shape_cache);
        assert_eq!(incremental.value_cache, full.value_cache);
        assert_eq!(incremental.attack_cache, full.attack_cache);
        assert_eq!(
            (
                incremental.value_cache[0][7][7],
                incremental.attack_cache[0][7][7]
            ),
            (0, 0)
        );
    }

    #[test]
    fn renju_eval_suppression_matches_detector_on_hand_fixtures() {
        assert_renju_eval_suppression_matches_detector(include_str!(
            "../../cases/renju/forbidden_hand_cases.jsonl"
        ));
    }

    #[test]
    #[ignore = "set RENJU_EVAL_SUPPRESSION_CASE_FILE to a JSONL fixture file"]
    fn renju_eval_suppression_matches_detector_on_env_case_file() {
        let path = std::env::var("RENJU_EVAL_SUPPRESSION_CASE_FILE")
            .expect("RENJU_EVAL_SUPPRESSION_CASE_FILE must point to a JSONL fixture file");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {path}: {err}"));
        assert_renju_eval_suppression_matches_detector(&raw);
    }

    fn assert_renju_eval_suppression_matches_detector(raw: &str) {
        #[derive(serde::Deserialize)]
        struct Stone {
            x: usize,
            y: usize,
            side: i8,
        }
        #[derive(serde::Deserialize)]
        struct Point {
            x: usize,
            y: usize,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            name: String,
            moves: Vec<Stone>,
            candidate: Point,
        }

        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let case: Fixture = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("fixture parse failed: {err}"));
            let stones = case
                .moves
                .iter()
                .map(|stone| (stone.x, stone.y, stone.side))
                .collect::<Vec<_>>();
            let board = board_from_stones(&stones);
            let candidate = (case.candidate.x, case.candidate.y);
            let move_ = crate::board::xy_to_move(candidate.0, candidate.1).unwrap();

            let freestyle = recompute_point_for_rule(&stones, candidate, RuleSet::Freestyle);
            let renju = recompute_point_for_rule(&stones, candidate, RuleSet::Renju);
            let freestyle_has_value = freestyle.value_cache[0][candidate.0][candidate.1] > 0
                || freestyle.attack_cache[0][candidate.0][candidate.1] > 0;
            let renju_suppressed = renju.value_cache[0][candidate.0][candidate.1] == 0
                && renju.attack_cache[0][candidate.0][candidate.1] == 0;

            if freestyle_has_value && renju_suppressed {
                let forbidden = classify_forbidden_move(&board, move_, BLACK, RuleSet::Renju)
                    .expect("fixture candidate is valid")
                    .is_forbidden();
                assert!(
                    forbidden,
                    "{}: Renju eval suppressed a detector-legal black point",
                    case.name
                );
            }
        }
    }
}
