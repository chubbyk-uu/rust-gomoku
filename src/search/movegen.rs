//! Normal search candidate generation.

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::EngineConfig;
use crate::constants::{BOARD_AREA, BOARD_SIZE, EMPTY, WIN};
use crate::eval::{attack_level, move_value, EvalCaches};
use crate::patterns::{Line, DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};
use crate::types::Move;

const COVER_DIRS: [(isize, isize); 32] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
    (-2, -2),
    (-2, -1),
    (-2, 0),
    (-2, 1),
    (-2, 2),
    (-1, -2),
    (-1, 2),
    (0, -2),
    (0, 2),
    (1, -2),
    (1, 2),
    (2, -2),
    (2, -1),
    (2, 0),
    (2, 1),
    (2, 2),
    (-3, -3),
    (-3, 0),
    (-3, 3),
    (0, -3),
    (0, 3),
    (3, -3),
    (3, 0),
    (3, 3),
];

const COVER_NEIGHBOR_CAP: usize = 32;
const COVER_SENTINEL: Move = u16::MAX;

static COVER_NEIGHBORS: LazyLock<[[Move; COVER_NEIGHBOR_CAP]; BOARD_AREA]> = LazyLock::new(|| {
    let mut table = [[COVER_SENTINEL; COVER_NEIGHBOR_CAP]; BOARD_AREA];
    for move_index in 0..BOARD_AREA {
        let x = move_index % BOARD_SIZE;
        let y = move_index / BOARD_SIZE;
        let mut count = 0;
        for &(dx, dy) in COVER_DIRS.iter() {
            let xx = x as isize + dx;
            let yy = y as isize + dy;
            if xx >= 0 && yy >= 0 && xx < BOARD_SIZE as isize && yy < BOARD_SIZE as isize {
                table[move_index][count] = (yy as usize * BOARD_SIZE + xx as usize) as Move;
                count += 1;
            }
        }
    }
    table
});

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub move_: Move,
    pub order_score: f64,
    pub self_attack: i32,
    pub opp_attack: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateGenerationResult {
    pub candidates: Vec<Candidate>,
    pub single_forcing: bool,
    pub hostile_threat: bool,
    pub win_priority: bool,
}

pub fn movegen_backend_name() -> &'static str {
    "python"
}

pub fn covered_moves(board: &Board) -> Vec<Move> {
    if board.move_count() == 0 {
        return vec![xy_to_move(BOARD_SIZE / 2, BOARD_SIZE / 2).expect("center move is valid")];
    }

    let mut seen = [false; BOARD_AREA];
    for played in board.move_history() {
        for &candidate in COVER_NEIGHBORS[played.move_ as usize].iter() {
            if candidate == COVER_SENTINEL {
                break;
            }
            if !seen[candidate as usize] {
                let (x, y) = move_to_xy(candidate).expect("covered neighbor move is in range");
                if board.grid_rows()[y][x] == EMPTY {
                    seen[candidate as usize] = true;
                }
            }
        }
    }
    (0..BOARD_AREA as Move)
        .filter(|&m| seen[m as usize])
        .collect()
}

pub fn apply_hostile_three_extension(
    board: &Board,
    move_: Move,
    side: i8,
    vbw_map: &mut [f64; BOARD_AREA],
) {
    let (x, y) = move_to_xy(move_).expect("move is valid");
    let hostile_side = -side;
    let line_specs = [
        (
            Line::from_board(board, x, HORIZONTAL).expect("direction valid"),
            y,
            1_usize,
        ),
        (
            Line::from_board(board, y, VERTICAL).expect("direction valid"),
            x,
            2_usize,
        ),
        (
            Line::from_board(board, x + y, DIAGONAL_DOWN).expect("direction valid"),
            y,
            3_usize,
        ),
        (
            Line::from_board(board, BOARD_SIZE - 1 - y + x, DIAGONAL_UP).expect("direction valid"),
            BOARD_SIZE - 1 - y,
            4_usize,
        ),
    ];

    let mut encoded = 0_i32;
    let mut direction = 0_usize;
    for (line, point_index, direction_id) in line_specs {
        if point_index + 1 < BOARD_SIZE && line.cells[point_index + 3] == i32::from(hostile_side) {
            encoded = line.a3pb(point_index + 1);
        } else if point_index >= 1 && line.cells[point_index + 1] == i32::from(hostile_side) {
            encoded = line.a3pb(point_index - 1);
        }
        if encoded > 0 {
            direction = direction_id;
            break;
        }
    }
    if direction == 0 {
        return;
    }

    for target in decode_bonus_targets(move_, direction, encoded) {
        vbw_map[target as usize] += 10_000.0;
    }
}

