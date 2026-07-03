//! Threat-focused board view and tactical helpers.

use std::cell::RefCell;

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::constants::{BLACK, BOARD_SIZE, EMPTY};
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

pub const MAX_BROKEN_FOUR_REPLIES: usize = 8;
const LEGALITY_CACHE_SIZE: usize = 1 << 16;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LegalityCacheEntry {
    key: u64,
    move_: Move,
    value: bool,
    occupied: bool,
}

#[derive(Clone, Debug)]
struct LegalityCache {
    entries: Vec<LegalityCacheEntry>,
}

impl LegalityCache {
    fn new() -> Self {
        debug_assert!(LEGALITY_CACHE_SIZE.is_power_of_two());
        Self {
            entries: vec![LegalityCacheEntry::default(); LEGALITY_CACHE_SIZE],
        }
    }

    fn index(key: u64, move_: Move) -> usize {
        let mixed = key ^ (key >> 32) ^ (u64::from(move_).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        mixed as usize & (LEGALITY_CACHE_SIZE - 1)
    }

    fn get(&self, key: u64, move_: Move) -> Option<bool> {
        let entry = self.entries[Self::index(key, move_)];
        if entry.occupied && entry.key == key && entry.move_ == move_ {
            Some(entry.value)
        } else {
            None
        }
    }

    fn insert(&mut self, key: u64, move_: Move, value: bool) {
        self.entries[Self::index(key, move_)] = LegalityCacheEntry {
            key,
            move_,
            value,
            occupied: true,
        };
    }
}

impl ThreatLine {
    fn from_values(values: &[i32]) -> Self {
        let mut cells = [1024; BOARD_SIZE + 4];
        cells[2..2 + BOARD_SIZE].copy_from_slice(values);
        Self { cells }
    }

    fn a4(&self, point_index: usize) -> i32 {
        crate::patterns::line::scan_a4(&self.cells, point_index)
    }

    fn a6(&self, point_index: usize) -> i32 {
        crate::patterns::line::scan_a6(&self.cells, point_index)
    }

    fn a5(&self, point_index: usize) -> i32 {
        crate::patterns::line::scan_a5(&self.cells, point_index)
    }

    fn b4(&self, point_index: usize) -> i32 {
        crate::patterns::line::scan_b4(&self.cells, point_index)
    }

    fn b4p(&self, point_index: usize) -> i32 {
        crate::patterns::line::scan_b4p(&self.cells, point_index)
    }

