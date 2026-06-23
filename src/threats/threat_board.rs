//! Threat-focused board view and tactical helpers.

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::constants::{BLACK, BOARD_SIZE, EMPTY, WHITE};
use crate::rules::RuleSet;
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

fn comb(x: usize, y: usize) -> i32 {
    ((x as i32) << 8) | (y as i32 - 2)
}

fn comc(x: usize, y: usize, z: usize) -> i32 {
    comb(comb(x, y) as usize, z)
}

pub const MAX_BROKEN_FOUR_REPLIES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrokenFourReplies {
    moves: [Move; MAX_BROKEN_FOUR_REPLIES],
    len: usize,
}

impl Default for BrokenFourReplies {
    fn default() -> Self {
        Self::new()
    }
}

impl BrokenFourReplies {
    pub fn new() -> Self {
        Self {
            moves: [u16::MAX; MAX_BROKEN_FOUR_REPLIES],
            len: 0,
        }
    }

    pub fn push_unique(&mut self, move_: Move) {
        if self.as_slice().contains(&move_) || self.len >= MAX_BROKEN_FOUR_REPLIES {
            return;
        }
        self.moves[self.len] = move_;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    pub fn first(&self) -> Option<Move> {
        self.as_slice().first().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ThreatLine {
    cells: [i32; BOARD_SIZE + 4],
}

impl ThreatLine {
    fn from_values(values: &[i32]) -> Self {
        let mut cells = [1024; BOARD_SIZE + 4];
        cells[2..2 + BOARD_SIZE].copy_from_slice(values);
        Self { cells }
    }

    fn a4(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(3));
        let xmax = usize::min(BOARD_SIZE - 2, p);
        for i in xmin..=xmax {
            if self.cells[i] + self.cells[i + 1] + self.cells[i + 2] + self.cells[i + 3] != 4 * x0 {
                continue;
            }
            if self.cells[i - 1] == i32::from(EMPTY) && self.cells[i + 4] == i32::from(EMPTY) {
                return 1;
            }
        }
        0
    }

    fn a6(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        if self.cells[p] != i32::from(BLACK) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(5));
        let xmax = usize::min(BOARD_SIZE - 4, p);
        for i in xmin..=xmax {
            if self.cells[i]
                + self.cells[i + 1]
                + self.cells[i + 2]
                + self.cells[i + 3]
                + self.cells[i + 4]
                + self.cells[i + 5]
                == 6
            {
                return 1;
            }
        }
        0
    }

    fn a5(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(4));
        let xmax = usize::min(BOARD_SIZE - 3, p);
        for i in xmin..=xmax {
            if self.cells[i]
                + self.cells[i + 1]
                + self.cells[i + 2]
                + self.cells[i + 3]
                + self.cells[i + 4]
                == 5 * x0
            {
                return 1;
            }
        }
        0
    }

