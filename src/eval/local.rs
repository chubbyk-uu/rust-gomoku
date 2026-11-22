//! Local point evaluation and cache maintenance.

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{BLACK, BOARD_SIZE, DSHAPE_SIZE, EMPTY, WHITE};
use crate::eval::caches::{dgn_contribution, update_packed_lines_cell, EvalCaches};
use crate::eval::global_eval::{blocked_neighbor_count, NEIGHBOR_CHECKS};
use crate::patterns::buckets::bucket_for_lines;
use crate::patterns::line::{
    shape_raw_from_board_point_hypothetical, PackedLine, PACKED_LINE_COUNT,
};
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

#[inline(always)]
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
        lines[idx] = if label >= ShapeLabel::L6 as i32 {
            label - ShapeLabel::L6 as i32
        } else {
            label
        };

        match label {
            x if x == ShapeLabel::L3 as i32 || x == ShapeLabel::L3B as i32 => {
                open_threes += 1;
                attack = attack.max(3);
            }
            x if x == ShapeLabel::L4S as i32 => {
                fours += aux;
                attack = attack.max(4);
                if aux >= 2 {
                    lines[idx] = 8;
                }
            }
            x if x == ShapeLabel::L5 as i32 => {
                attack = 6;
                exact_fives += 1;
            }
            x if x == ShapeLabel::L4 as i32 => {
                fours += 1;
                attack = attack.max(5);
            }
            x if x == ShapeLabel::L6 as i32 => {
                overlines += 1;
            }
            _ => {}
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

    let bucket = ((top1 * (top1 + 1)) >> 1) + top2 + 1;
    debug_assert_eq!(
        bucket,
        bucket_for_lines(top1, top2).expect("normalized line strengths are valid")
    );
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
                    caches.push_value_log(player, x, y, old_bucket, old_attack);
                }
            }

            caches.value_cache[player][x][y] = 0;
            caches.attack_cache[player][x][y] = 0;

            for direction in 0..4 {
                let old = caches.shape_cache[player][x][y][direction];
                if old != 0 {
                    if caches.active_snapshot_count > 0 {
                        caches.push_shape_log(player, x, y, direction, old);
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
                    caches.push_shape_log(player, x, y, direction_index, old);
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
            caches.push_value_log(player, x, y, old_bucket, old_attack);
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
    if rule == RuleSet::Renju {
        // Populate the incremental apparent-double-three set from scratch. Safe
        // to hard-reset the list when no snapshot is outstanding (the usual
        // setup/rule-change case); otherwise fall back to per-point updates.
        if caches.active_snapshot_count == 0 {
            caches.clear_dt3();
        }
        for x in 0..size {
            for y in 0..size {
                refresh_dt3_membership(caches, board, x, y);
            }
        }
    }
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
    if !caches.initialized || caches.rule_set != rule {
        recompute_all_for_rule(board, caches, rule);
        return;
    }
    caches.rule_set = rule;

    let (cx, cy) = changed;
    let renju = rule == RuleSet::Renju;
    let radius = if renju { 5 } else { 4 };
    let mut black_refreshed = [0_u64; 4];

    let old_cell = caches.board_shadow[cx][cy];
    let new_cell = board.grid_rows()[cy][cx];
    let center_changed = old_cell != new_cell;
    if center_changed {
        update_packed_lines_cell(&mut caches.packed_lines, cx, cy, new_cell);
    }
    let [horizontal, vertical, diagonal_down, diagonal_up] = caches.packed_lines_through(cx, cy);
    #[cfg(debug_assertions)]
    {
        let grid = board.grid_rows();
        debug_assert_eq!(horizontal, PackedLine::through(grid, cx, cy, 0, 1));
        debug_assert_eq!(vertical, PackedLine::through(grid, cx, cy, 1, 0));
        debug_assert_eq!(diagonal_down, PackedLine::through(grid, cx, cy, -1, 1));
        debug_assert_eq!(diagonal_up, PackedLine::through(grid, cx, cy, -1, -1));
    }
    let mut old_center_zero = [[false; 4]; 2];
    if center_changed {
        for (player, row) in old_center_zero.iter_mut().enumerate() {
            for (dir, slot) in row.iter_mut().enumerate() {
                *slot = ((caches.shape_cache[player][cx][cy][dir] >> 16) & 15) == 0;
            }
        }
        if old_cell != EMPTY {
            let old_player = side_index(old_cell);
            let old_cc = caches.dgn_cc[cx][cy];
            caches.dgn_sum[old_player] -= dgn_contribution(old_cc);
            caches.set_dgn_cc(cx, cy, 0);
        }
    }

    let skip_stone = center_changed.then_some((cx, cy));
    // Process center cell (cx, cy) directly (the sole intersection of all 4 lines).
    remove_empty_bucket_contribution(caches, cx, cy);
    if new_cell == EMPTY {
        let notify_center_dgn = !center_changed;
        let mut changed_player = [false; 2];
        merge_changed_player(
            &mut changed_player,
            update_direction_cache(
                board,
                caches,
                cx,
                cy,
                HORIZONTAL,
                horizontal.hypothetical_shapes(cy, renju),
                notify_center_dgn,
                skip_stone,
            ),
        );
        merge_changed_player(
            &mut changed_player,
            update_direction_cache(
                board,
                caches,
                cx,
                cy,
                VERTICAL,
                vertical.hypothetical_shapes(cx, renju),
                notify_center_dgn,
                skip_stone,
            ),
        );
        merge_changed_player(
            &mut changed_player,
            update_direction_cache(
                board,
                caches,
                cx,
                cy,
                DIAGONAL_DOWN,
                diagonal_down.hypothetical_shapes(cy, renju),
                notify_center_dgn,
                skip_stone,
            ),
        );
        merge_changed_player(
            &mut changed_player,
            update_direction_cache(
                board,
                caches,
                cx,
                cy,
                DIAGONAL_UP,
                diagonal_up.hypothetical_shapes(BOARD_SIZE - 1 - cy, renju),
                notify_center_dgn,
                skip_stone,
            ),
        );
        for (player, changed) in changed_player.into_iter().enumerate() {
            if changed {
                let is_dt3 = update_bucket_attack(board, caches, cx, cy, player, rule);
                if renju && player == 0 {
                    caches.set_dt3_present(cx, cy, is_dt3);
                    set_point_bit(&mut black_refreshed, cx, cy);
                }
            }
        }
    } else {
        clear_occupied_point(caches, cx, cy);
        if renju {
            caches.set_dt3_present(cx, cy, false);
        }
    }
    if center_changed {
        update_center_neighbor_dgn(board, caches, cx, cy, old_cell, new_cell, &old_center_zero);
    }
    add_empty_bucket_contribution(board, caches, cx, cy);
    caches.set_board_shadow(cx, cy, new_cell);

    // Non-center dirty points lie on strictly one of the 4 lines through (cx, cy).
    let (h_fwd, h_bwd) = horizontal.neighbor_counts(cy, radius);
    for (dy, count) in [(1_isize, h_fwd), (-1_isize, h_bwd)] {
        for step in 1..=count as isize {
            let y = (cy as isize + step * dy) as usize;
            update_line_neighbor_point(
                board,
                caches,
                cx,
                y,
                HORIZONTAL,
                horizontal,
                y,
                rule,
                renju,
                skip_stone,
                &mut black_refreshed,
            );
        }
    }
    let (v_fwd, v_bwd) = vertical.neighbor_counts(cx, radius);
    for (dx, count) in [(1_isize, v_fwd), (-1_isize, v_bwd)] {
        for step in 1..=count as isize {
            let x = (cx as isize + step * dx) as usize;
            update_line_neighbor_point(
                board,
                caches,
                x,
                cy,
                VERTICAL,
                vertical,
                x,
                rule,
                renju,
                skip_stone,
                &mut black_refreshed,
            );
        }
    }
    let (dd_fwd, dd_bwd) = diagonal_down.neighbor_counts(cy, radius);
    for (dx, dy, count) in [(-1_isize, 1_isize, dd_fwd), (1_isize, -1_isize, dd_bwd)] {
        for step in 1..=count as isize {
            let x = (cx as isize + step * dx) as usize;
            let y = (cy as isize + step * dy) as usize;
            update_line_neighbor_point(
                board,
                caches,
                x,
                y,
                DIAGONAL_DOWN,
                diagonal_down,
                y,
                rule,
                renju,
                skip_stone,
                &mut black_refreshed,
            );
        }
    }
    let (up_fwd, up_bwd) = diagonal_up.neighbor_counts(BOARD_SIZE - 1 - cy, radius);
    for (dx, dy, count) in [(1_isize, 1_isize, up_bwd), (-1_isize, -1_isize, up_fwd)] {
        for step in 1..=count as isize {
            let x = (cx as isize + step * dx) as usize;
            let y = (cy as isize + step * dy) as usize;
            update_line_neighbor_point(
                board,
                caches,
                x,
                y,
                DIAGONAL_UP,
                diagonal_up,
                BOARD_SIZE - 1 - y,
                rule,
                renju,
                skip_stone,
                &mut black_refreshed,
            );
        }
    }

    if center_changed && new_cell != EMPTY {
        let new_player = side_index(new_cell);
        let cc = blocked_neighbor_count(
            board.grid_rows(),
            &caches.shape_cache[new_player],
            cx,
            cy,
            board.size(),
        ) as u8;
        caches.set_dgn_cc(cx, cy, cc);
        caches.dgn_sum[new_player] += dgn_contribution(cc);
    }

    if renju {
        refresh_renju_apparent_double_three_points(board, caches, &black_refreshed);
    }
}

#[inline(always)]
fn set_point_bit(bits: &mut [u64; 4], x: usize, y: usize) {
    let idx = y * BOARD_SIZE + x;
    bits[idx >> 6] |= 1_u64 << (idx & 63);
}

#[inline(always)]
fn has_point_bit(bits: &[u64; 4], idx: usize) -> bool {
    (bits[idx >> 6] & (1_u64 << (idx & 63))) != 0
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn update_line_neighbor_point(
    board: &Board,
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    direction: i32,
    line: PackedLine,
    line_pos: usize,
    rule: RuleSet,
    renju: bool,
    skip_stone: Option<(usize, usize)>,
    black_refreshed: &mut [u64; 4],
) {
    let cell = board.grid_rows()[y][x];
    if cell == EMPTY {
        let was_empty = caches.board_shadow[x][y] == EMPTY;
        if !was_empty {
            remove_empty_bucket_contribution(caches, x, y);
        }
        let shapes = line.hypothetical_shapes(line_pos, renju);
        let changed_player =
            update_direction_cache(board, caches, x, y, direction, shapes, true, skip_stone);
        for (player, changed) in changed_player.into_iter().enumerate() {
            if changed {
                let old_bucket = caches.value_cache[player][x][y] as usize;
                let is_dt3 = update_bucket_attack(board, caches, x, y, player, rule);
                if was_empty {
                    let new_bucket = caches.value_cache[player][x][y] as usize;
                    if old_bucket != new_bucket {
                        caches.dec_empty_bucket(player, old_bucket);
                        caches.inc_empty_bucket(player, new_bucket);
                    }
                }
                if renju && player == 0 {
                    caches.set_dt3_present(x, y, is_dt3);
                    set_point_bit(black_refreshed, x, y);
                }
            }
        }
        if !was_empty {
            add_empty_bucket_contribution(board, caches, x, y);
            caches.set_board_shadow(x, y, EMPTY);
        }
    } else if caches.board_shadow[x][y] != cell {
        remove_empty_bucket_contribution(caches, x, y);
        clear_occupied_point(caches, x, y);
        if renju {
            caches.set_dt3_present(x, y, false);
        }
        caches.set_board_shadow(x, y, cell);
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
    caches.nonzero_bucket_mask = [[0; 3]; 2];
    caches.dgn_sum = [0; 2];
    let preserve_snapshot = caches.active_snapshot_count > 0;
    if !preserve_snapshot {
        caches.dgn_cc = [[0; BOARD_SIZE]; BOARD_SIZE];
        caches.dgn_cc_log.clear();
        caches.shadow_log.clear();
        caches.packed_lines = [[0; 2]; PACKED_LINE_COUNT];
    }

    let size = board.size();
    let grid = board.grid_rows();
    for x in 0..size {
        for y in 0..size {
            let cell = grid[y][x];
            if preserve_snapshot {
                caches.set_board_shadow(x, y, cell);
            } else {
                caches.board_shadow[x][y] = cell;
                if cell != EMPTY {
                    update_packed_lines_cell(&mut caches.packed_lines, x, y, cell);
                }
            }
            if cell == EMPTY {
                if preserve_snapshot {
                    caches.set_dgn_cc(x, y, 0);
                }
                for player in 0..2 {
                    let bucket = caches.value_cache[player][x][y] as usize;
                    caches.inc_empty_bucket(player, bucket);
                }
            } else {
                let player = side_index(cell);
                let cc =
                    blocked_neighbor_count(grid, &caches.shape_cache[player], x, y, size) as u8;
                if preserve_snapshot {
                    caches.set_dgn_cc(x, y, cc);
                } else {
                    caches.dgn_cc[x][y] = cc;
                }
                caches.dgn_sum[player] += dgn_contribution(cc);
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
        caches.dec_empty_bucket(player, bucket);
    }
}

fn add_empty_bucket_contribution(board: &Board, caches: &mut EvalCaches, x: usize, y: usize) {
    if board.grid_rows()[y][x] != EMPTY {
        return;
    }
    for player in 0..2 {
        let bucket = caches.value_cache[player][x][y] as usize;
        caches.inc_empty_bucket(player, bucket);
    }
}

fn merge_changed_player(accumulator: &mut [bool; 2], changed: [bool; 2]) {
    accumulator[0] |= changed[0];
    accumulator[1] |= changed[1];
}

const DIRECTION_STEPS: [(isize, isize); 4] = [(0, 1), (1, 0), (-1, 1), (1, 1)];

fn update_center_neighbor_dgn(
    board: &Board,
    caches: &mut EvalCaches,
    cx: usize,
    cy: usize,
    old_cell: i8,
    new_cell: i8,
    old_center_zero: &[[bool; 4]; 2],
) {
    let grid = board.grid_rows();
    let size = board.size() as isize;
    for &(dx, dy, dir) in &NEIGHBOR_CHECKS {
        let ox = cx as isize + dx;
        let oy = cy as isize + dy;
        if ox < 0 || oy < 0 || ox >= size || oy >= size {
            continue;
        }
        let (ux, uy) = (ox as usize, oy as usize);
        let stone = grid[uy][ux];
        if stone == EMPTY {
            continue;
        }
        let player = side_index(stone);
        let old_blocked = old_cell != EMPTY || old_center_zero[player][dir];
        let new_blocked =
            new_cell != EMPTY || ((caches.shape_cache[player][cx][cy][dir] >> 16) & 15) == 0;
        if old_blocked != new_blocked {
            caches.adjust_dgn_cc(ux, uy, player, if new_blocked { 1 } else { -1 });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_direction_cache(
    board: &Board,
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    direction: i32,
    shapes: [i32; 2],
    notify_dgn: bool,
    skip_stone: Option<(usize, usize)>,
) -> [bool; 2] {
    let mut changed = [false; 2];
    let direction_index = direction as usize;
    let (dx, dy) = DIRECTION_STEPS[direction_index];
    let grid = board.grid_rows();
    let size = board.size() as isize;
    for (player, new_shape) in shapes.into_iter().enumerate() {
        let old = caches.shape_cache[player][x][y][direction_index];
        if old != new_shape {
            if caches.active_snapshot_count > 0 {
                caches.push_shape_log(player, x, y, direction_index, old);
            }
            caches.shape_cache[player][x][y][direction_index] = new_shape;
            changed[player] = true;
            if notify_dgn {
                let old_zero = ((old >> 16) & 15) == 0;
                let new_zero = ((new_shape >> 16) & 15) == 0;
                if old_zero != new_zero {
                    let target_stone = if player == 0 { BLACK } else { WHITE };
                    let delta = if new_zero { 1 } else { -1 };
                    for sign in [-1_isize, 1_isize] {
                        let ox = x as isize + sign * dx;
                        let oy = y as isize + sign * dy;
                        if ox < 0 || oy < 0 || ox >= size || oy >= size {
                            continue;
                        }
                        let (ux, uy) = (ox as usize, oy as usize);
                        if skip_stone == Some((ux, uy)) {
                            continue;
                        }
                        if grid[uy][ux] == target_stone {
                            caches.adjust_dgn_cc(ux, uy, player, delta);
                        }
                    }
                }
            }
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
) -> bool {
    if board.grid_rows()[y][x] != EMPTY {
        return false;
    }
    let shapes = &caches.shape_cache[player][x][y];
    let side = if player == 0 { BLACK } else { WHITE };
    let counts = compute_bucket_attack_and_counts((shapes[0], shapes[1], shapes[2], shapes[3]));
    let is_apparent_dt3 = counts.exact_fives == 0 && counts.open_threes >= 2;
    let (bucket, attack) = if rule == RuleSet::Renju && side == BLACK && counts.exact_fives == 0 {
        if counts.fours >= 2 || counts.overlines > 0 {
            (0, 0)
        } else if counts.open_threes >= 2
            && classify_forbidden_move(board, (y * BOARD_SIZE + x) as Move, side, rule)
                .is_ok_and(|kind| kind.is_forbidden())
        {
            (0, 0)
        } else {
            (counts.bucket, counts.attack)
        }
    } else {
        (counts.bucket, counts.attack)
    };
    set_bucket_attack(caches, x, y, player, bucket, attack);
    is_apparent_dt3
}

fn set_bucket_attack(
    caches: &mut EvalCaches,
    x: usize,
    y: usize,
    player: usize,
    bucket: i32,
    attack: i32,
) {
    let old_bucket = caches.value_cache[player][x][y];
    let old_attack = caches.attack_cache[player][x][y];
    if old_bucket == bucket && old_attack == attack {
        return;
    }
    if caches.active_snapshot_count > 0 {
        caches.push_value_log(player, x, y, old_bucket, old_attack);
    }
    caches.value_cache[player][x][y] = bucket;
    caches.attack_cache[player][x][y] = attack;
}

/// Recompute the black apparent-double-three membership of `(x, y)` from its
/// current black shapes and update the incremental set. Membership is
/// `empty && exact_fives == 0 && open_threes >= 2` — exactly the condition the
/// refresh below uses, so the set holds precisely the points the refresh visits.
fn refresh_dt3_membership(caches: &mut EvalCaches, board: &Board, x: usize, y: usize) {
    let member = if board.grid_rows()[y][x] != EMPTY {
        false
    } else {
        let shapes = &caches.shape_cache[0][x][y];
        let counts = compute_bucket_attack_and_counts((shapes[0], shapes[1], shapes[2], shapes[3]));
        counts.exact_fives == 0 && counts.open_threes >= 2
    };
    caches.set_dt3_present(x, y, member);
}

fn refresh_renju_apparent_double_three_points(
    board: &Board,
    caches: &mut EvalCaches,
    black_refreshed: &[u64; 4],
) {
    let player = 0_usize;

    // Iterate only the maintained apparent-double-three candidates, re-evaluating
    // each one because a non-local move can flip its forbidden status without
    // changing its own shapes. `dt3_members` is append-only and may hold stale
    // entries, so skip any whose membership flag is no longer set, and skip
    // dirty points that were already evaluated against the current board above.
    let member_count = caches.dt3_members.len();
    for i in 0..member_count {
        let move_ = caches.dt3_members[i] as usize;
        let x = move_ % BOARD_SIZE;
        let y = move_ / BOARD_SIZE;
        if !caches.dt3_present[x][y] || has_point_bit(black_refreshed, move_) {
            continue;
        }

        let shapes = &caches.shape_cache[player][x][y];
        let direction_shapes = (shapes[0], shapes[1], shapes[2], shapes[3]);
        let (bucket, attack) = compute_bucket_and_attack_for_rule(
            board,
            x,
            y,
            BLACK,
            RuleSet::Renju,
            direction_shapes,
        );
        let old_bucket = caches.value_cache[player][x][y];
        let old_attack = caches.attack_cache[player][x][y];
        if old_bucket == bucket && old_attack == attack {
            continue;
        }

        let old_bucket_index = old_bucket as usize;
        let new_bucket_index = bucket as usize;
        caches.dec_empty_bucket(player, old_bucket_index);
        caches.inc_empty_bucket(player, new_bucket_index);
        set_bucket_attack(caches, x, y, player, bucket, attack);
    }
}

fn clear_occupied_point(caches: &mut EvalCaches, x: usize, y: usize) {
    for player in 0..2 {
        let old_bucket = caches.value_cache[player][x][y];
        let old_attack = caches.attack_cache[player][x][y];
        if caches.active_snapshot_count > 0 && (old_bucket != 0 || old_attack != 0) {
            caches.push_value_log(player, x, y, old_bucket, old_attack);
        }
        caches.value_cache[player][x][y] = 0;
        caches.attack_cache[player][x][y] = 0;

        for direction in 0..4 {
            let old = caches.shape_cache[player][x][y][direction];
            if old != 0 {
                if caches.active_snapshot_count > 0 {
                    caches.push_shape_log(player, x, y, direction, old);
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

    // Renju double-three legality is recursive and non-monotonic: a move can
    // flip a distant black point between forbidden and legal. SlowRenju handles
    // this with its `trd3` dirty list (ValueWide.cpp). This stress check plays
    // many seeded move sequences in a dense cluster and asserts the incremental
    // `value_wide_compute_for_rule` caches stay identical to a full recompute,
    // so any missed non-local invalidation would surface as a mismatch.
    // It reproduces the original bug (e.g. trial 24 with the seed below): an
    // incrementally updated black point stayed stale-suppressed (0) after a
    // later move made it legal again.
    #[test]
    fn renju_incremental_eval_matches_full_recompute_over_random_sequences() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for trial in 0..400usize {
            let mut board = Board::new();
            let mut incremental = EvalCaches::new();
            recompute_all_for_rule(&mut board, &mut incremental, RuleSet::Renju);

            let move_count = 8 + (next() % 22) as usize;
            for _ in 0..move_count {
                // Dense 9x9 cluster around the centre, black-heavy to provoke
                // overlapping fours/threes and forbidden shapes.
                let x = 3 + (next() % 9) as usize;
                let y = 3 + (next() % 9) as usize;
                if board.grid_rows()[y][x] != EMPTY {
                    continue;
                }
                let side = if next() % 100 < 68 { BLACK } else { WHITE };
                board.grid_rows_mut()[y][x] = side;
                value_wide_compute_for_rule(&mut board, &mut incremental, (x, y), RuleSet::Renju);
            }

            let mut full = EvalCaches::new();
            recompute_all_for_rule(&mut board, &mut full, RuleSet::Renju);
            assert_eq!(
                incremental.shape_cache, full.shape_cache,
                "trial {trial}: shape_cache diverged"
            );
            assert_eq!(
                incremental.value_cache, full.value_cache,
                "trial {trial}: value_cache diverged (likely non-local forbidden invalidation)"
            );
            assert_eq!(
                incremental.attack_cache, full.attack_cache,
                "trial {trial}: attack_cache diverged (likely non-local forbidden invalidation)"
            );
        }
    }

    #[test]
    fn renju_snapshot_restore_reverts_caches_including_dt3() {
        // Exercises the make/unmake path the stress test above does not: every
        // step snapshots, plays a trial move with value_wide_compute (which
        // mutates the incremental apparent-double-three set), then restores and
        // asserts the entire cache — dt3_present/dt3_members included — returns
        // to the pre-snapshot state.
        let mut state = 0x51a2_b3c4_d5e6_f701_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for trial in 0..200usize {
            let mut board = Board::new();
            let mut caches = EvalCaches::new();
            recompute_all_for_rule(&mut board, &mut caches, RuleSet::Renju);

            // Build a random dense base position so apparent double-threes exist.
            let base_moves = 6 + (next() % 16) as usize;
            for _ in 0..base_moves {
                let x = 3 + (next() % 9) as usize;
                let y = 3 + (next() % 9) as usize;
                if board.grid_rows()[y][x] != EMPTY {
                    continue;
                }
                let side = if next() % 100 < 68 { BLACK } else { WHITE };
                board.grid_rows_mut()[y][x] = side;
                value_wide_compute_for_rule(&mut board, &mut caches, (x, y), RuleSet::Renju);
            }

            // Try several snapshot -> trial move -> restore cycles.
            for _ in 0..6 {
                let mut tx = 3 + (next() % 9) as usize;
                let mut ty = 3 + (next() % 9) as usize;
                let mut tries = 0;
                while board.grid_rows()[ty][tx] != EMPTY && tries < 8 {
                    tx = 3 + (next() % 9) as usize;
                    ty = 3 + (next() % 9) as usize;
                    tries += 1;
                }
                if board.grid_rows()[ty][tx] != EMPTY {
                    continue;
                }

                let before = caches.clone();
                let snapshot = caches.snapshot();
                let side = if next() % 100 < 68 { BLACK } else { WHITE };
                board.grid_rows_mut()[ty][tx] = side;
                value_wide_compute_for_rule(&mut board, &mut caches, (tx, ty), RuleSet::Renju);
                board.grid_rows_mut()[ty][tx] = EMPTY;
                caches.restore_snapshot(&snapshot);

                assert_eq!(
                    caches, before,
                    "trial {trial}: caches did not revert after snapshot/restore"
                );
            }
        }
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

    #[test]
    fn recompute_all_under_active_snapshot_restores_dgn_cc_cleanly() {
        use crate::board::xy_to_move;
        use crate::config::load_default_config;
        use crate::eval::{evaluate_board_main_cached, evaluate_board_main_scan};

        let config = load_default_config();
        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            let mut board = Board::new();
            let mut caches = EvalCaches::new();
            board.play(xy_to_move(7, 7).unwrap(), Some(BLACK)).unwrap();
            recompute_all_for_rule(&mut board, &mut caches, rule);

            let snapshot = caches.snapshot();
            board.play(xy_to_move(6, 6).unwrap(), Some(WHITE)).unwrap();
            recompute_all_for_rule(&mut board, &mut caches, rule);
            board.undo().unwrap();
            caches.restore_snapshot(&snapshot);

            board.play(xy_to_move(7, 6).unwrap(), Some(WHITE)).unwrap();
            value_wide_compute_for_rule(&mut board, &mut caches, (7, 6), rule);

            for side in [BLACK, WHITE] {
                let cached = evaluate_board_main_cached(&board, &caches, side, &config);
                let scan = evaluate_board_main_scan(&board, &caches, side, &config);
                assert_eq!(
                    cached, scan,
                    "{rule:?} side={side}: incremental dgn must match scan after snapshot + recompute_all"
                );
            }
        }
    }
}
