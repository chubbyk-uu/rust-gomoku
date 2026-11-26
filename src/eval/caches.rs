//! Incremental evaluation caches.

use crate::constants::{BLACK, BOARD_SIZE, DSHAPE_SIZE, WHITE};
use crate::patterns::line::{PackedLine, PACKED_LINE_COUNT};
use crate::rules::RuleSet;
use crate::types::Move;

pub fn caches_backend_name() -> &'static str {
    "python"
}

pub type BoardShadow = [[i8; BOARD_SIZE]; BOARD_SIZE];
pub type ValueCache = [[[i32; BOARD_SIZE]; BOARD_SIZE]; 2];
pub type ShapeCache = [[[[i32; 4]; BOARD_SIZE]; BOARD_SIZE]; 2];
pub type EmptyBucketCounts = [[u16; DSHAPE_SIZE]; 2];
pub type NonZeroBucketMask = [[u64; 3]; 2];
pub type DgnBlockedCounts = [[u8; BOARD_SIZE]; BOARD_SIZE];
pub(crate) type PackedLineStones = [[u32; 2]; PACKED_LINE_COUNT];
/// Membership flags for empty points that are black "apparent double-threes"
/// (`exact_fives == 0 && open_threes >= 2`). Maintained incrementally so the
/// Renju refresh only re-evaluates these points instead of scanning the board.
pub type Dt3Present = [[bool; BOARD_SIZE]; BOARD_SIZE];

#[inline(always)]
pub(crate) fn update_packed_lines_cell(lines: &mut PackedLineStones, x: usize, y: usize, cell: i8) {
    let h_idx = x;
    let v_idx = BOARD_SIZE + y;
    let dd_idx = 30 + x + y;
    let inv_y = BOARD_SIZE - 1 - y;
    let du_idx = 59 + x + inv_y;

    let y_bit = 1_u32 << (y + 5);
    let x_bit = 1_u32 << (x + 5);
    let inv_y_bit = 1_u32 << (inv_y + 5);

    if cell == BLACK {
        lines[h_idx][0] |= y_bit;
        lines[h_idx][1] &= !y_bit;
        lines[v_idx][0] |= x_bit;
        lines[v_idx][1] &= !x_bit;
        lines[dd_idx][0] |= y_bit;
        lines[dd_idx][1] &= !y_bit;
        lines[du_idx][0] |= inv_y_bit;
        lines[du_idx][1] &= !inv_y_bit;
    } else if cell == WHITE {
        lines[h_idx][0] &= !y_bit;
        lines[h_idx][1] |= y_bit;
        lines[v_idx][0] &= !x_bit;
        lines[v_idx][1] |= x_bit;
        lines[dd_idx][0] &= !y_bit;
        lines[dd_idx][1] |= y_bit;
        lines[du_idx][0] &= !inv_y_bit;
        lines[du_idx][1] |= inv_y_bit;
    } else {
        lines[h_idx][0] &= !y_bit;
        lines[h_idx][1] &= !y_bit;
        lines[v_idx][0] &= !x_bit;
        lines[v_idx][1] &= !x_bit;
        lines[dd_idx][0] &= !y_bit;
        lines[dd_idx][1] &= !y_bit;
        lines[du_idx][0] &= !inv_y_bit;
        lines[du_idx][1] &= !inv_y_bit;
    }
}

#[inline(always)]
pub(crate) fn dgn_contribution(cc: u8) -> i32 {
    if cc <= 1 {
        5
    } else if cc >= 6 {
        i32::from(cc) - 4
    } else {
        0
    }
}