pub fn generate_candidates(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
    wide: Option<usize>,
    root_allowed_moves: Option<&HashSet<Move>>,
    preferred_move: Option<Move>,
    preserve_scan_order: bool,
) -> CandidateGenerationResult {
    let moves = covered_moves(board);
    let _wide = wide.unwrap_or(config.root_search.wide as usize);

    let mut vbw_map = [0.0_f64; BOARD_AREA];
    let mut self_attack_map = [0_i32; BOARD_AREA];
    let mut opp_attack_map = [0_i32; BOARD_AREA];
    let mut at1pri = 0_i32;
    let mut at2pri = 0_i32;
    let mut sglflag = 0_i32;
    let mut hsflag = None::<Move>;

    for &move_ in &moves {
        let (x, y) = move_to_xy(move_).expect("covered move is valid");
        let vbw = move_value(caches, x, y, side, config).trunc();
        let att1 = attack_level(caches, x, y, side);
        let att2 = attack_level(caches, x, y, -side);
        let move_index = move_ as usize;
        vbw_map[move_index] = vbw;
        self_attack_map[move_index] = att1;
        opp_attack_map[move_index] = att2;

        if vbw <= 0.0 {
            at2pri = at2pri.max(att2);
            continue;
        }
        if att2 == 6 || att1 >= 5 {
            sglflag += 1;
        } else if att2 == 5 {
            hsflag = Some(move_);
        }
        at1pri = at1pri.max(att1);
        at2pri = at2pri.max(att2);
    }

    let winpri = at1pri == 6 || (at1pri == 5 && at2pri <= 5);
    if sglflag == 0 {
        if let Some(hostile_move) = hsflag {
            apply_hostile_three_extension(board, hostile_move, side, &mut vbw_map);
        }
    }

    let mut candidates = Vec::with_capacity(moves.len());
    for &move_ in &moves {
        if let Some(allowed) = root_allowed_moves {
            if !allowed.contains(&move_) {
                continue;
            }
        }

        let move_index = move_ as usize;
        let mut vbw = vbw_map[move_index];
        if hsflag.is_some() {
            vbw -= 5000.0;
            if self_attack_map[move_index] >= 4 {
                vbw += 8000.0;
            }
        }
        if vbw <= 0.0 {
            continue;
        }

        let mut score = vbw - 300_000_000.0;
        if preferred_move == Some(move_) {
            score = 100.0;
        }
        let candidate = Candidate {
            move_,
            order_score: score,
            self_attack: self_attack_map[move_index],
            opp_attack: opp_attack_map[move_index],
        };
        if score >= f64::from(WIN) {
            candidates = vec![candidate];
            break;
        }
        if score <= -f64::from(WIN) && score >= -200_000_000.0 {
            continue;
        }
        candidates.push(candidate);
    }

    if !preserve_scan_order {
        candidates.sort_unstable_by(|a, b| {
            b.order_score
                .partial_cmp(&a.order_score)
                .unwrap()
                .then_with(|| a.move_.cmp(&b.move_))
        });
    }

    if winpri && !candidates.is_empty() {
        return CandidateGenerationResult {
            candidates: vec![candidates[0]],
            single_forcing: false,
            hostile_threat: hsflag.is_some(),
            win_priority: true,
        };
    }
    if sglflag > 0 && !candidates.is_empty() {
        return CandidateGenerationResult {
            candidates: vec![candidates[0]],
            single_forcing: true,
            hostile_threat: hsflag.is_some(),
            win_priority: winpri,
        };
    }

    CandidateGenerationResult {
        candidates,
        single_forcing: false,
        hostile_threat: hsflag.is_some(),
        win_priority: winpri,
    }
}

fn decode_bonus_targets(move_: Move, direction_index: usize, encoded: i32) -> Vec<Move> {
    let (x, y) = move_to_xy(move_).expect("move is valid");
    let mut raw = vec![ga(encoded), gb(encoded)];
    if encoded >= (1 << 24) {
        raw.push(gc(encoded));
    }

    let mut targets = Vec::new();
    for value in raw {
        let (tx, ty) = match direction_index {
            1 => (x as isize, value as isize),
            2 => (value as isize, y as isize),
            3 => (x as isize + y as isize - value as isize, value as isize),
            4 => (
                BOARD_SIZE as isize - 1 + x as isize - y as isize - value as isize,
                BOARD_SIZE as isize - 1 - value as isize,
            ),
            _ => continue,
        };
        if tx >= 0 && ty >= 0 && tx < BOARD_SIZE as isize && ty < BOARD_SIZE as isize {
            targets.push(
                xy_to_move(tx as usize, ty as usize)
                    .expect("decoded hostile bonus target is valid"),
            );
        }
    }
    targets
}

fn ga(value: i32) -> i32 {
    value & 0xFF
}

fn gb(value: i32) -> i32 {
    (value >> 8) & 0xFF
}

fn gc(value: i32) -> i32 {
    (value >> 16) & 0xFF
}