    fn b4(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(4));
        let xmax = usize::min(BOARD_SIZE - 3, p);
        for i in xmin..=xmax {
            if self.cells[i]
                + self.cells[i + 1]
                + self.cells[i + 2]
                + self.cells[i + 3]
                + self.cells[i + 4]
                != 4 * x0
            {
                continue;
            }
            let mut shape = (self.cells[i] << 4)
                + (self.cells[i + 1] << 3)
                + (self.cells[i + 2] << 2)
                + (self.cells[i + 3] << 1)
                + self.cells[i + 4];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x1E || shape == 0x0F {
                return 1;
            }
            if shape == 0x1D {
                if i <= BOARD_SIZE - 7
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && self.cells[i + 8] == x0
                    && p == i + 4
                {
                    return 2;
                }
                return 1;
            }
            if shape == 0x1B {
                if i <= BOARD_SIZE - 6
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && (p == i + 4 || p == i + 3)
                {
                    return 2;
                }
                return 1;
            }
            if shape == 0x17 {
                if i <= BOARD_SIZE - 5
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && (p == i + 4 || p == i + 3 || p == i + 2)
                {
                    return 2;
                }
                return 1;
            }
        }
        0
    }

    fn b4p(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(4));
        let xmax = usize::min(BOARD_SIZE - 3, p);
        for i in xmin..=xmax {
            if self.cells[i]
                + self.cells[i + 1]
                + self.cells[i + 2]
                + self.cells[i + 3]
                + self.cells[i + 4]
                != 4 * x0
            {
                continue;
            }
            let mut shape = (self.cells[i] << 4)
                + (self.cells[i + 1] << 3)
                + (self.cells[i + 2] << 2)
                + (self.cells[i + 3] << 1)
                + self.cells[i + 4];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x1E {
                if self.cells[i - 1] == i32::from(EMPTY) {
                    return comc(1, i - 1, i + 4);
                }
                return comb(1, i + 4);
            }
            if shape == 0x1D {
                if i <= BOARD_SIZE - 7
                    && self.cells[i + 5] == x0
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && p == i + 4
                    && self.cells[i + 3] == i32::from(EMPTY)
                {
                    return comc(1, i + 3, i + 5);
                }
                if self.cells[i + 3] == i32::from(EMPTY) {
                    return comb(1, i + 3);
                }
            }
            if shape == 0x1B {
                if i <= BOARD_SIZE - 6
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && (p == i + 4 || p == i + 3)
                    && self.cells[i + 2] == i32::from(EMPTY)
                {
                    return comc(1, i + 2, i + 5);
                }
                if self.cells[i + 2] == i32::from(EMPTY) {
                    return comb(1, i + 2);
                }
            }
            if shape == 0x17 {
                if i <= BOARD_SIZE - 5
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && (p == i + 4 || p == i + 3 || p == i + 2)
                    && self.cells[i + 1] == i32::from(EMPTY)
                {
                    return comc(1, i + 1, i + 5);
                }
                if self.cells[i + 1] == i32::from(EMPTY) {
                    return comb(1, i + 1);
                }
            }
            if shape == 0x0F {
                if self.cells[i + 5] == i32::from(EMPTY) {
                    return comc(1, i, i + 5);
                }
                return comb(1, i);
            }
        }
        0
    }

    fn a3(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(3));
        let xmax = usize::min(BOARD_SIZE - 2, p);
        for i in xmin..=xmax {
            let num1 = self.cells[i] + self.cells[i + 1] + self.cells[i + 2] + self.cells[i + 3];
            let num2 = self.cells[i] * self.cells[i + 1] * self.cells[i + 2] * self.cells[i + 3];
            if num1 != 3 * x0 || num2 != 0 {
                continue;
            }
            let mut shape = (self.cells[i] << 3)
                + (self.cells[i + 1] << 2)
                + (self.cells[i + 2] << 1)
                + self.cells[i + 3];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x0E {
                if self.cells[i - 1] == i32::from(EMPTY)
                    && self.cells[i - 2] != x0
                    && self.cells[i + 4] != x0
                {
                    if self.cells[i - 2] == i32::from(EMPTY)
                        && self.cells[i + 4] == i32::from(EMPTY)
                    {
                        return comc(1, i - 1, i + 3);
                    }
                    if self.cells[i - 2] == i32::from(EMPTY) {
                        return comb(1, i - 1);
                    }
                    if self.cells[i + 4] == i32::from(EMPTY) {
                        return comb(1, i + 3);
                    }
                }
            }
            if shape == 0x0D
                && self.cells[i - 1] == i32::from(EMPTY)
                && self.cells[i + 4] == i32::from(EMPTY)
            {
                return comb(1, i + 2);
            }
            if shape == 0x0B
                && self.cells[i - 1] == i32::from(EMPTY)
                && self.cells[i + 4] == i32::from(EMPTY)
            {
                return comb(1, i + 1);
            }
        }
        0
    }
}

fn decode_line_move(
    board: &Board,
    x: usize,
    y: usize,
    direction_index: usize,
    encoded: i32,
) -> Option<Move> {
    let raw = ga(encoded);
    decode_line_raw_move(board, x, y, direction_index, raw)
}