/// Compact threat category per bucket (`0..92`):
/// `0 = none`, `1 = L5 (79..=91)`, `2 = L4 (67..=78)`, `3 = double-four (66)`,
/// `4 = four-three (64..=65)`, `5 = double-three (45, 54, 55)`.
pub(crate) const THREAT_CLASS: [u8; DSHAPE_SIZE] = {
    let mut table = [0_u8; DSHAPE_SIZE];
    table[45] = 5;
    table[54] = 5;
    table[55] = 5;
    table[64] = 4;
    table[65] = 4;
    table[66] = 3;
    let mut b = 67;
    while b <= 78 {
        table[b] = 2;
        b += 1;
    }
    let mut b = 79;
    while b <= 91 {
        table[b] = 1;
        b += 1;
    }
    table
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSnapshot {
    pub initialized: bool,
    pub rule_set: RuleSet,
    pub empty_bucket_counts: EmptyBucketCounts,
    pub nonzero_bucket_mask: NonZeroBucketMask,
    pub threat_counts: [[u8; 6]; 2],
    pub attack4_mask: [[u64; 4]; 2],
    pub l5_mask: [[u64; 4]; 2],
    pub dgn_sum: [i32; 2],
    pub shadow_log_len: usize,
    pub dgn_cc_log_len: usize,
    pub shape_log_len: usize,
    pub value_log_len: usize,
    pub dt3_present_log_len: usize,
    pub dt3_members_len: usize,
}

/// Lightweight snapshot for NNUE search: skips `empty_bucket_counts` (368B),
/// `nonzero_bucket_mask` (48B), and classical `dgn` state since NNUE reads only
/// `threat_counts`, `attack4_mask`, `l5_mask`, and per-point shape/bucket/attack caches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSnapshotNnue {
    pub initialized: bool,
    pub rule_set: RuleSet,
    pub threat_counts: [[u8; 6]; 2],
    pub attack4_mask: [[u64; 4]; 2],
    pub l5_mask: [[u64; 4]; 2],
    pub shadow_log_len: usize,
    pub shape_log_len: usize,
    pub value_log_len: usize,
    pub dt3_present_log_len: usize,
    pub dt3_members_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalCaches {
    pub initialized: bool,
    pub rule_set: RuleSet,
    pub board_shadow: BoardShadow,
    pub shape_cache: ShapeCache,
    pub value_cache: ValueCache,
    pub attack_cache: ValueCache,
    pub empty_bucket_counts: EmptyBucketCounts,
    pub(crate) nonzero_bucket_mask: NonZeroBucketMask,
    pub(crate) threat_counts: [[u8; 6]; 2],
    pub(crate) attack4_mask: [[u64; 4]; 2],
    pub(crate) l5_mask: [[u64; 4]; 2],
    pub(crate) packed_lines: PackedLineStones,
    pub(crate) dgn_cc: DgnBlockedCounts,
    pub(crate) dgn_sum: [i32; 2],
    pub(crate) shadow_log: Vec<(u8, u8, i8)>,
    pub(crate) dgn_cc_log: Vec<(u8, u8, u8)>,
    pub(crate) shape_log: Vec<(u8, u8, u8, u8, i32)>,
    pub(crate) value_log: Vec<(u8, u8, u8, i16, i8)>,
    pub(crate) active_snapshot_count: usize,
    // Incremental black apparent-double-three set. `dt3_present` is the
    // authoritative per-point membership; `dt3_members` is an append-only list
    // of candidate points (it may hold stale entries, filtered by `dt3_present`
    // when iterated). Membership transitions are journaled in `dt3_present_log`
    // and `dt3_members` is append-only, so unmake reverts both by replay/truncate
    // with zero cost when no transitions occur (e.g. Freestyle never touches it).
    pub(crate) dt3_present: Dt3Present,
    pub(crate) dt3_members: Vec<Move>,
    pub(crate) dt3_present_log: Vec<(u8, u8, bool)>,
}

impl Default for EvalCaches {
    fn default() -> Self {
        Self::new()
    }
}

impl EvalCaches {
    /// Word-1 bitmask for `ShapeLabel::L5` buckets (`79..=91`, bits `15..=27`).
    pub const L5_BUCKET_WORD1_MASK: u64 = ((1_u64 << 13) - 1) << (79 - 64);
    /// Word-1 bitmask for `ShapeLabel::L4` buckets (`67..=78`, bits `3..=14`).
    pub const L4_BUCKET_WORD1_MASK: u64 = ((1_u64 << 12) - 1) << (67 - 64);
    /// Word-1 bitmask for `L4S + L4S` double-four bucket (`66`, bit `2`).
    pub const DOUBLE_FOUR_BUCKET_WORD1_MASK: u64 = 1_u64 << (66 - 64);
    /// Word-1 bitmask for `L4S + L3B` / `L4S + L3` four-three buckets (`64..=65`, bits `0..=1`).
    pub const FOUR_THREE_BUCKET_WORD1_MASK: u64 = (1_u64 << (64 - 64)) | (1_u64 << (65 - 64));
    /// Word-0 bitmask for `L3B + L3B`, `L3 + L3B`, `L3 + L3` double-three buckets (`45, 54, 55`).
    pub const DOUBLE_THREE_BUCKET_WORD0_MASK: u64 = (1_u64 << 45) | (1_u64 << 54) | (1_u64 << 55);
    /// Combined word-1 bitmask for `L4` and `L5` buckets (`67..=91`).
    pub const L4_L5_BUCKET_WORD1_MASK: u64 =
        Self::L4_BUCKET_WORD1_MASK | Self::L5_BUCKET_WORD1_MASK;

    #[inline(always)]
    pub fn has_l5(&self, side: i8) -> bool {
        let player = if side == BLACK { 0 } else { 1 };
        self.threat_counts[player][1] != 0
    }

    #[inline(always)]
    pub fn has_l4(&self, side: i8) -> bool {
        let player = if side == BLACK { 0 } else { 1 };
        self.threat_counts[player][2] != 0
    }

    #[inline(always)]
    pub fn has_double_four(&self, side: i8) -> bool {
        let player = if side == BLACK { 0 } else { 1 };
        self.threat_counts[player][3] != 0
    }

    #[inline(always)]
    pub fn has_four_three(&self, side: i8) -> bool {
        let player = if side == BLACK { 0 } else { 1 };
        self.threat_counts[player][4] != 0
    }

    #[inline(always)]
    pub fn has_double_three(&self, side: i8) -> bool {
        let player = if side == BLACK { 0 } else { 1 };
        self.threat_counts[player][5] != 0
    }

    #[inline(always)]
    pub fn has_any_l4_or_l5(&self) -> bool {
        (self.threat_counts[0][1]
            | self.threat_counts[0][2]
            | self.threat_counts[1][1]
            | self.threat_counts[1][2])
            != 0
    }

    #[inline(always)]
    pub fn has_attack4_plus(&self, side: i8) -> bool {
        let m = &self.attack4_mask[if side == BLACK { 0 } else { 1 }];
        (m[0] | m[1] | m[2] | m[3]) != 0
    }

    #[inline(always)]
    pub fn l5_count(&self, side: i8) -> u32 {
        let m = &self.l5_mask[if side == BLACK { 0 } else { 1 }];
        m[0].count_ones() + m[1].count_ones() + m[2].count_ones() + m[3].count_ones()
    }

    #[inline(always)]
    pub fn first_l5_move(&self, side: i8) -> Option<Move> {
        let m = &self.l5_mask[if side == BLACK { 0 } else { 1 }];
        for (word_idx, &word) in m.iter().enumerate() {
            if word != 0 {
                return Some(((word_idx << 6) + word.trailing_zeros() as usize) as Move);
            }
        }
        None
    }

    #[inline(always)]
    pub(crate) fn set_attack_masks(&mut self, player: usize, move_idx: usize, attack: i32) {
        let word = move_idx >> 6;
        let bit = 1_u64 << (move_idx & 63);
        if attack >= 4 {
            self.attack4_mask[player][word] |= bit;
            if attack == 6 {
                self.l5_mask[player][word] |= bit;
            } else {
                self.l5_mask[player][word] &= !bit;
            }
        } else {
            self.attack4_mask[player][word] &= !bit;
            self.l5_mask[player][word] &= !bit;
        }
    }

    #[inline(always)]
    pub(crate) fn clear_attack_masks(&mut self, player: usize, move_idx: usize) {
        let word = move_idx >> 6;
        let bit = !(1_u64 << (move_idx & 63));
        self.attack4_mask[player][word] &= bit;
        self.l5_mask[player][word] &= bit;
    }

    pub fn new() -> Self {
        Self {
            initialized: false,
            rule_set: RuleSet::Freestyle,
            board_shadow: new_board_matrix(),
            shape_cache: new_shape_cache(),
            value_cache: new_value_cache(),
            attack_cache: new_value_cache(),
            empty_bucket_counts: new_empty_bucket_counts(),
            nonzero_bucket_mask: [[0; 3]; 2],
            threat_counts: [[0; 6]; 2],
            attack4_mask: [[0; 4]; 2],
            l5_mask: [[0; 4]; 2],
            packed_lines: [[0; 2]; PACKED_LINE_COUNT],
            dgn_cc: [[0; BOARD_SIZE]; BOARD_SIZE],
            dgn_sum: [0; 2],
            shadow_log: Vec::with_capacity(32),
            dgn_cc_log: Vec::with_capacity(64),
            shape_log: Vec::with_capacity(512),
            value_log: Vec::with_capacity(256),
            active_snapshot_count: 0,
            dt3_present: new_dt3_present(),
            dt3_members: Vec::with_capacity(64),
            dt3_present_log: Vec::with_capacity(64),
        }
    }

    #[inline(always)]
    pub fn set_board_shadow(&mut self, x: usize, y: usize, new_cell: i8) {
        let old_cell = self.board_shadow[x][y];
        if old_cell == new_cell {
            return;
        }
        if self.active_snapshot_count > 0 {
            self.shadow_log.push((x as u8, y as u8, old_cell));
        }
        self.board_shadow[x][y] = new_cell;
        update_packed_lines_cell(&mut self.packed_lines, x, y, new_cell);
    }

    #[inline(always)]
    pub(crate) fn packed_lines_through(&self, x: usize, y: usize) -> [PackedLine; 4] {
        let h_idx = x;
        let v_idx = BOARD_SIZE + y;
        let dd_idx = 30 + x + y;
        let du_idx = 59 + x + (BOARD_SIZE - 1 - y);
        [
            PackedLine::from_stones(self.packed_lines[h_idx], h_idx),
            PackedLine::from_stones(self.packed_lines[v_idx], v_idx),
            PackedLine::from_stones(self.packed_lines[dd_idx], dd_idx),
            PackedLine::from_stones(self.packed_lines[du_idx], du_idx),
        ]
    }

    #[inline(always)]
    pub(crate) fn push_shape_log(
        &mut self,
        player: usize,
        x: usize,
        y: usize,
        direction: usize,
        old_value: i32,
    ) {
        self.shape_log
            .push((player as u8, x as u8, y as u8, direction as u8, old_value));
    }

    #[inline(always)]
    pub(crate) fn push_value_log(
        &mut self,
        player: usize,
        x: usize,
        y: usize,
        old_bucket: i32,
        old_attack: i32,
    ) {
        self.value_log.push((
            player as u8,
            x as u8,
            y as u8,
            old_bucket as i16,
            old_attack as i8,
        ));
    }

    #[inline(always)]
    pub(crate) fn inc_empty_bucket(&mut self, player: usize, bucket: usize) {
        debug_assert!(bucket < DSHAPE_SIZE);
        let count = &mut self.empty_bucket_counts[player][bucket];
        if *count == 0 {
            self.nonzero_bucket_mask[player][bucket >> 6] |= 1_u64 << (bucket & 63);
        }
        *count += 1;
        let cls = THREAT_CLASS[bucket] as usize;
        self.threat_counts[player][cls] = self.threat_counts[player][cls].wrapping_add(1);
    }

    #[inline(always)]
    pub(crate) fn dec_empty_bucket(&mut self, player: usize, bucket: usize) {
        debug_assert!(bucket < DSHAPE_SIZE);
        debug_assert!(self.empty_bucket_counts[player][bucket] > 0);
        let count = &mut self.empty_bucket_counts[player][bucket];
        *count -= 1;
        if *count == 0 {
            self.nonzero_bucket_mask[player][bucket >> 6] &= !(1_u64 << (bucket & 63));
        }
        let cls = THREAT_CLASS[bucket] as usize;
        self.threat_counts[player][cls] = self.threat_counts[player][cls].wrapping_sub(1);
    }

    #[inline(always)]
    pub(crate) fn inc_threat_bucket(&mut self, player: usize, bucket: usize) {
        debug_assert!(bucket < DSHAPE_SIZE);
        let cls = THREAT_CLASS[bucket] as usize;
        self.threat_counts[player][cls] = self.threat_counts[player][cls].wrapping_add(1);
    }

    #[inline(always)]
    pub(crate) fn dec_threat_bucket(&mut self, player: usize, bucket: usize) {
        debug_assert!(bucket < DSHAPE_SIZE);
        let cls = THREAT_CLASS[bucket] as usize;
        self.threat_counts[player][cls] = self.threat_counts[player][cls].wrapping_sub(1);
    }

    #[inline(always)]
    pub(crate) fn swap_threat_bucket(
        &mut self,
        player: usize,
        old_bucket: usize,
        new_bucket: usize,
    ) {
        debug_assert!(old_bucket < DSHAPE_SIZE && new_bucket < DSHAPE_SIZE);
        let old_cls = THREAT_CLASS[old_bucket] as usize;
        let new_cls = THREAT_CLASS[new_bucket] as usize;
        if old_cls != new_cls {
            self.threat_counts[player][old_cls] =
                self.threat_counts[player][old_cls].wrapping_sub(1);
            self.threat_counts[player][new_cls] =
                self.threat_counts[player][new_cls].wrapping_add(1);
        }
    }

    #[inline(always)]
    pub(crate) fn set_dgn_cc(&mut self, x: usize, y: usize, new_cc: u8) {
        let old_cc = self.dgn_cc[x][y];
        if old_cc == new_cc {
            return;
        }
        if self.active_snapshot_count > 0 {
            self.dgn_cc_log.push((x as u8, y as u8, old_cc));
        }
        self.dgn_cc[x][y] = new_cc;
    }

    #[inline(always)]
    pub(crate) fn adjust_dgn_cc(&mut self, x: usize, y: usize, player: usize, delta: i8) {
        let old_cc = self.dgn_cc[x][y];
        let new_cc = (old_cc as i16 + i16::from(delta)) as u8;
        if self.active_snapshot_count > 0 {
            self.dgn_cc_log.push((x as u8, y as u8, old_cc));
        }
        self.dgn_cc[x][y] = new_cc;
        self.dgn_sum[player] += dgn_contribution(new_cc) - dgn_contribution(old_cc);
    }

    /// Set the apparent-double-three membership of an empty point. Members are
    /// appended to `dt3_members` on a false->true transition; losing membership
    /// only clears `dt3_present` and leaves a stale list entry (filtered on
    /// iteration).
    pub(crate) fn set_dt3_present(&mut self, x: usize, y: usize, value: bool) {
        if self.dt3_present[x][y] == value {
            return;
        }
        if self.active_snapshot_count > 0 {
            self.dt3_present_log
                .push((x as u8, y as u8, self.dt3_present[x][y]));
        }
        self.dt3_present[x][y] = value;
        if value {
            self.dt3_members.push((y * BOARD_SIZE + x) as Move);
        }
    }

    pub fn set_shape_value(
        &mut self,
        player: usize,
        x: usize,
        y: usize,
        direction: usize,
        value: i32,
    ) {
        let old_value = self.shape_cache[player][x][y][direction];
        if old_value == value {
            return;
        }
        if self.active_snapshot_count > 0 {
            self.push_shape_log(player, x, y, direction, old_value);
        }
        self.shape_cache[player][x][y][direction] = value;
    }

    pub fn snapshot(&mut self) -> EvalSnapshot {
        self.active_snapshot_count += 1;
        EvalSnapshot {
            initialized: self.initialized,
            rule_set: self.rule_set,
            empty_bucket_counts: self.empty_bucket_counts,
            nonzero_bucket_mask: self.nonzero_bucket_mask,
            threat_counts: self.threat_counts,
            attack4_mask: self.attack4_mask,
            l5_mask: self.l5_mask,
            dgn_sum: self.dgn_sum,
            shadow_log_len: self.shadow_log.len(),
            dgn_cc_log_len: self.dgn_cc_log.len(),
            shape_log_len: self.shape_log.len(),
            value_log_len: self.value_log.len(),
            dt3_present_log_len: self.dt3_present_log.len(),
            dt3_members_len: self.dt3_members.len(),
        }
    }

    pub fn restore_snapshot(&mut self, snapshot: &EvalSnapshot) {
        self.initialized = snapshot.initialized;
        self.rule_set = snapshot.rule_set;
        self.empty_bucket_counts = snapshot.empty_bucket_counts;
        self.nonzero_bucket_mask = snapshot.nonzero_bucket_mask;
        self.threat_counts = snapshot.threat_counts;
        self.attack4_mask = snapshot.attack4_mask;
        self.l5_mask = snapshot.l5_mask;
        self.dgn_sum = snapshot.dgn_sum;

        while self.shadow_log.len() > snapshot.shadow_log_len {
            let (x, y, old_cell) = self.shadow_log.pop().expect("shadow log length tracked");
            let ux = x as usize;
            let uy = y as usize;
            self.board_shadow[ux][uy] = old_cell;
            update_packed_lines_cell(&mut self.packed_lines, ux, uy, old_cell);
        }
        while self.dgn_cc_log.len() > snapshot.dgn_cc_log_len {
            let (x, y, old_cc) = self.dgn_cc_log.pop().expect("dgn cc log length tracked");
            self.dgn_cc[x as usize][y as usize] = old_cc;
        }
        while self.shape_log.len() > snapshot.shape_log_len {
            let (player, x, y, direction, old_value) =
                self.shape_log.pop().expect("shape log length tracked");
            self.shape_cache[player as usize][x as usize][y as usize][direction as usize] =
                old_value;
        }
        while self.value_log.len() > snapshot.value_log_len {
            let (player, x, y, old_bucket, old_attack) =
                self.value_log.pop().expect("value log length tracked");
            self.value_cache[player as usize][x as usize][y as usize] = i32::from(old_bucket);
            self.attack_cache[player as usize][x as usize][y as usize] = i32::from(old_attack);
        }
        while self.dt3_present_log.len() > snapshot.dt3_present_log_len {
            let (x, y, old_value) = self
                .dt3_present_log
                .pop()
                .expect("dt3 present log length tracked");
            self.dt3_present[x as usize][y as usize] = old_value;
        }
        self.dt3_members.truncate(snapshot.dt3_members_len);
        if self.active_snapshot_count > 0 {
            self.active_snapshot_count -= 1;
        }
    }

    pub fn snapshot_nnue(&mut self) -> EvalSnapshotNnue {
        self.active_snapshot_count += 1;
        EvalSnapshotNnue {
            initialized: self.initialized,
            rule_set: self.rule_set,
            threat_counts: self.threat_counts,
            attack4_mask: self.attack4_mask,
            l5_mask: self.l5_mask,
            shadow_log_len: self.shadow_log.len(),
            shape_log_len: self.shape_log.len(),
            value_log_len: self.value_log.len(),
            dt3_present_log_len: self.dt3_present_log.len(),
            dt3_members_len: self.dt3_members.len(),
        }
    }

    pub fn restore_snapshot_nnue(&mut self, snapshot: &EvalSnapshotNnue) {
        self.initialized = snapshot.initialized;
        self.rule_set = snapshot.rule_set;
        self.threat_counts = snapshot.threat_counts;
        self.attack4_mask = snapshot.attack4_mask;
        self.l5_mask = snapshot.l5_mask;

        while self.shadow_log.len() > snapshot.shadow_log_len {
            let (x, y, old_cell) = self.shadow_log.pop().expect("shadow log length tracked");
            let ux = x as usize;
            let uy = y as usize;
            self.board_shadow[ux][uy] = old_cell;
            update_packed_lines_cell(&mut self.packed_lines, ux, uy, old_cell);
        }
        while self.shape_log.len() > snapshot.shape_log_len {
            let (player, x, y, direction, old_value) =
                self.shape_log.pop().expect("shape log length tracked");
            self.shape_cache[player as usize][x as usize][y as usize][direction as usize] =
                old_value;
        }
        while self.value_log.len() > snapshot.value_log_len {
            let (player, x, y, old_bucket, old_attack) =
                self.value_log.pop().expect("value log length tracked");
            self.value_cache[player as usize][x as usize][y as usize] = i32::from(old_bucket);
            self.attack_cache[player as usize][x as usize][y as usize] = i32::from(old_attack);
        }
        while self.dt3_present_log.len() > snapshot.dt3_present_log_len {
            let (x, y, old_value) = self
                .dt3_present_log
                .pop()
                .expect("dt3 present log length tracked");
            self.dt3_present[x as usize][y as usize] = old_value;
        }
        self.dt3_members.truncate(snapshot.dt3_members_len);
        if self.active_snapshot_count > 0 {
            self.active_snapshot_count -= 1;
        }
    }

    pub fn copy(&self) -> Self {
        self.clone()
    }

    pub fn shape_log_len(&self) -> usize {
        self.shape_log.len()
    }

    pub fn value_log_len(&self) -> usize {
        self.value_log.len()
    }

    pub fn restore_from(&mut self, other: &Self) {
        self.initialized = other.initialized;
        self.rule_set = other.rule_set;
        self.board_shadow = other.board_shadow;
        self.shape_cache = other.shape_cache;
        self.value_cache = other.value_cache;
        self.attack_cache = other.attack_cache;
        self.empty_bucket_counts = other.empty_bucket_counts;
        self.nonzero_bucket_mask = other.nonzero_bucket_mask;
        self.threat_counts = other.threat_counts;
        self.attack4_mask = other.attack4_mask;
        self.l5_mask = other.l5_mask;
        self.packed_lines = other.packed_lines;
        self.dgn_cc = other.dgn_cc;
        self.dgn_sum = other.dgn_sum;
        self.shadow_log.clear();
        self.dgn_cc_log.clear();
        self.dt3_present = other.dt3_present;
        self.dt3_members.clear();
        self.dt3_members.extend_from_slice(&other.dt3_members);
        self.dt3_present_log.clear();
        self.shape_log.clear();
        self.value_log.clear();
        self.active_snapshot_count = 0;
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Drop all apparent-double-three membership, for a full from-scratch
    /// rebuild. Only safe with no active snapshot (it discards the list that
    /// snapshots truncate against).
    pub(crate) fn clear_dt3(&mut self) {
        self.dt3_present = new_dt3_present();
        self.dt3_members.clear();
        self.dt3_present_log.clear();
    }
}

fn new_board_matrix() -> BoardShadow {
    [[0; BOARD_SIZE]; BOARD_SIZE]
}

fn new_shape_cache() -> ShapeCache {
    [[[[0; 4]; BOARD_SIZE]; BOARD_SIZE]; 2]
}

fn new_value_cache() -> ValueCache {
    [[[0; BOARD_SIZE]; BOARD_SIZE]; 2]
}

fn new_empty_bucket_counts() -> EmptyBucketCounts {
    [[0; DSHAPE_SIZE]; 2]
}

fn new_dt3_present() -> Dt3Present {
    [[false; BOARD_SIZE]; BOARD_SIZE]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::buckets::bucket_for_lines;

    #[test]
    fn l4_and_l5_bucket_masks_match_exact_bucket_ranges() {
        for top1 in 0..=12 {
            for top2 in 0..=top1 {
                let bucket = bucket_for_lines(top1, top2).expect("valid bucket") as usize;
                let mut caches = EvalCaches::new();
                caches.inc_empty_bucket(0, bucket);
                assert_eq!(
                    caches.has_l5(BLACK),
                    top1 == 12,
                    "L5 mask mismatch at top1={top1}, top2={top2}, bucket={bucket}"
                );
                assert_eq!(
                    caches.has_l4(BLACK),
                    top1 == 11,
                    "L4 mask mismatch at top1={top1}, top2={top2}, bucket={bucket}"
                );
                assert_eq!(
                    caches.has_any_l4_or_l5(),
                    top1 >= 11,
                    "L4/L5 mask mismatch at top1={top1}, top2={top2}, bucket={bucket}"
                );
                assert_eq!(
                    caches.has_double_four(BLACK),
                    top1 == 10 && top2 == 10,
                    "4-4 mask mismatch at top1={top1}, top2={top2}, bucket={bucket}"
                );
                assert_eq!(
                    caches.has_four_three(BLACK),
                    top1 == 10 && (top2 == 8 || top2 == 9),
                    "4-3 mask mismatch at top1={top1}, top2={top2}, bucket={bucket}"
                );
                assert_eq!(
                    caches.has_double_three(BLACK),
                    (top1 == 8 || top1 == 9) && (top2 == 8 || top2 == 9),
                    "3-3 mask mismatch at top1={top1}, top2={top2}, bucket={bucket}"
                );
                assert!(!caches.has_l5(WHITE));
                assert!(!caches.has_l4(WHITE));
                assert!(!caches.has_double_four(WHITE));
                assert!(!caches.has_four_three(WHITE));
                assert!(!caches.has_double_three(WHITE));
            }
        }
    }
}