    fn a3(&self, point_index: usize) -> i32 {
        crate::patterns::line::scan_a3(&self.cells, point_index)
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
    /// Memoizes the expensive Renju-black forbidden-move verdict, keyed by
    /// `(zobrist_key, move)`. `is_legal_move_for_rule` for black under Renju
    /// runs a full forbidden classification (double-three/four, overline) that
    /// dominates deep VCT search; the verdict is a pure function of the current
    /// position and candidate point, so caching by zobrist is sound and needs
    /// no invalidation across play/undo. Freestyle and white are never cached
    /// (their legality is a cheap empty/winner check).
    legality_cache: RefCell<Option<Box<LegalityCache>>>,
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
            legality_cache: RefCell::new(
                (rule_set == RuleSet::Renju).then(|| Box::new(LegalityCache::new())),
            ),
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
                        if self.rule_legal_cached(move_, side) {
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

    /// Rule-aware "this open four wins" check for the stone at `(x, y)`.
    /// Freestyle (and white under Renju) matches `has_a4`. For black under
    /// Renju an apparent open four can be fake: a completion may form an
    /// overline or another forbidden shape. Require at least two rule-legal
    /// winning completions so the four is really unstoppable.
    pub fn has_winning_a4(&mut self, x: usize, y: usize) -> bool {
        if !self.has_a4(x, y) {
            return false;
        }
        if self.rule_set != RuleSet::Renju || self.board.grid_rows()[y][x] != BLACK {
            return true;
        }
        self.legal_win_completions(x, y, BLACK).len() >= 2
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
                    .filter(|&gain| self.rule_legal_cached(gain, defender))
                    .collect();
                Some(AttackMove {
                    move_,
                    level: ThreatLevel::A3,
                    defenses,
                })
            }
            1 => {
                let completion = completions[0];
                if self.rule_legal_cached(completion, defender) {
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
                // Equivalent to play(candidate) + winner()==side + undo (play
                // sets `winner` from exactly this predicate) but without the
                // board mutation and threat-line churn.
                if self
                    .board
                    .is_winning_move_for_rule(cxu, cyu, side, self.rule_set)
                {
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
        if !self.rule_legal_cached(gain, attacker) {
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
        self.rule_legal_cached(move_, side)
    }

    /// `Board::is_legal_move_for_rule` with the Renju-black forbidden verdict
    /// memoized by `(zobrist_key, move)`. Only that branch is cached: for
    /// freestyle or white the underlying check is a cheap empty/winner test, so
    /// caching would only add hashing overhead. The cached verdict is a pure
    /// function of the current position and candidate, so it stays valid as the
    /// board is played and unplayed (the zobrist key moves with it).
    pub fn rule_legal_cached(&self, move_: Move, side: Side) -> bool {
        if self.rule_set != RuleSet::Renju || side != BLACK {
            return self
                .board
                .is_legal_move_for_rule(move_, side, self.rule_set);
        }
        let key = self.board.zobrist_key();
        {
            let cache = self.legality_cache.borrow();
            if let Some(cache) = cache.as_ref() {
                if let Some(cached) = cache.get(key, move_) {
                    return cached;
                }
            }
        }
        let verdict = self
            .board
            .is_legal_move_for_rule(move_, side, self.rule_set);
        self.legality_cache
            .borrow_mut()
            .get_or_insert_with(|| Box::new(LegalityCache::new()))
            .insert(key, move_, verdict);
        verdict
    }

    pub fn rule_set(&self) -> RuleSet {
        self.rule_set
    }

    /// True when the four(s) through the stone at `(x, y)` can actually be
    /// completed into a rule-legal winning five by their owner. Freestyle and
    /// white fours always can; a Renju black "four" whose every completion
    /// forms an overline is fake and carries no forcing power.
    pub fn four_has_rule_legal_completion(&mut self, x: usize, y: usize) -> bool {
        let side = self.board.grid_rows()[y][x];
        if self.rule_set != RuleSet::Renju || side != BLACK {
            return true;
        }
        !self.legal_win_completions(x, y, BLACK).is_empty()
    }

    pub fn collect_attack_moves(&mut self, attacker: Side) -> Vec<AttackMove> {
        let moves = self.threat_moves(attacker);
        let mut attacks = Vec::new();
        for move_ in moves {
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
            if self.broken_four_reply(x, y).is_some() && self.four_has_rule_legal_completion(x, y) {
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
        let is_b4_plus = view.board.winner() == side
            || view.has_winning_a4(x, y)
            || (view.b4_count(x, y) >= 1 && view.four_has_rule_legal_completion(x, y));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WHITE;

    // Equivalence gate for the ThreatLine/Line dedup: the two duplicate copies
    // of a3/a4/a5/a6/b4/b4p must return the same value for the same line at
    // every point index. Proven here before collapsing them onto one shared
    // implementation, so a hidden divergence would surface as a mismatch.
    #[test]
    fn threat_line_matches_pattern_line_on_random_lines() {
        use crate::patterns::line::Line;

        let mut state = 0x0bad_c0de_1337_beef_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for _ in 0..30_000 {
            let mut values = [0_i32; BOARD_SIZE];
            for value in values.iter_mut() {
                *value = match next() % 3 {
                    0 => 0,
                    1 => i32::from(BLACK),
                    _ => i32::from(WHITE),
                };
            }
            let threat_line = ThreatLine::from_values(&values);
            let mut cells = vec![1024_i32; BOARD_SIZE + 4];
            cells[2..2 + BOARD_SIZE].copy_from_slice(&values);
            let pattern_line = Line { cells };

            for pi in 0..BOARD_SIZE {
                assert_eq!(
                    threat_line.a3(pi),
                    pattern_line.a3(pi),
                    "a3 @ {pi}: {values:?}"
                );
                assert_eq!(
                    threat_line.a4(pi),
                    pattern_line.a4(pi),
                    "a4 @ {pi}: {values:?}"
                );
                assert_eq!(
                    threat_line.a5(pi),
                    pattern_line.a5(pi),
                    "a5 @ {pi}: {values:?}"
                );
                assert_eq!(
                    threat_line.a6(pi),
                    pattern_line.a6(pi),
                    "a6 @ {pi}: {values:?}"
                );
                assert_eq!(
                    threat_line.b4(pi),
                    pattern_line.b4(pi),
                    "b4 @ {pi}: {values:?}"
                );
                assert_eq!(
                    threat_line.b4p(pi),
                    pattern_line.b4p(pi),
                    "b4p @ {pi}: {values:?}"
                );
            }
        }
    }

    // Oracle: every point `broken_four_legal_replies_for_side` returns for a
    // black stone must, when black plays it, actually complete a five. A reply
    // that does not is a spurious five point (e.g. a mis-detected jump four).
    #[test]
    fn broken_four_replies_are_real_five_completions() {
        let mut state = 0x1234_5678_9abc_def0_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for _ in 0..4000 {
            let mut board = Board::new();
            let n = 6 + (next() % 12) as usize;
            for _ in 0..n {
                let x = (next() % BOARD_SIZE as u64) as usize;
                let y = (next() % BOARD_SIZE as u64) as usize;
                if board.grid_rows()[y][x] == EMPTY {
                    board.grid_rows_mut()[y][x] = if next() % 5 == 0 { WHITE } else { BLACK };
                }
            }
            let view = ThreatBoardView::from_board(board.clone());
            for x in 0..BOARD_SIZE {
                for y in 0..BOARD_SIZE {
                    if board.grid_rows()[y][x] != BLACK {
                        continue;
                    }
                    let replies = view.broken_four_legal_replies_for_side(x, y, BLACK);
                    for &r in replies.as_slice() {
                        let (rx, ry) = move_to_xy(r).expect("reply stays valid");
                        let mut probe = ThreatBoardView::from_board(board.clone());
                        probe.play(r, BLACK);
                        assert!(
                            probe.has_a5(rx, ry),
                            "reply ({rx},{ry}) for stone ({x},{y}) does not complete five; board={:?}",
                            board.grid_rows()
                        );
                    }
                }
            }
        }
    }

    // Deterministic Case B: row 7 holds X X X _ X _ X X (columns 3,4,5,7,9,10
    // black; 6,8,11 empty). Anchor (7,7) sits between the two gaps. Only (6,7)
    // completes a five (3..7). Filling (8,7) yields 7,8,9,10 — four in a row,
    // not five — because column 11 is empty. A correct b4p must not return
    // (8,7); a jump-four second five needs the full X X X on cols 9,10,11.
    #[test]
    fn b4p_does_not_report_incomplete_jump_four_second_five() {
        let mut board = Board::new();
        for x in [3, 4, 5, 7, 9, 10] {
            board.grid_rows_mut()[7][x] = BLACK;
        }
        let view = ThreatBoardView::from_board(board.clone());
        let replies = view.broken_four_legal_replies_for_side(7, 7, BLACK);
        for &r in replies.as_slice() {
            let (rx, ry) = move_to_xy(r).expect("reply stays valid");
            let mut probe = ThreatBoardView::from_board(board.clone());
            probe.play(r, BLACK);
            assert!(
                probe.has_a5(rx, ry),
                "spurious reply ({rx},{ry}); replies={:?}",
                replies.as_slice()
            );
        }
    }

    // Black 3,4,5,6 and 8 on row 7, with white blocking (2,7): every black
    // "four" on this line is fake under Renju. The board four 3..6 can only
    // complete at (7,7), which makes the overline 3..8; the move (9,7) creates
    // the raw four 5,6,_,8,9 whose sole five point is again (7,7), now the
    // overline 3..9. Black therefore has no real forcing threat.
    fn fake_black_four_board() -> Board {
        let mut board = Board::new();
        for (x, y, side) in [
            (3, 7, BLACK),
            (4, 7, BLACK),
            (5, 7, BLACK),
            (6, 7, BLACK),
            (8, 7, BLACK),
            (2, 7, WHITE),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }
        board
    }

    #[test]
    fn renju_fake_black_four_is_not_a_forcing_threat() {
        let board = fake_black_four_board();
        let attack = xy_to_move(9, 7).expect("attack stays valid");

        let freestyle = forcing_threat_moves_for_rule(&board, BLACK, RuleSet::Freestyle);
        assert!(
            freestyle.contains(&attack),
            "freestyle: the raw four 5,6,_,8,9 is forcing"
        );

        let renju = forcing_threat_moves_for_rule(&board, BLACK, RuleSet::Renju);
        assert!(
            renju.is_empty(),
            "renju: the only five point (7,7) is an overline, nothing is forcing: {renju:?}"
        );
    }

    #[test]
    fn renju_fake_black_four_does_not_trigger_vct() {
        let board = fake_black_four_board();

        assert!(
            has_vct_trigger_for_rule(&board, BLACK, RuleSet::Freestyle),
            "freestyle: completing at (7,7) wins outright, the trigger must fire"
        );
        assert!(
            !has_vct_trigger_for_rule(&board, BLACK, RuleSet::Renju),
            "renju: black has no real four or dual three, the trigger must stay off"
        );
    }
}