fn decode_line_raw_move(
    board: &Board,
    x: usize,
    y: usize,
    direction_index: usize,
    raw: usize,
) -> Option<Move> {
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

fn push_legal_line_replies(
    replies: &mut BrokenFourReplies,
    board: &Board,
    side: Side,
    rule: RuleSet,
    x: usize,
    y: usize,
    direction_index: usize,
    encoded: i32,
) {
    if encoded <= 0 {
        return;
    }
    let raws = [ga(encoded), gb(encoded)];
    let raw_count = if encoded >= (1 << 16) { 2 } else { 1 };
    for &raw in &raws[..raw_count] {
        if let Some(move_) = decode_line_raw_move(board, x, y, direction_index, raw) {
            if board.is_legal_move_for_rule(move_, side, rule) {
                replies.push_unique(move_);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ThreatBoardView {
    pub board: Board,
    pub x1: Vec<Vec<i32>>,
    pub x2: Vec<Vec<i32>>,
    pub x3: Vec<Vec<i32>>,
    pub x4: Vec<Vec<i32>>,
    rule_set: RuleSet,
    previous_sides: Vec<Side>,
}

impl ThreatBoardView {
    pub fn from_board(board: Board) -> Self {
        Self::from_board_with_rule(board, RuleSet::Freestyle)
    }

    pub fn from_board_with_rule(board: Board, rule_set: RuleSet) -> Self {
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
            rule_set,
            previous_sides: Vec::new(),
        }
    }

    fn lines_for(
        &self,
        x: usize,
        y: usize,
    ) -> (
        ThreatLine,
        ThreatLine,
        ThreatLine,
        ThreatLine,
        usize,
        usize,
        usize,
        usize,
    ) {
        (
            ThreatLine::from_values(&self.x1[x]),
            ThreatLine::from_values(&self.x2[y]),
            ThreatLine::from_values(&self.x3[x + y]),
            ThreatLine::from_values(&self.x4[BOARD_SIZE - 1 - y + x]),
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
            .play_assuming_rule_legal(move_, Some(side), self.rule_set)
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
                        let move_ = xy_to_move(x, y).expect("threat move stays valid");
                        if self
                            .board
                            .is_legal_move_for_rule(move_, side, self.rule_set)
                        {
                            candidates.push(move_);
                        }
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

    // NOTE: this can call a5test -> play/undo while already inside another
    // ThreatBoardView::play frame (for example has_vct_trigger). The
    // previous_sides stack is intentionally frame-local: each undo must restore
    // the side_to_move observed immediately before its matching play, not the
    // original board side. Keep the nested restore invariant covered by tests.
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
            let mut raws = [0_usize; 2];
            raws[0] = ga(encoded);
            let raw_count = if encoded >= 65_536 {
                raws[1] = gb(encoded);
                2
            } else {
                1
            };
            let mut penalty = true;
            for &raw in &raws[..raw_count] {
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

    pub fn broken_four_legal_reply(&self, x: usize, y: usize) -> Option<Move> {
        self.broken_four_legal_replies(x, y).first()
    }

    pub fn broken_four_legal_replies(&self, x: usize, y: usize) -> BrokenFourReplies {
        self.broken_four_legal_replies_for_side(x, y, BLACK)
    }

    pub fn broken_four_legal_replies_for_side(
        &self,
        x: usize,
        y: usize,
        side: Side,
    ) -> BrokenFourReplies {
        let (l1, l2, l3, l4, p1, p2, p3, p4) = self.lines_for(x, y);
        let counts = [l1.b4p(p1), l2.b4p(p2), l3.b4p(p3), l4.b4p(p4)];
        let mut replies = BrokenFourReplies::new();

        for (index, encoded) in counts.iter().enumerate() {
            if *encoded >= (1 << 16) {
                push_legal_line_replies(
                    &mut replies,
                    &self.board,
                    side,
                    self.rule_set,
                    x,
                    y,
                    index + 1,
                    *encoded,
                );
            }
        }

        let mut mask = 0_u8;
        for (index, encoded) in counts.iter().enumerate() {
            if *encoded != 0 {
                mask |= 1 << index;
            }
        }

        let direction_order: &[usize] = match mask {
            0 => &[],
            1 => &[1],
            2 => &[2],
            3 => &[1, 2],
            4 => &[3],
            5 => &[1, 3],
            6 => &[2, 3],
            7 => &[1, 2, 3],
            8 => &[4],
            9 => &[1, 4],
            10 => &[2, 4],
            11 => &[1, 2, 4],
            12 => &[3, 4],
            13 => &[1, 3, 4],
            14 => &[2, 3, 4],
            15 => &[1, 2, 3, 4],
            _ => &[],
        };
        for &direction in direction_order {
            let encoded = counts[direction - 1];
            push_legal_line_replies(
                &mut replies,
                &self.board,
                side,
                self.rule_set,
                x,
                y,
                direction,
                encoded,
            );
        }
        replies
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
        if self.rule_set != RuleSet::Freestyle {
            return self.broken_four_legal_point_for_side(side);
        }
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

    pub fn broken_four_legal_point_for_side(&self, side: Side) -> (Option<Move>, bool) {
        let grid = self.board.grid_rows();
        let size = self.board.size();
        let mut first_reply = None;
        for x in 0..size {
            for y in 0..size {
                if grid[y][x] != side {
                    continue;
                }
                let replies = self.broken_four_legal_replies_for_side(x, y, side);
                let Some(reply) = replies.first() else {
                    continue;
                };
                if replies.len() > 1 {
                    return (Some(reply), true);
                }
                if first_reply.is_none() {
                    first_reply = Some(reply);
                } else if first_reply != Some(reply) {
                    return (Some(reply), true);
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
        let mut seen = [false; BOARD_SIZE * BOARD_SIZE];
        let mut gains = Vec::new();
        for (encoded, direction) in encoded_lines {
            if encoded <= 0 {
                continue;
            }
            let mut raws = [0_usize; 2];
            raws[0] = ga(encoded);
            let raw_count = if encoded >= 65_536 {
                raws[1] = gb(encoded);
                2
            } else {
                1
            };
            for &raw in &raws[..raw_count] {
                if let Some(move_) = decode_line_move(&self.board, x, y, direction, raw as i32) {
                    let index = move_ as usize;
                    if !seen[index] {
                        seen[index] = true;
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
        if self.rule_set == RuleSet::Freestyle {
            self.classify_attack_at_freestyle(x, y, attacker, move_)
        } else {
            self.classify_attack_at_renju(x, y, attacker, move_)
        }
    }

    fn classify_attack_at_freestyle(
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

    /// Rule-aware attack classification. The attacker stone is already on the
    /// board at `(x, y)`. Threat levels are reconstructed from rule-legal winning
    /// completions so that black threats relying on forbidden moves (overline
    /// fours, fake open threes) are not counted, while forbidden defender blocks
    /// are removed so the attacker can exploit them.
    fn classify_attack_at_renju(
        &mut self,
        x: usize,
        y: usize,
        attacker: Side,
        move_: Move,
    ) -> Option<AttackMove> {
        let defender = -attacker;
        if self.board.winner() == attacker {
            return Some(AttackMove {
                move_,
                level: ThreatLevel::WIN5,
                defenses: Vec::new(),
            });
        }

        let completions = self.legal_win_completions(x, y, attacker);
        match completions.len() {
            0 => {
                let gains = self.a3_gain_squares(x, y);
                let has_real_threat = gains
                    .iter()
                    .any(|&gain| self.gain_creates_open_four(gain, attacker));
                if !has_real_threat {
                    return None;
                }
                let defenses: Vec<_> = gains
                    .into_iter()
                    .filter(|&gain| {
                        self.board
                            .is_legal_move_for_rule(gain, defender, self.rule_set)
                    })
                    .collect();
                Some(AttackMove {
                    move_,
                    level: ThreatLevel::A3,
                    defenses,
                })
            }
            1 => {
                let completion = completions[0];
                if self
                    .board
                    .is_legal_move_for_rule(completion, defender, self.rule_set)
                {
                    Some(AttackMove {
                        move_,
                        level: ThreatLevel::B4,
                        defenses: vec![completion],
                    })
                } else {
                    // The defender cannot legally block the only winning point.
                    Some(AttackMove {
                        move_,
                        level: ThreatLevel::A4,
                        defenses: Vec::new(),
                    })
                }
            }
            _ => Some(AttackMove {
                move_,
                level: ThreatLevel::A4,
                defenses: Vec::new(),
            }),
        }
    }

    /// Empty points where `side` could play next to win immediately under the
    /// active rule (exact five for black in Renju, five-or-more otherwise),
    /// restricted to the lines passing through the just-played stone `(x, y)`.
    fn legal_win_completions(&mut self, x: usize, y: usize, side: Side) -> Vec<Move> {
        const AXES: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];
        let size = self.board.size() as isize;
        let mut wins = Vec::new();
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
                let (cxu, cyu) = (cx as usize, cy as usize);
                if self.board.grid_rows()[cyu][cxu] != EMPTY {
                    continue;
                }
                let candidate = xy_to_move(cxu, cyu).expect("completion stays valid");
                if wins.contains(&candidate) || !self.board.is_legal_move(candidate) {
                    continue;
                }
                self.play(candidate, side);
                let win = self.board.winner() == side;
                self.undo();
                if win {
                    wins.push(candidate);
                }
            }
        }
        wins
    }

    /// True if `attacker` playing `gain` is rule-legal and yields an unstoppable
    /// open four (two or more rule-legal winning completions) or an immediate
    /// win. Used to verify that an open-three threat can really be escalated.
    fn gain_creates_open_four(&mut self, gain: Move, attacker: Side) -> bool {
        if !self
            .board
            .is_legal_move_for_rule(gain, attacker, self.rule_set)
        {
            return false;
        }
        self.play(gain, attacker);
        let (gx, gy) = move_to_xy(gain).expect("gain stays valid");
        let real = self.board.winner() == attacker
            || self.legal_win_completions(gx, gy, attacker).len() >= 2;
        self.undo();
        real
    }

    /// Rule-aware legality of `move_` for `side`, used by the threat searcher to
    /// drop forbidden defender replies without reaching into private fields.
    pub fn is_rule_legal(&self, move_: Move, side: Side) -> bool {
        self.board
            .is_legal_move_for_rule(move_, side, self.rule_set)
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
    has_vct_trigger_for_rule(board, side, RuleSet::Freestyle)
}

pub fn has_vct_trigger_for_rule(board: &Board, side: Side, rule: RuleSet) -> bool {
    let mut view = ThreatBoardView::from_board_with_rule(board.clone(), rule);
    let moves = view.threat_moves(side);
    for move_ in moves {
        if !view.is_rule_legal(move_, side) {
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

pub fn forcing_threat_moves_for_rule(board: &Board, side: Side, rule: RuleSet) -> Vec<Move> {
    ThreatBoardView::from_board_with_rule(board.clone(), rule).forcing_threat_moves(side)
}
