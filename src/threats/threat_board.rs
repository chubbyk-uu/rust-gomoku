//! Threat-focused board view and tactical helpers.

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::constants::{BOARD_SIZE, EMPTY};
use crate::patterns::Line;
use crate::threats::types::{AttackMove, ThreatLevel};
use crate::types::{Move, Side};

const THREAT_DIRS: [(isize, isize); 16] = [
    (-2, -2),
    (-1, -1),
    (2, 2),
    (1, 1),
    (-2, 2),
    (-1, 1),
    (2, -2),
    (1, -1),
    (2, 0),
    (1, 0),
    (0, 2),
    (0, 1),
    (-2, 0),
    (-1, 0),
    (0, -2),
    (0, -1),
];

fn ga(value: i32) -> usize {
    (value & 0xFF) as usize
}

fn gb(value: i32) -> usize {
    ((value >> 8) & 0xFF) as usize
}

fn decode_line_move(
    board: &Board,
    x: usize,
    y: usize,
    direction_index: usize,
    encoded: i32,
) -> Option<Move> {
    let raw = ga(encoded);
    let (tx, ty) = match direction_index {
        1 => (x as isize, raw as isize),
        2 => (raw as isize, y as isize),
        3 => (x as isize + y as isize - raw as isize, raw as isize),
        4 => (
            board.size() as isize - 1 + x as isize - y as isize - raw as isize,
            board.size() as isize - 1 - raw as isize,
        ),
        _ => return None,
    };
    if tx >= 0 && ty >= 0 && tx < board.size() as isize && ty < board.size() as isize {
        xy_to_move(tx as usize, ty as usize).ok()
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub struct ThreatBoardView {
    pub board: Board,
    pub x1: Vec<Vec<i32>>,
    pub x2: Vec<Vec<i32>>,
    pub x3: Vec<Vec<i32>>,
    pub x4: Vec<Vec<i32>>,
    previous_sides: Vec<Side>,
}

impl ThreatBoardView {
    pub fn from_board(board: Board) -> Self {
        let size = board.size();
        let grid = board.grid_rows();
        let x1 = (0..size)
            .map(|x| (0..size).map(|y| i32::from(grid[y][x])).collect())
            .collect();
        let x2 = (0..size)
            .map(|y| (0..size).map(|x| i32::from(grid[y][x])).collect())
            .collect();
        let width = 2 * size - 1;
        let mut x3 = vec![vec![1024; size]; width];
        let mut x4 = vec![vec![1024; size]; width];
        for p in 0..width {
            if p < size {
                for i in 0..=p {
                    x3[p][i] = i32::from(grid[i][p - i]);
                    x4[p][i] = i32::from(grid[size - 1 - i][p - i]);
                }
            } else {
                let start = p - size + 1;
                for i in start..size {
                    x3[p][i] = i32::from(grid[i][p - i]);
                    x4[p][i] = i32::from(grid[size - 1 - i][p - i]);
                }
            }
        }
        Self {
            board,
            x1,
            x2,
            x3,
            x4,
            previous_sides: Vec::new(),
        }
    }

    fn lines_for(
        &self,
        x: usize,
        y: usize,
    ) -> (Line, Line, Line, Line, usize, usize, usize, usize) {
        fn pad(values: &[i32]) -> Vec<i32> {
            let mut padded = vec![1024, 1024];
            padded.extend_from_slice(values);
            padded.extend_from_slice(&[1024, 1024]);
            padded
        }

        (
            Line {
                cells: pad(&self.x1[x]),
            },
            Line {
                cells: pad(&self.x2[y]),
            },
            Line {
                cells: pad(&self.x3[x + y]),
            },
            Line {
                cells: pad(&self.x4[BOARD_SIZE - 1 - y + x]),
            },
            y,
            x,
            y,
            BOARD_SIZE - 1 - y,
        )
    }

    fn set_point(&mut self, x: usize, y: usize, value: Side) {
        self.x1[x][y] = i32::from(value);
        self.x2[y][x] = i32::from(value);
        self.x3[x + y][y] = i32::from(value);
        self.x4[BOARD_SIZE - 1 - y + x][BOARD_SIZE - 1 - y] = i32::from(value);
    }

    pub fn play(&mut self, move_: Move, side: Side) {
        let previous_side = self.board.side_to_move();
        self.previous_sides.push(previous_side);
        self.board
            .force_side_to_move(side)
            .expect("threat board only plays valid sides");
        self.board
            .play(move_, Some(side))
            .expect("threat board play should stay legal");
        let (x, y) = move_to_xy(move_).expect("move stays in range");
        self.set_point(x, y, side);
    }

    pub fn undo(&mut self) {
        let previous_side = self.previous_sides.pop().expect("previous side tracked");
        let played = self
            .board
            .undo()
            .expect("threat board move history stays valid");
        self.board
            .force_side_to_move(previous_side)
            .expect("tracked previous side remains valid");
        let (x, y) = move_to_xy(played.move_).expect("move stays in range");
        self.set_point(x, y, EMPTY);
    }

    pub fn threat_moves(&self, side: Side) -> Vec<Move> {
        let size = self.board.size();
        let grid = self.board.grid_rows();
        let mut candidates = Vec::new();
        for x in 0..size {
            for y in 0..size {
                if grid[y][x] != EMPTY {
                    continue;
                }
                for (dx, dy) in THREAT_DIRS {
                    let xx = x as isize + dx;
                    let yy = y as isize + dy;
                    if xx >= 0
                        && yy >= 0
                        && xx < size as isize
                        && yy < size as isize
                        && grid[yy as usize][xx as usize] == side
                    {
                        candidates.push(xy_to_move(x, y).expect("threat move stays valid"));
                        break;
                    }
                }
            }
        }
        candidates
    }

    pub fn has_a4(&self, x: usize, y: usize) -> bool {
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        l1.a4(p1) > 0 || l2.a4(p2) > 0 || l3.a4(p3) > 0 || l4.a4(p4) > 0
    }

    pub fn has_a6(&self, x: usize, y: usize) -> bool {
        if self.board.grid_rows()[y][x] == EMPTY {
            return false;
        }
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        l1.a6(p1) > 0 || l2.a6(p2) > 0 || l3.a6(p3) > 0 || l4.a6(p4) > 0
    }

    pub fn has_a5(&self, x: usize, y: usize) -> bool {
        if self.board.grid_rows()[y][x] == EMPTY {
            return false;
        }
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        l1.a5(p1) > 0 || l2.a5(p2) > 0 || l3.a5(p3) > 0 || l4.a5(p4) > 0
    }

    pub fn a5test(&mut self, x: usize, y: usize, side: Side) -> bool {
        let point = self.board.grid_rows()[y][x];
        if point == side {
            return self.has_a5(x, y);
        }
        if point != EMPTY {
            return false;
        }
        let move_ = xy_to_move(x, y).expect("point stays valid");
        self.play(move_, side);
        let result = self.has_a5(x, y);
        self.undo();
        result
    }

    pub fn b4_count(&self, x: usize, y: usize) -> i32 {
        if self.board.grid_rows()[y][x] == EMPTY {
            return 0;
        }
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        l1.b4(p1) + l2.b4(p2) + l3.b4(p3) + l4.b4(p4)
    }

    pub fn a3r_count(&mut self, x: usize, y: usize) -> i32 {
        let point = self.board.grid_rows()[y][x];
        if point == EMPTY {
            return 0;
        }
        let side = point;
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        let mut count = 0;
        let encoded_lines = [
            (l1.a3(p1), 1_usize),
            (l2.a3(p2), 2_usize),
            (l3.a3(p3), 3_usize),
            (l4.a3(p4), 4_usize),
        ];
        for (encoded, direction) in encoded_lines {
            if encoded <= 0 {
                continue;
            }
            let mut raws = vec![ga(encoded)];
            if encoded >= 65_536 {
                raws.push(gb(encoded));
            }
            let mut penalty = true;
            for raw in raws {
                let move_ = decode_line_move(&self.board, x, y, direction, raw as i32);
                if let Some(move_) = move_ {
                    let (rx, ry) = move_to_xy(move_).expect("decoded move stays valid");
                    if !(side == 1 && self.a5test(rx, ry, side)) {
                        penalty = false;
                    }
                }
            }
            if penalty && side == 1 {
                count -= 1;
            }
            count += 1;
        }
        count
    }

    pub fn is_double4(&self, x: usize, y: usize) -> bool {
        self.b4_count(x, y) >= 2
    }

    pub fn is_double3r(&mut self, x: usize, y: usize) -> bool {
        self.a3r_count(x, y) >= 2
    }

    pub fn broken_four_reply(&self, x: usize, y: usize) -> Option<Move> {
        self.broken_four_reply_with_ambiguity(x, y).0
    }

    pub fn broken_four_reply_with_ambiguity(&self, x: usize, y: usize) -> (Option<Move>, bool) {
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        let counts = [l1.b4p(p1), l2.b4p(p2), l3.b4p(p3), l4.b4p(p4)];
        let directions = [1_usize, 2, 3, 4];

        for (encoded, direction) in counts.into_iter().zip(directions) {
            if encoded >= (1 << 16) {
                return (
                    decode_line_move(&self.board, x, y, direction, encoded),
                    true,
                );
            }
        }

        let mut mask = 0_u8;
        for (index, encoded) in counts.iter().enumerate() {
            if *encoded != 0 {
                mask |= 1 << index;
            }
        }
        if mask == 0 {
            return (None, false);
        }
        if mask == 1 {
            return (decode_line_move(&self.board, x, y, 1, counts[0]), false);
        }
        if mask == 2 {
            return (decode_line_move(&self.board, x, y, 2, counts[1]), false);
        }
        if mask == 4 {
            return (decode_line_move(&self.board, x, y, 3, counts[2]), false);
        }
        if mask == 8 {
            return (decode_line_move(&self.board, x, y, 4, counts[3]), false);
        }
        if matches!(mask, 3 | 5 | 7 | 9 | 11 | 13 | 15) {
            return (decode_line_move(&self.board, x, y, 1, counts[0]), true);
        }
        if matches!(mask, 6 | 10 | 14) {
            return (decode_line_move(&self.board, x, y, 2, counts[1]), true);
        }
        if mask == 12 {
            return (decode_line_move(&self.board, x, y, 3, counts[2]), true);
        }
        (None, false)
    }

    pub fn broken_four_point_for_side(&self, side: Side) -> (Option<Move>, bool) {
        let grid = self.board.grid_rows();
        let size = self.board.size();
        let mut first_reply = None;
        for x in 0..size {
            for y in 0..size {
                if grid[y][x] != side {
                    continue;
                }
                let (reply, local_ambiguous) = self.broken_four_reply_with_ambiguity(x, y);
                if reply.is_none() {
                    continue;
                }
                if local_ambiguous {
                    return (reply, true);
                }
                if first_reply.is_none() {
                    first_reply = reply;
                } else if reply != first_reply {
                    return (reply, true);
                }
            }
        }
        (first_reply, false)
    }

    pub fn a3_gain_squares(&self, x: usize, y: usize) -> Vec<Move> {
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        let encoded_lines = [
            (l1.a3(p1), 1_usize),
            (l2.a3(p2), 2_usize),
            (l3.a3(p3), 3_usize),
            (l4.a3(p4), 4_usize),
        ];
        let mut seen = std::collections::HashSet::new();
        let mut gains = Vec::new();
        for (encoded, direction) in encoded_lines {
            if encoded <= 0 {
                continue;
            }
            let mut raws = vec![ga(encoded)];
            if encoded >= 65_536 {
                raws.push(gb(encoded));
            }
            for raw in raws {
                if let Some(move_) = decode_line_move(&self.board, x, y, direction, raw as i32) {
                    if seen.insert(move_) {
                        gains.push(move_);
                    }
                }
            }
        }
        gains
    }

    pub fn classify_attack_at(
        &mut self,
        x: usize,
        y: usize,
        attacker: Side,
        move_: Move,
    ) -> Option<AttackMove> {
        if self.board.winner() == attacker {
            return Some(AttackMove {
                move_,
                level: ThreatLevel::WIN5,
                defenses: Vec::new(),
            });
        }
        if self.has_a4(x, y) {
            return Some(AttackMove {
                move_,
                level: ThreatLevel::A4,
                defenses: Vec::new(),
            });
        }
        let (reply, ambiguous) = self.broken_four_reply_with_ambiguity(x, y);
        if ambiguous {
            return Some(AttackMove {
                move_,
                level: ThreatLevel::A4,
                defenses: Vec::new(),
            });
        }
        if let Some(reply) = reply {
            if self.board.is_legal_move(reply) {
                return Some(AttackMove {
                    move_,
                    level: ThreatLevel::B4,
                    defenses: vec![reply],
                });
            }
            return Some(AttackMove {
                move_,
                level: ThreatLevel::A4,
                defenses: Vec::new(),
            });
        }
        let legal: Vec<_> = self
            .a3_gain_squares(x, y)
            .into_iter()
            .filter(|&gain| self.board.is_legal_move(gain))
            .collect();
        if !legal.is_empty() {
            return Some(AttackMove {
                move_,
                level: ThreatLevel::A3,
                defenses: legal,
            });
        }
        None
    }

    pub fn collect_attack_moves(&mut self, attacker: Side) -> Vec<AttackMove> {
        let moves = self.threat_moves(attacker);
        let mut attacks = Vec::new();
        for move_ in moves {
            if !self.board.is_legal_move(move_) {
                continue;
            }
            self.play(move_, attacker);
            let (x, y) = move_to_xy(move_).expect("move stays valid");
            let attack = self.classify_attack_at(x, y, attacker, move_);
            self.undo();
            if let Some(attack) = attack {
                attacks.push(attack);
            }
        }
        attacks.sort_by_key(|attack| std::cmp::Reverse(attack.level));
        attacks
    }

    pub fn winning_threat_moves(&mut self, side: Side) -> Vec<Move> {
        let moves = self.threat_moves(side);
        let mut wins = Vec::new();
        for move_ in moves {
            self.play(move_, side);
            let (x, y) = move_to_xy(move_).expect("move stays valid");
            if self.board.winner() == side || self.has_a4(x, y) {
                wins.push(move_);
            }
            self.undo();
        }
        wins
    }

    pub fn forcing_threat_moves(&mut self, side: Side) -> Vec<Move> {
        let moves = self.threat_moves(side);
        let mut forcing = Vec::new();
        for move_ in moves {
            self.play(move_, side);
            let (x, y) = move_to_xy(move_).expect("move stays valid");
            if self.broken_four_reply(x, y).is_some() {
                forcing.push(move_);
            }
            self.undo();
        }
        forcing
    }
}

pub fn has_vct_trigger(board: &Board, side: Side) -> bool {
    let mut view = ThreatBoardView::from_board(board.clone());
    let moves = view.threat_moves(side);
    for move_ in moves {
        if !board.is_legal_move(move_) {
            continue;
        }
        view.play(move_, side);
        let (x, y) = move_to_xy(move_).expect("move stays valid");
        let is_b4_plus =
            view.board.winner() == side || view.has_a4(x, y) || view.b4_count(x, y) >= 1;
        let is_dual_a3 = !is_b4_plus && view.a3r_count(x, y) >= 2;
        view.undo();
        if is_b4_plus || is_dual_a3 {
            return true;
        }
    }
    false
}

pub fn threat_moves(board: &Board, side: Side) -> Vec<Move> {
    ThreatBoardView::from_board(board.clone()).threat_moves(side)
}

pub fn has_open_four(board: &Board, x: usize, y: usize) -> bool {
    ThreatBoardView::from_board(board.clone()).has_a4(x, y)
}

pub fn broken_four_reply(board: &Board, x: usize, y: usize) -> Option<Move> {
    ThreatBoardView::from_board(board.clone()).broken_four_reply(x, y)
}

pub fn winning_threat_moves(board: &Board, side: Side) -> Vec<Move> {
    ThreatBoardView::from_board(board.clone()).winning_threat_moves(side)
}

pub fn forcing_threat_moves(board: &Board, side: Side) -> Vec<Move> {
    ThreatBoardView::from_board(board.clone()).forcing_threat_moves(side)
}
