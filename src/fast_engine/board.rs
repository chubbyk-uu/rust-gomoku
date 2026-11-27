//! Zero-allocation 15x15 `FastBoard` with incremental dense-key `Pattern` /
//! 14-level `Pattern4` maintenance, candidate bitboards, and Renju forbidden
//! point verification.

use std::sync::LazyLock;

use crate::constants::{BOARD_AREA, BOARD_SIZE};
use crate::fast_engine::classical_model::{
    classical_model, table_index, ClassicalModel, CLASSICAL_VALUE_BLEND_WEIGHTS,
};
use crate::fast_engine::pattern::{
    lookup_pattern, pcode_table_for, tables, Pattern, Pattern2x, Pattern4, PatternTables, FE_BLACK,
    FE_WHITE, HALF_LEN_FREESTYLE, HALF_LEN_RENJU, PATTERN4_NB,
};
use crate::fast_engine::tt::{VALUE_EVAL_MAX, VALUE_EVAL_MIN};
use crate::rules::RuleSet;

/// Sentinel move representing a pass (used for Null Move Pruning).
pub const PASS_MOVE: u16 = u16::MAX;
/// Sentinel representing "no move".
pub const NONE_MOVE: u16 = u16::MAX - 1;

/// Wall padding cells on the low end of each 64-bit line bitkey (`6` wall cells
/// below `0..15`, `11` wall cells above `0..15` -> 32 cells = 64 bits).
const LINE_PAD: usize = 6;

/// Linear step in `pos = y * 15 + x` for the 4 line directions:
/// - `0`: horizontal (`+1,  0`)
/// - `1`: vertical   (` 0, +1`)
/// - `2`: anti-diag  (`+1, -1`)  (`x + y` constant)
/// - `3`: main-diag  (`+1, +1`)  (`14 - x + y` constant)
pub const DIR_STEP: [isize; 4] = [
    1,
    BOARD_SIZE as isize,
    -(BOARD_SIZE as isize) + 1,
    BOARD_SIZE as isize + 1,
];

/// Maximum number of plies stored in `FastBoard` history (225 moves + passes).
const MAX_HIST: usize = 320;
/// Maximum number of candidate bitboard word deltas across an entire game.
const MAX_CAND_JOURNAL: usize = 512;

/// Candidate neighborhood radius around each played stone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandRange {
    /// 5x5 square plus 8-direction line out to 3 cells.
    Square2Line3,
    /// 7x7 square plus 8-direction line out to 4 cells (Rapfi default).
    Square3Line4,
}

/// 225-bit board bitset packed into 4 `u64` words (32 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bitboard225 {
    pub words: [u64; 4],
}

impl Bitboard225 {
    #[inline(always)]
    pub const fn empty() -> Self {
        Self { words: [0; 4] }
    }

    #[inline(always)]
    pub const fn all_on_board() -> Self {
        // 225 = 64 + 64 + 64 + 33
        Self {
            words: [u64::MAX, u64::MAX, u64::MAX, (1_u64 << 33) - 1],
        }
    }

    #[inline(always)]
    pub fn test(&self, pos: usize) -> bool {
        ((self.words[pos >> 6] >> (pos & 63)) & 1) != 0
    }

    #[inline(always)]
    pub fn set(&mut self, pos: usize) {
        self.words[pos >> 6] |= 1_u64 << (pos & 63);
    }

    #[inline(always)]
    pub fn clear(&mut self, pos: usize) {
        self.words[pos >> 6] &= !(1_u64 << (pos & 63));
    }

    #[inline(always)]
    pub fn and(&self, other: &Self) -> Self {
        Self {
            words: [
                self.words[0] & other.words[0],
                self.words[1] & other.words[1],
                self.words[2] & other.words[2],
                self.words[3] & other.words[3],
            ],
        }
    }

    #[inline(always)]
    pub fn count_ones(&self) -> u32 {
        self.words[0].count_ones()
            + self.words[1].count_ones()
            + self.words[2].count_ones()
            + self.words[3].count_ones()
    }

    #[inline(always)]
    pub fn for_each_bit(&self, mut f: impl FnMut(usize)) {
        for (w_idx, &word) in self.words.iter().enumerate() {
            let mut w = word;
            let base = w_idx << 6;
            while w != 0 {
                let tz = w.trailing_zeros() as usize;
                f(base + tz);
                w &= w - 1;
            }
        }
    }
}

const fn build_cand_stencils(square_r: isize, line_r: isize) -> [Bitboard225; BOARD_AREA] {
    let mut table = [Bitboard225::empty(); BOARD_AREA];
    let mut idx = 0_usize;
    while idx < BOARD_AREA {
        let cx = (idx % BOARD_SIZE) as isize;
        let cy = (idx / BOARD_SIZE) as isize;
        let mut words = [0_u64; 4];

        // Square neighborhood
        let mut dy = -square_r;
        while dy <= square_r {
            let mut dx = -square_r;
            while dx <= square_r {
                if dx != 0 || dy != 0 {
                    let nx = cx + dx;
                    let ny = cy + dy;
                    if nx >= 0 && nx < BOARD_SIZE as isize && ny >= 0 && ny < BOARD_SIZE as isize {
                        let nidx = ny as usize * BOARD_SIZE + nx as usize;
                        words[nidx >> 6] |= 1_u64 << (nidx & 63);
                    }
                }
                dx += 1;
            }
            dy += 1;
        }

        // 8-direction line rays out to line_r
        const DIRS8: [(isize, isize); 8] = [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
        ];
        let mut d = 0_usize;
        while d < 8 {
            let (dx, dy) = DIRS8[d];
            let mut dist = square_r + 1;
            while dist <= line_r {
                let nx = cx + dx * dist;
                let ny = cy + dy * dist;
                if nx >= 0 && nx < BOARD_SIZE as isize && ny >= 0 && ny < BOARD_SIZE as isize {
                    let nidx = ny as usize * BOARD_SIZE + nx as usize;
                    words[nidx >> 6] |= 1_u64 << (nidx & 63);
                }
                dist += 1;
            }
            d += 1;
        }

        table[idx] = Bitboard225 { words };
        idx += 1;
    }
    table
}

static CAND_STENCIL_S2L3: [Bitboard225; BOARD_AREA] = build_cand_stencils(2, 3);
static CAND_STENCIL_S3L4: [Bitboard225; BOARD_AREA] = build_cand_stencils(3, 4);

struct ZobristKeys {
    piece: [[u64; BOARD_AREA]; 2],
    side_to_move: u64,
    initial: u64,
}

static ZOBRIST: LazyLock<ZobristKeys> = LazyLock::new(|| {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next_u64 = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut piece = [[0_u64; BOARD_AREA]; 2];
    for side in 0..2 {
        for sq in 0..BOARD_AREA {
            piece[side][sq] = next_u64();
        }
    }
    let side_to_move = next_u64();
    let initial = next_u64();
    ZobristKeys {
        piece,
        side_to_move,
        initial,
    }
});

#[derive(Clone, Copy, Debug, Default)]
struct CandJournalEntry {
    word_idx: u8,
    delta: u64,
}

#[derive(Clone, Copy, Debug)]
struct StateInfo {
    journal_start: u16,
    last_move: u16,
    last_actual_move: [u16; 2],
    last_flex4_attack: [u16; 2],
    /// Most recent empty cell reaching `[CBlock4Flex3, BFlex4, AFive]` per side.
    last_p4_move: [[u16; 3]; 2],
    p4_count: [[u16; PATTERN4_NB]; 2],
    value_black: i32,
}

impl Default for StateInfo {
    fn default() -> Self {
        Self {
            journal_start: 0,
            last_move: NONE_MOVE,
            last_actual_move: [NONE_MOVE; 2],
            last_flex4_attack: [NONE_MOVE; 2],
            last_p4_move: [[NONE_MOVE; 3]; 2],
            p4_count: [[0; PATTERN4_NB]; 2],
            value_black: 0,
        }
    }
}

/// Bounds-safe step from `pos` along line direction `dir` (`0..4`) by `step` cells.
#[inline(always)]
pub fn step_pos(pos: usize, dir: usize, step: isize) -> Option<usize> {
    const DIRS: [(isize, isize); 4] = [(1, 0), (0, 1), (1, -1), (1, 1)];
    let (dx, dy) = DIRS[dir];
    let nx = (pos % BOARD_SIZE) as isize + dx * step;
    let ny = (pos / BOARD_SIZE) as isize + dy * step;
    if nx >= 0 && nx < BOARD_SIZE as isize && ny >= 0 && ny < BOARD_SIZE as isize {
        Some((ny as usize) * BOARD_SIZE + (nx as usize))
    } else {
        None
    }
}

/// Chebyshev distance matching Rapfi `Pos::distance`: returns `-1` if either
/// operand is `NONE_MOVE` or `PASS_MOVE`.
#[inline(always)]
pub fn chebyshev_distance(a: u16, b: u16) -> i32 {
    if (a as usize) >= BOARD_AREA || (b as usize) >= BOARD_AREA {
        return -1;
    }
    let ax = (a as usize % BOARD_SIZE) as i32;
    let ay = (a as usize / BOARD_SIZE) as i32;
    let bx = (b as usize % BOARD_SIZE) as i32;
    let by = (b as usize / BOARD_SIZE) as i32;
    (ax - bx).abs().max((ay - by).abs())
}

#[derive(Clone, Copy, Debug)]
struct MoveRestore {
    len: u8,
    /// Packed `u32`: `posi (8) | (dir << 8) | (old_p2x << 16) | (old_p4 << 24)`.
    entries: [u32; 40],
}

impl Default for MoveRestore {
    fn default() -> Self {
        Self {
            len: 0,
            entries: [0; 40],
        }
    }
}

#[inline(always)]
const fn pack_p4_pair(black: u8, white: u8) -> u8 {
    (black & 0x0F) | ((white & 0x0F) << 4)
}

/// Zero-allocation incremental 15x15 board state aligned with Rapfi `Board`.
#[derive(Clone)]
pub struct FastBoard {
    rule: RuleSet,
    cand_range: CandRange,
    tables: &'static PatternTables,
    classical: Option<&'static ClassicalModel>,
    bit_key0: [u64; BOARD_SIZE],
    bit_key1: [u64; BOARD_SIZE],
    bit_key2: [u64; 2 * BOARD_SIZE - 1],
    bit_key3: [u64; 2 * BOARD_SIZE - 1],
    pattern2xs: [[Pattern2x; 4]; BOARD_AREA],
    pattern4x2: [u8; BOARD_AREA],
    empty_bb: Bitboard225,
    candidates_bb: Bitboard225,
    cand_journal: [CandJournalEntry; MAX_CAND_JOURNAL],
    journal_top: usize,
    state_infos: Vec<StateInfo>,
    move_restores: Vec<MoveRestore>,
    move_count: usize,
    pass_count: [usize; 2],
    current_side: usize,
    zobrist_key: u64,
}

#[cfg(test)]
pub(crate) static TEST_FORBID_CALLS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

impl FastBoard {
    pub fn new(rule: RuleSet, cand_range: CandRange) -> Self {
        let mut board = Self {
            rule,
            cand_range,
            tables: tables(),
            classical: classical_model(),
            bit_key0: [0; BOARD_SIZE],
            bit_key1: [0; BOARD_SIZE],
            bit_key2: [0; 2 * BOARD_SIZE - 1],
            bit_key3: [0; 2 * BOARD_SIZE - 1],
            pattern2xs: [[Pattern2x::default(); 4]; BOARD_AREA],
            pattern4x2: [0; BOARD_AREA],
            empty_bb: Bitboard225::all_on_board(),
            candidates_bb: Bitboard225::empty(),
            cand_journal: [CandJournalEntry::default(); MAX_CAND_JOURNAL],
            journal_top: 0,
            state_infos: vec![StateInfo::default(); MAX_HIST],
            move_restores: vec![MoveRestore::default(); MAX_HIST],
            move_count: 0,
            pass_count: [0; 2],
            current_side: FE_BLACK,
            zobrist_key: ZOBRIST.initial,
        };
        board.reset(rule);
        board
    }

    pub fn reset(&mut self, rule: RuleSet) {
        self.rule = rule;
        self.classical = classical_model();
        self.bit_key0 = [0; BOARD_SIZE];
        self.bit_key1 = [0; BOARD_SIZE];
        self.bit_key2 = [0; 2 * BOARD_SIZE - 1];
        self.bit_key3 = [0; 2 * BOARD_SIZE - 1];
        self.empty_bb = Bitboard225::all_on_board();
        self.candidates_bb = Bitboard225::empty();
        self.journal_top = 0;
        self.move_count = 0;
        self.pass_count = [0; 2];
        self.current_side = FE_BLACK;
        self.zobrist_key = ZOBRIST.initial;

        // Seed all 225 on-board cells with 0b11 (empty); walls stay 0b00.
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                let sx = 2 * (x + LINE_PAD);
                let sy = 2 * (y + LINE_PAD);
                self.bit_key0[y] |= 0b11_u64 << sx;
                self.bit_key1[x] |= 0b11_u64 << sy;
                self.bit_key2[x + y] |= 0b11_u64 << sx;
                self.bit_key3[BOARD_SIZE - 1 - x + y] |= 0b11_u64 << sx;
            }
        }

        if rule == RuleSet::Renju {
            self.init_patterns::<true>();
        } else {
            self.init_patterns::<false>();
        }
    }

    fn init_patterns<const RENJU: bool>(&mut self) {
        let mut st = StateInfo::default();
        let t = self.tables;
        let pc_b = pcode_table_for::<RENJU>(t, FE_BLACK);
        let pc_w = pcode_table_for::<RENJU>(t, FE_WHITE);
        let mut value_black = 0_i32;

        for pos in 0..BOARD_AREA {
            let mut p2x = [Pattern2x::default(); 4];
            for dir in 0..4 {
                let key = self.get_key_at::<RENJU>(pos, dir);
                p2x[dir] = lookup_pattern::<RENJU>(t, key);
            }
            self.pattern2xs[pos] = p2x;
            let idx_b = (p2x[0].black_raw() << 12)
                | (p2x[1].black_raw() << 8)
                | (p2x[2].black_raw() << 4)
                | p2x[3].black_raw();
            let idx_w = (p2x[0].white_raw() << 12)
                | (p2x[1].white_raw() << 8)
                | (p2x[2].white_raw() << 4)
                | p2x[3].white_raw();
            let cp_b = pc_b[idx_b];
            let cp_w = pc_w[idx_w];
            let p4_b = cp_b.pattern4_raw();
            let p4_w = cp_w.pattern4_raw();
            self.pattern4x2[pos] = pack_p4_pair(p4_b, p4_w);
            st.p4_count[FE_BLACK][p4_b as usize] += 1;
            st.p4_count[FE_WHITE][p4_w as usize] += 1;
            if let Some(cm) = self.classical {
                value_black += cm.get_value_black::<RENJU>(cp_b.pcode(), cp_w.pcode());
            }
        }
        st.value_black = value_black;
        self.state_infos[0] = st;
    }

    #[inline(always)]
    pub fn rule(&self) -> RuleSet {
        self.rule
    }

    #[inline(always)]
    pub fn side_to_move(&self) -> usize {
        self.current_side
    }

    #[inline(always)]
    pub fn move_count(&self) -> usize {
        self.move_count
    }

    #[inline(always)]
    pub fn non_pass_move_count(&self) -> usize {
        self.move_count - self.pass_count[FE_BLACK] - self.pass_count[FE_WHITE]
    }

    #[inline(always)]
    pub fn moves_left(&self) -> usize {
        BOARD_AREA.saturating_sub(self.non_pass_move_count())
    }

    #[inline(always)]
    pub fn zobrist_key(&self) -> u64 {
        self.zobrist_key
    }

    #[inline(always)]
    pub fn zobrist_key_after(&self, pos: usize) -> u64 {
        self.zobrist_key ^ ZOBRIST.piece[self.current_side][pos] ^ ZOBRIST.side_to_move
    }

    #[inline(always)]
    pub fn zobrist_key_after_pass(&self) -> u64 {
        self.zobrist_key ^ ZOBRIST.side_to_move
    }

    #[inline(always)]
    pub fn is_empty(&self, pos: usize) -> bool {
        self.empty_bb.test(pos)
    }

    /// Raw 2-bit occupancy code at `pos`: `0b11`=empty, `0b10`=black, `0b01`=white.
    #[inline(always)]
    pub fn cell_bits(&self, pos: usize) -> u8 {
        let x = pos % BOARD_SIZE;
        let y = pos / BOARD_SIZE;
        ((self.bit_key0[y] >> (2 * (x + LINE_PAD))) & 0b11) as u8
    }

    #[inline(always)]
    pub fn is_black(&self, pos: usize) -> bool {
        self.cell_bits(pos) == 0b10
    }

    #[inline(always)]
    pub fn is_white(&self, pos: usize) -> bool {
        self.cell_bits(pos) == 0b01
    }

    #[inline(always)]
    pub fn pattern4(&self, pos: usize, side: usize) -> Pattern4 {
        Pattern4::from_u8((self.pattern4x2[pos] >> (side * 4)) & 0x0F)
    }

    #[inline(always)]
    pub fn pattern(&self, pos: usize, side: usize, dir: usize) -> Pattern {
        self.pattern2xs[pos][dir].for_side(side)
    }

    #[inline(always)]
    pub fn pcode_pair(&self, pos: usize) -> (u16, u16) {
        let p2x = self.pattern2xs[pos];
        let idx_b = (p2x[0].black_raw() << 12)
            | (p2x[1].black_raw() << 8)
            | (p2x[2].black_raw() << 4)
            | p2x[3].black_raw();
        let idx_w = (p2x[0].white_raw() << 12)
            | (p2x[1].white_raw() << 8)
            | (p2x[2].white_raw() << 4)
            | p2x[3].white_raw();
        (
            self.tables.pcode[idx_b].pcode(),
            self.tables.pcode[idx_w].pcode(),
        )
    }

    #[inline(always)]
    pub fn p4_count(&self, side: usize, p4: Pattern4) -> u16 {
        self.state_infos[self.move_count].p4_count[side][p4 as usize]
    }

    #[inline(always)]
    pub fn last_move(&self) -> u16 {
        self.state_infos[self.move_count].last_move
    }

    #[inline(always)]
    pub fn last_actual_move(&self, side: usize) -> u16 {
        self.state_infos[self.move_count].last_actual_move[side]
    }

    #[inline(always)]
    pub fn tables(&self) -> &'static PatternTables {
        self.tables
    }

    #[inline(always)]
    pub fn classical(&self) -> Option<&'static ClassicalModel> {
        self.classical
    }

    /// Compute Rapfi's blended classical evaluation in `[-6000, 6000]` from
    /// incremental `value_black` and 11-bit threat mask (`eval/eval.cpp:34-255`).
    #[inline]
    pub fn evaluate_classical<const RENJU: bool>(&self) -> Option<i32> {
        let cm = self.classical?;
        let self_side = self.current_side;
        let oppo = self_side ^ 1;
        let st0 = &self.state_infos[self.move_count];
        let st1 = &self.state_infos[self.move_count.saturating_sub(1)];

        let cur_basic = if self_side == FE_BLACK {
            st0.value_black
        } else {
            -st0.value_black
        };
        let prev_basic = if self_side == FE_BLACK {
            st1.value_black
        } else {
            -st1.value_black
        };

        let p4_s = &st0.p4_count[self_side];
        let p4_o = &st0.p4_count[oppo];
        let mut mask = 0_usize;
        mask |= usize::from(p4_o[Pattern4::AFive as usize] != 0);
        mask |= usize::from(p4_s[Pattern4::BFlex4 as usize] != 0) << 1;
        mask |= usize::from(p4_o[Pattern4::BFlex4 as usize] != 0) << 2;
        mask |= usize::from(
            p4_s[Pattern4::DBlock4Plus as usize] + p4_s[Pattern4::CBlock4Flex3 as usize] != 0,
        ) << 3;
        mask |= usize::from(p4_s[Pattern4::EBlock4 as usize] != 0) << 4;
        mask |= usize::from(
            p4_s[Pattern4::GFlex3Plus as usize] + p4_s[Pattern4::FFlex3x2 as usize] != 0,
        ) << 5;
        mask |= usize::from(p4_s[Pattern4::HFlex3 as usize] != 0) << 6;
        mask |= usize::from(
            p4_o[Pattern4::DBlock4Plus as usize] + p4_o[Pattern4::CBlock4Flex3 as usize] != 0,
        ) << 7;
        mask |= usize::from(p4_o[Pattern4::EBlock4 as usize] != 0) << 8;
        mask |= usize::from(
            p4_o[Pattern4::GFlex3Plus as usize] + p4_o[Pattern4::FFlex3x2 as usize] != 0,
        ) << 9;
        mask |= usize::from(p4_o[Pattern4::HFlex3 as usize] != 0) << 10;

        let t_idx = table_index::<RENJU>(self_side);
        let cur_threat = cm.evals_threat[t_idx][mask] as i32;
        let w = &CLASSICAL_VALUE_BLEND_WEIGHTS[t_idx];
        let weighted = (w[0] as i64) * (cur_basic as i64)
            + (w[1] as i64) * (prev_basic as i64)
            + (w[2] as i64) * (cur_threat as i64);
        Some(((weighted / 256) as i32).clamp(VALUE_EVAL_MIN, VALUE_EVAL_MAX))
    }

    /// Check whether `pos` is a legal non-pass move for `side_to_move()`.
    #[inline]
    pub fn is_legal_non_pass(&mut self, pos: u16) -> bool {
        if (pos as usize) >= BOARD_AREA || !self.is_empty(pos as usize) {
            return false;
        }
        if self.rule == RuleSet::Renju && self.current_side == FE_BLACK {
            return !self.check_forbidden_point(pos as usize);
        }
        true
    }

    /// Temporarily switch side to `side`, place a stone at `pos` without flipping
    /// side, run `f(self)`, then undo and restore the original side.
    #[inline]
    pub fn with_probe_move<const RENJU: bool, R>(
        &mut self,
        side: usize,
        pos: usize,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let saved_side = self.current_side;
        self.current_side = side;
        self.play_typed::<RENJU, false>(pos);
        let res = f(self);
        self.undo_typed::<RENJU, false>();
        self.current_side = saved_side;
        res
    }

    #[inline(always)]
    pub fn last_p4_move(&self, side: usize, p4: Pattern4) -> u16 {
        debug_assert!(p4 >= Pattern4::CBlock4Flex3 && p4 <= Pattern4::AFive);
        self.state_infos[self.move_count].last_p4_move[side]
            [p4 as usize - Pattern4::CBlock4Flex3 as usize]
    }

    #[inline(always)]
    pub fn last_flex4_attack(&self, side: usize) -> u16 {
        self.state_infos[self.move_count].last_flex4_attack[side]
    }

    #[inline(always)]
    pub fn empty_candidates(&self) -> Bitboard225 {
        self.candidates_bb.and(&self.empty_bb)
    }

    #[inline(always)]
    pub fn empty_cells(&self) -> &Bitboard225 {
        &self.empty_bb
    }

    #[inline(always)]
    pub fn is_empty_candidate(&self, pos: usize) -> bool {
        self.empty_bb.test(pos) && self.candidates_bb.test(pos)
    }

    #[inline(always)]
    pub fn get_key_at<const RENJU: bool>(&self, pos: usize, dir: usize) -> u64 {
        let l = if RENJU {
            HALF_LEN_RENJU
        } else {
            HALF_LEN_FREESTYLE
        };
        let x = pos % BOARD_SIZE;
        let y = pos / BOARD_SIZE;
        match dir {
            0 => self.bit_key0[y] >> (2 * (x + LINE_PAD - l)),
            1 => self.bit_key1[x] >> (2 * (y + LINE_PAD - l)),
            2 => self.bit_key2[x + y] >> (2 * (x + LINE_PAD - l)),
            _ => self.bit_key3[BOARD_SIZE - 1 - x + y] >> (2 * (x + LINE_PAD - l)),
        }
    }

    #[inline(always)]
    fn flip_bit_key(&mut self, pos: usize, side: usize) {
        let x = pos % BOARD_SIZE;
        let y = pos / BOARD_SIZE;
        let bit_x = 1_u64 << (2 * (x + LINE_PAD) + side);
        let bit_y = 1_u64 << (2 * (y + LINE_PAD) + side);
        self.bit_key0[y] ^= bit_x;
        self.bit_key1[x] ^= bit_y;
        self.bit_key2[x + y] ^= bit_x;
        self.bit_key3[BOARD_SIZE - 1 - x + y] ^= bit_x;
    }

    #[inline]
    pub fn play_pass(&mut self) {
        debug_assert!(self.move_count + 1 < MAX_HIST);
        let prev = self.state_infos[self.move_count];
        self.move_count += 1;
        let st = &mut self.state_infos[self.move_count];
        *st = prev;
        st.last_move = PASS_MOVE;
        st.journal_start = self.journal_top as u16;
        self.pass_count[self.current_side] += 1;
        self.current_side ^= 1;
        self.zobrist_key ^= ZOBRIST.side_to_move;
    }

    #[inline]
    pub fn play(&mut self, pos: usize) {
        if self.rule == RuleSet::Renju {
            self.play_typed::<true, true>(pos);
        } else {
            self.play_typed::<false, true>(pos);
        }
    }

    #[inline]
    pub fn undo(&mut self) {
        if self.rule == RuleSet::Renju {
            self.undo_typed::<true, true>();
        } else {
            self.undo_typed::<false, true>();
        }
    }

    /// Monomorphized hot-path move placement.
    /// When `FLIP_SIDE` is `false`, side-to-move is left unchanged (used for
    /// Renju recursive forbidden-point probing).
    #[inline]
    pub fn play_typed<const RENJU: bool, const FLIP_SIDE: bool>(&mut self, pos: usize) {
        debug_assert!(pos < BOARD_AREA);
        debug_assert!(self.empty_bb.test(pos));
        debug_assert!(self.move_count + 1 < MAX_HIST);

        let side = self.current_side;
        let f4_before_b =
            self.state_infos[self.move_count].p4_count[FE_BLACK][Pattern4::BFlex4 as usize];
        let f4_before_w =
            self.state_infos[self.move_count].p4_count[FE_WHITE][Pattern4::BFlex4 as usize];

        let prev_st = self.state_infos[self.move_count];
        let restore_slot = self.move_count;
        self.move_count += 1;
        let mut st = prev_st;
        st.last_move = pos as u16;
        st.last_actual_move[side] = pos as u16;
        st.journal_start = self.journal_top as u16;

        self.zobrist_key ^= ZOBRIST.piece[side][pos];
        if FLIP_SIDE {
            self.zobrist_key ^= ZOBRIST.side_to_move;
        }
        self.flip_bit_key(pos, side);
        self.empty_bb.clear(pos);

        let l: isize = if RENJU {
            HALF_LEN_RENJU as isize
        } else {
            HALF_LEN_FREESTYLE as isize
        };
        let x = pos % BOARD_SIZE;
        let y = pos / BOARD_SIZE;
        let shift_x = ((2 * ((x + LINE_PAD) as isize - 2 * l)).rem_euclid(64)) as u32;
        let shift_y = ((2 * ((y + LINE_PAD) as isize - 2 * l)).rem_euclid(64)) as u32;
        let mut bit_key = [
            self.bit_key0[y].rotate_right(shift_x),
            self.bit_key1[x].rotate_right(shift_y),
            self.bit_key2[x + y].rotate_right(shift_x),
            self.bit_key3[BOARD_SIZE - 1 - x + y].rotate_right(shift_x),
        ];

        let t = self.tables;
        let pc_b = pcode_table_for::<RENJU>(t, FE_BLACK);
        let pc_w = pcode_table_for::<RENJU>(t, FE_WHITE);
        let classical = self.classical;
        let mut delta_value_black = 0_i32;
        let mut restore_idx = 0_usize;

        let mut i = -l;
        while i <= l {
            for dir in 0..4 {
                if ((bit_key[dir] >> (2 * l)) & 0b11) != 0b11 {
                    continue;
                }
                let posi = (pos as isize + DIR_STEP[dir] * i) as usize;
                debug_assert!(posi < BOARD_AREA && self.empty_bb.test(posi));
                debug_assert_eq!(
                    bit_key[dir] & ((1_u64 << (4 * l + 2)) - 1),
                    self.get_key_at::<RENJU>(posi, dir) & ((1_u64 << (4 * l + 2)) - 1)
                );

                // SAFETY: `posi < BOARD_AREA` (empty cell check in padded bit_key guarantees
                // on-board cell) and `dir < 4`, `idx_b/idx_w < 65536`, `p4 < 14`.
                unsafe {
                    let old_p2x = *self.pattern2xs.get_unchecked(posi).get_unchecked(dir);
                    let new_p2x = lookup_pattern::<RENJU>(t, bit_key[dir]);
                    let p4_pair = *self.pattern4x2.get_unchecked(posi);

                    if new_p2x == old_p2x {
                        let b4 = p4_pair & 0x0F;
                        let w4 = p4_pair >> 4;
                        if b4 >= Pattern4::CBlock4Flex3 as u8 {
                            *st.last_p4_move[FE_BLACK]
                                .get_unchecked_mut((b4 - Pattern4::CBlock4Flex3 as u8) as usize) =
                                posi as u16;
                        }
                        if w4 >= Pattern4::CBlock4Flex3 as u8 {
                            *st.last_p4_move[FE_WHITE]
                                .get_unchecked_mut((w4 - Pattern4::CBlock4Flex3 as u8) as usize) =
                                posi as u16;
                        }
                        continue;
                    }

                    let packed = (posi as u32)
                        | ((dir as u32) << 8)
                        | ((old_p2x.0 as u32) << 16)
                        | ((p4_pair as u32) << 24);
                    *self
                        .move_restores
                        .get_unchecked_mut(restore_slot)
                        .entries
                        .get_unchecked_mut(restore_idx) = packed;
                    restore_idx += 1;

                    let row_mut = self.pattern2xs.get_unchecked_mut(posi);
                    *row_mut.get_unchecked_mut(dir) = new_p2x;
                    let row = *row_mut;
                    let idx_b = (row[0].black_raw() << 12)
                        | (row[1].black_raw() << 8)
                        | (row[2].black_raw() << 4)
                        | row[3].black_raw();
                    let idx_w = (row[0].white_raw() << 12)
                        | (row[1].white_raw() << 8)
                        | (row[2].white_raw() << 4)
                        | row[3].white_raw();
                    let cp_b = *pc_b.get_unchecked(idx_b);
                    let cp_w = *pc_w.get_unchecked(idx_w);
                    if let Some(cm) = classical {
                        let shift = (3 - dir) * 4;
                        let old_idx_b = (idx_b & !(0xF << shift)) | (old_p2x.black_raw() << shift);
                        let old_idx_w = (idx_w & !(0xF << shift)) | (old_p2x.white_raw() << shift);
                        delta_value_black += cm
                            .get_value_black::<RENJU>(cp_b.pcode(), cp_w.pcode())
                            - cm.get_value_black::<RENJU>(
                                pc_b.get_unchecked(old_idx_b).pcode(),
                                pc_w.get_unchecked(old_idx_w).pcode(),
                            );
                    }
                    let p4_b = cp_b.pattern4_raw();
                    let p4_w = cp_w.pattern4_raw();

                    *st.p4_count[FE_BLACK].get_unchecked_mut((p4_pair & 0x0F) as usize) -= 1;
                    *st.p4_count[FE_WHITE].get_unchecked_mut((p4_pair >> 4) as usize) -= 1;
                    *st.p4_count[FE_BLACK].get_unchecked_mut(p4_b as usize) += 1;
                    *st.p4_count[FE_WHITE].get_unchecked_mut(p4_w as usize) += 1;
                    *self.pattern4x2.get_unchecked_mut(posi) = pack_p4_pair(p4_b, p4_w);

                    if p4_b >= Pattern4::CBlock4Flex3 as u8 {
                        *st.last_p4_move[FE_BLACK]
                            .get_unchecked_mut((p4_b - Pattern4::CBlock4Flex3 as u8) as usize) =
                            posi as u16;
                    }
                    if p4_w >= Pattern4::CBlock4Flex3 as u8 {
                        *st.last_p4_move[FE_WHITE]
                            .get_unchecked_mut((p4_w - Pattern4::CBlock4Flex3 as u8) as usize) =
                            posi as u16;
                    }
                }
            }

            let shamt = if i == -1 { 4 } else { 2 };
            bit_key[0] >>= shamt;
            bit_key[1] >>= shamt;
            bit_key[2] >>= shamt;
            bit_key[3] >>= shamt;
            i += if i == -1 { 2 } else { 1 };
        }
        self.move_restores[restore_slot].len = restore_idx as u8;

        // Drop the placed cell's own frozen p4 contribution (cell is no longer empty).
        if let Some(cm) = classical {
            let (pb, pw) = self.pcode_pair(pos);
            st.value_black += delta_value_black - cm.get_value_black::<RENJU>(pb, pw);
        }
        let self_pair = self.pattern4x2[pos];
        st.p4_count[FE_BLACK][(self_pair & 0x0F) as usize] -= 1;
        st.p4_count[FE_WHITE][(self_pair >> 4) as usize] -= 1;

        if f4_before_b == 0 && st.p4_count[FE_BLACK][Pattern4::BFlex4 as usize] > 0 {
            st.last_flex4_attack[FE_BLACK] = pos as u16;
        }
        if f4_before_w == 0 && st.p4_count[FE_WHITE][Pattern4::BFlex4 as usize] > 0 {
            st.last_flex4_attack[FE_WHITE] = pos as u16;
        }

        self.state_infos[self.move_count] = st;

        if FLIP_SIDE {
            self.current_side ^= 1;
        }

        // Mark candidate neighborhood and journal 0 -> 1 transitions for O(1) undo.
        let stencil = match self.cand_range {
            CandRange::Square2Line3 => &CAND_STENCIL_S2L3[pos],
            CandRange::Square3Line4 => &CAND_STENCIL_S3L4[pos],
        };
        for w in 0..4 {
            let delta = stencil.words[w] & !self.candidates_bb.words[w];
            if delta != 0 {
                self.candidates_bb.words[w] |= delta;
                self.cand_journal[self.journal_top] = CandJournalEntry {
                    word_idx: w as u8,
                    delta,
                };
                self.journal_top += 1;
            }
        }
    }

    #[inline]
    pub fn undo_typed<const RENJU: bool, const FLIP_SIDE: bool>(&mut self) {
        debug_assert!(self.move_count > 0);
        let dying = self.state_infos[self.move_count];
        let last_pos = dying.last_move;

        // Rewind candidate-bit 0->1 transitions recorded on this ply.
        let j_start = dying.journal_start as usize;
        while self.journal_top > j_start {
            self.journal_top -= 1;
            let entry = self.cand_journal[self.journal_top];
            self.candidates_bb.words[entry.word_idx as usize] ^= entry.delta;
        }

        if last_pos == PASS_MOVE {
            self.current_side ^= 1;
            self.zobrist_key ^= ZOBRIST.side_to_move;
            self.pass_count[self.current_side] -= 1;
            self.move_count -= 1;
            return;
        }

        let pos = last_pos as usize;
        if FLIP_SIDE {
            self.current_side ^= 1;
            self.zobrist_key ^= ZOBRIST.side_to_move;
        }
        let side = self.current_side;
        self.flip_bit_key(pos, side);
        self.zobrist_key ^= ZOBRIST.piece[side][pos];
        self.empty_bb.set(pos);

        self.move_count -= 1;
        // SAFETY: every entry recorded in `play_typed` has `posi < BOARD_AREA` and `dir < 4`.
        unsafe {
            let restore = self.move_restores.get_unchecked(self.move_count);
            let len = restore.len as usize;
            for idx in 0..len {
                let packed = *restore.entries.get_unchecked(idx);
                let posi = (packed & 0xFF) as usize;
                let dir = ((packed >> 8) & 0x03) as usize;
                let old_p2x = ((packed >> 16) & 0xFF) as u8;
                let old_p4 = (packed >> 24) as u8;
                *self
                    .pattern2xs
                    .get_unchecked_mut(posi)
                    .get_unchecked_mut(dir) = Pattern2x(old_p2x);
                *self.pattern4x2.get_unchecked_mut(posi) = old_p4;
            }
        }
    }

    /// Exact Renju forbidden point detector aligned with Rapfi `Board::checkForbiddenPoint`.
    /// Must only be called on an empty cell in Renju rule.
    pub fn check_forbidden_point(&mut self, pos: usize) -> bool {
        debug_assert_eq!(self.rule, RuleSet::Renju);
        debug_assert!(self.empty_bb.test(pos));

        if self.pattern4(pos, FE_BLACK) != Pattern4::Forbid {
            return false;
        }
        #[cfg(test)]
        TEST_FORBID_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut win_by_four = 0_u8;
        for dir in 0..4 {
            let p = self.pattern(pos, FE_BLACK, dir);
            if p == Pattern::Ol {
                return true;
            } else if matches!(p, Pattern::B4 | Pattern::B4S | Pattern::F4) {
                win_by_four += 1;
                if win_by_four >= 2 {
                    return true;
                }
            }
        }

        // Verify double-three by temporarily placing Black at `pos` (relying on
        // the freeze invariant: `self.pattern(pos, FE_BLACK, dir)` stays frozen).
        let saved_side = self.current_side;
        self.current_side = FE_BLACK;
        self.play_typed::<true, false>(pos);

        let mut win_by_three = 0_u8;
        const DIRS: [(isize, isize); 4] = [(1, 0), (0, 1), (1, -1), (1, 1)];
        let px = (pos % BOARD_SIZE) as isize;
        let py = (pos / BOARD_SIZE) as isize;

        for dir in 0..4 {
            let p = self.pattern(pos, FE_BLACK, dir);
            if p != Pattern::F3 && p != Pattern::F3S {
                continue;
            }
            let (dx, dy) = DIRS[dir];
            let mut found_open_four = false;
            for sign in [-1_isize, 1_isize] {
                for step in 1..=4_isize {
                    let nx = px + dx * sign * step;
                    let ny = py + dy * sign * step;
                    if nx < 0 || nx >= BOARD_SIZE as isize || ny < 0 || ny >= BOARD_SIZE as isize {
                        break;
                    }
                    let posi = ny as usize * BOARD_SIZE + nx as usize;
                    let bits = self.cell_bits(posi);
                    if bits == 0b11 {
                        // Empty cell: check whether it makes a legal open-four along `dir`.
                        // Note: `p4` may be `BFlex4`, `AFive` (five in another direction
                        // overrides forbidden), or false-`Forbid` — all are legal iff
                        // `p4 != Forbid || !check_forbidden_point(posi)`.
                        let p4 = self.pattern4(posi, FE_BLACK);
                        let line_p = self.pattern(posi, FE_BLACK, dir);
                        if line_p == Pattern::F4
                            && (p4 != Pattern4::Forbid || !self.check_forbidden_point(posi))
                        {
                            found_open_four = true;
                        }
                        break;
                    } else if bits != 0b10 {
                        // Opponent stone blocks this direction.
                        break;
                    }
                }
                if found_open_four {
                    win_by_three += 1;
                    break;
                }
            }
            if win_by_three >= 2 {
                break;
            }
        }

        self.undo_typed::<true, false>();
        self.current_side = saved_side;
        win_by_three >= 2
    }

    /// Exact Rapfi `quickWinCheck` (`game/wincheck.h`) using Rapfi's `VALUE_MATE = 30_000`
    /// mate-distance convention (`mate_in(ply + k)` / `mated_in(ply + k)`).
    /// Returns `0` if no static forced win/loss is proved.
    pub fn quick_win_check<const RENJU: bool>(&mut self, ply: i32, beta: i32) -> i32 {
        use crate::fast_engine::tt::{mate_in, mated_in};
        let self_side = self.current_side;
        let opp_side = self_side ^ 1;

        if self.p4_count(self_side, Pattern4::AFive) > 0 {
            return mate_in(ply + 1);
        }
        match self.p4_count(opp_side, Pattern4::AFive) {
            0 => {}
            1 => return 0,
            _ => return mated_in(ply + 2),
        }
        if self.p4_count(self_side, Pattern4::BFlex4) > 0 {
            return mate_in(ply + 3);
        }

        let block4_count = |b: &Self, s: usize| -> u16 {
            b.p4_count(s, Pattern4::CBlock4Flex3)
                + b.p4_count(s, Pattern4::DBlock4Plus)
                + b.p4_count(s, Pattern4::EBlock4)
        };

        if !RENJU || mate_in(ply + 5) >= beta {
            let mut c_count = self.p4_count(self_side, Pattern4::CBlock4Flex3);
            if c_count > 0 {
                if (!RENJU || self_side == FE_WHITE) && block4_count(self, opp_side) == 0 {
                    return mate_in(ply + 5);
                }
                let cands = self.empty_candidates();
                let mut found_win = false;
                'scan: for w_idx in 0..4 {
                    let mut w = cands.words[w_idx];
                    let base = w_idx << 6;
                    while w != 0 {
                        let tz = w.trailing_zeros() as usize;
                        w &= w - 1;
                        let pos = base + tz;
                        if self.pattern4(pos, self_side) != Pattern4::CBlock4Flex3 {
                            continue;
                        }
                        self.play_typed::<RENJU, true>(pos);
                        let defend_mv = self.last_p4_move(self_side, Pattern4::AFive) as usize;
                        let defend_p4 = self.pattern4(defend_mv, opp_side);
                        let has_defend = defend_p4 >= Pattern4::EBlock4
                            || (RENJU
                                && defend_p4 == Pattern4::Forbid
                                && opp_side == FE_BLACK
                                && !self.check_forbidden_point(defend_mv));
                        let is_fake_c = RENJU && self.p4_count(self_side, Pattern4::BFlex4) == 0;
                        self.undo_typed::<RENJU, true>();

                        if !has_defend && !is_fake_c {
                            found_win = true;
                            break 'scan;
                        }
                        c_count -= 1;
                        if c_count == 0 {
                            break 'scan;
                        }
                    }
                }
                if found_win {
                    return mate_in(ply + 5);
                }
            }

            if self.p4_count(self_side, Pattern4::FFlex3x2) > 0
                && self.p4_count(opp_side, Pattern4::BFlex4) + block4_count(self, opp_side) == 0
            {
                return mate_in(ply + 5);
            }
        }

        0
    }

    /// Synchronize a `FastBoard` from a legacy `crate::board::Board` move history.
    pub fn from_legacy_board(board: &crate::board::Board, rule: RuleSet, range: CandRange) -> Self {
        let mut fb = Self::new(rule, range);
        for rec in board.move_history() {
            fb.play(rec.move_ as usize);
        }
        fb
    }

    #[cfg(test)]
    fn assert_invariants(&self) {
        let mut expected_p4 = [[0_u16; PATTERN4_NB]; 2];
        let renju = self.rule == RuleSet::Renju;
        let t = self.tables;
        let pc_b = if renju {
            &t.pcode_renju_black
        } else {
            &t.pcode
        };
        let pc_w = &t.pcode;
        let mut expected_value_black = 0_i32;

        for pos in 0..BOARD_AREA {
            let empty = self.empty_bb.test(pos);
            assert_eq!(
                empty,
                self.cell_bits(pos) == 0b11,
                "empty_bb mismatch at {pos}"
            );
            if !empty {
                continue;
            }
            for dir in 0..4 {
                let key = if renju {
                    self.get_key_at::<true>(pos, dir)
                } else {
                    self.get_key_at::<false>(pos, dir)
                };
                let exp = if renju {
                    lookup_pattern::<true>(t, key)
                } else {
                    lookup_pattern::<false>(t, key)
                };
                assert_eq!(
                    self.pattern2xs[pos][dir], exp,
                    "pattern2x mismatch at pos={pos} dir={dir}"
                );
            }
            let row = self.pattern2xs[pos];
            let idx_b = (row[0].black_raw() << 12)
                | (row[1].black_raw() << 8)
                | (row[2].black_raw() << 4)
                | row[3].black_raw();
            let idx_w = (row[0].white_raw() << 12)
                | (row[1].white_raw() << 8)
                | (row[2].white_raw() << 4)
                | row[3].white_raw();
            let cp_b = pc_b[idx_b];
            let cp_w = pc_w[idx_w];
            let exp_b = cp_b.pattern4_raw();
            let exp_w = cp_w.pattern4_raw();
            assert_eq!(
                self.pattern4x2[pos],
                pack_p4_pair(exp_b, exp_w),
                "pattern4x2 mismatch at pos={pos}"
            );
            expected_p4[FE_BLACK][exp_b as usize] += 1;
            expected_p4[FE_WHITE][exp_w as usize] += 1;
            if let Some(cm) = self.classical {
                expected_value_black += if renju {
                    cm.get_value_black::<true>(cp_b.pcode(), cp_w.pcode())
                } else {
                    cm.get_value_black::<false>(cp_b.pcode(), cp_w.pcode())
                };
            }
        }
        assert_eq!(
            self.state_infos[self.move_count].p4_count, expected_p4,
            "p4_count mismatch at move_count={}",
            self.move_count
        );
        if self.classical.is_some() {
            assert_eq!(
                self.state_infos[self.move_count].value_black, expected_value_black,
                "value_black mismatch at move_count={}",
                self.move_count
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{xy_to_move, Board};
    use crate::constants::BLACK;
    use crate::rules::classify_forbidden_move;

    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[test]
    fn fastboard_random_play_undo_and_pass_invariants_both_rules() {
        let mut rng = 0x1234_5678_9ABC_DEF0_u64;
        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            let mut fb = FastBoard::new(rule, CandRange::Square3Line4);
            fb.assert_invariants();
            let initial_key = fb.zobrist_key();
            let initial_cands = fb.candidates_bb;

            for _trial in 0..40 {
                let len = 10 + (splitmix64(&mut rng) as usize % 35);
                let mut played = 0_usize;
                for step in 0..len {
                    if step > 0 && step % 11 == 0 {
                        let expected_key = fb.zobrist_key_after_pass();
                        fb.play_pass();
                        assert_eq!(fb.zobrist_key(), expected_key);
                        played += 1;
                        continue;
                    }
                    let empties: Vec<usize> = (0..BOARD_AREA).filter(|&p| fb.is_empty(p)).collect();
                    let pos = empties[splitmix64(&mut rng) as usize % empties.len()];
                    let expected_key = fb.zobrist_key_after(pos);
                    fb.play(pos);
                    assert_eq!(fb.zobrist_key(), expected_key);
                    played += 1;
                    if step % 5 == 0 {
                        fb.assert_invariants();
                    }
                }
                fb.assert_invariants();
                for _ in 0..played {
                    fb.undo();
                }
                assert_eq!(fb.move_count(), 0);
                assert_eq!(fb.zobrist_key(), initial_key);
                assert_eq!(fb.candidates_bb, initial_cands);
                fb.assert_invariants();
            }
        }
    }

    #[test]
    fn fastboard_renju_forbidden_matches_oracle_across_random_and_tricky_positions() {
        // 1. Hand-crafted tricky Renju positions (overline, 4-4, 3-3, false 3-3, five overrides forbidden).
        let tricky_boards: &[&[(usize, usize)]] = &[
            // Double-three at (7,7)
            &[
                (6, 7),
                (0, 0),
                (5, 7),
                (0, 2),
                (7, 6),
                (0, 4),
                (7, 5),
                (0, 6),
            ],
            // Overline at (7,7): B B B . B B
            &[
                (4, 7),
                (0, 0),
                (5, 7),
                (0, 2),
                (6, 7),
                (0, 4),
                (8, 7),
                (0, 6),
                (9, 7),
                (0, 8),
            ],
            // Exact-five overrides 4-4 / 3-3 at (7,7)
            &[
                (3, 7),
                (0, 0),
                (4, 7),
                (0, 2),
                (5, 7),
                (0, 4),
                (6, 7),
                (0, 6),
                (7, 5),
                (0, 8),
                (7, 6),
                (0, 10),
            ],
            // Single-line double-four: B _ B . B _ B at (7,7)
            &[
                (4, 7),
                (0, 0),
                (6, 7),
                (0, 2),
                (8, 7),
                (0, 4),
                (10, 7),
                (0, 6),
            ],
            // False double-three where one open-three's completion point is itself a 4-4 forbidden point!
            &[
                (7, 7),
                (0, 0),
                (7, 8),
                (0, 2),
                (8, 6),
                (0, 4),
                (9, 6),
                (0, 6),
                (5, 6),
                (0, 8),
                (4, 6),
                (0, 10),
                (6, 4),
                (0, 12),
                (6, 3),
                (0, 14),
            ],
        ];

        for (bi, seq) in tricky_boards.iter().enumerate() {
            let mut legacy = Board::new();
            let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
            for &(x, y) in *seq {
                let m = xy_to_move(x, y).unwrap();
                legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
                fb.play(m as usize);
            }
            for pos in 0..BOARD_AREA {
                if !fb.is_empty(pos) {
                    continue;
                }
                let oracle = classify_forbidden_move(&legacy, pos as u16, BLACK, RuleSet::Renju)
                    .unwrap()
                    .is_forbidden();
                let fast = fb.check_forbidden_point(pos);
                assert_eq!(
                    fast,
                    oracle,
                    "tricky board {bi} pos=({},{}): fast={fast} oracle={oracle} p4={:?}",
                    pos % BOARD_SIZE,
                    pos / BOARD_SIZE,
                    fb.pattern4(pos, FE_BLACK)
                );
            }
            fb.assert_invariants();
        }

        // 2. Fuzz 1000 dense center Renju boards (checking ~180,000 empty cells!).
        let mut rng = 0xCAFE_BABE_DEAD_BEEF_u64;
        let mut total_forbid = 0_usize;
        let mut total_false_forbid = 0_usize;
        for trial in 0..1000 {
            let mut legacy = Board::new();
            let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
            let plies = 12 + (splitmix64(&mut rng) as usize % 36);
            for _ in 0..plies {
                if legacy.is_terminal() {
                    break;
                }
                // Concentrate stones in 9x9 center to maximize 3-3 / 4-4 / overline / false-3-3 density.
                let mut picked = None;
                for _ in 0..40 {
                    let x = 3 + (splitmix64(&mut rng) as usize % 9);
                    let y = 3 + (splitmix64(&mut rng) as usize % 9);
                    let m = xy_to_move(x, y).unwrap();
                    if legacy.is_legal_move_for_rule(m, legacy.side_to_move(), RuleSet::Renju) {
                        picked = Some(m);
                        break;
                    }
                }
                let Some(m) = picked else { break };
                legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
                fb.play(m as usize);
            }
            if legacy.is_terminal() {
                continue;
            }
            for pos in 0..BOARD_AREA {
                if !fb.is_empty(pos) {
                    continue;
                }
                let oracle_kind =
                    classify_forbidden_move(&legacy, pos as u16, BLACK, RuleSet::Renju).unwrap();
                let oracle = oracle_kind.is_forbidden();
                let fast = fb.check_forbidden_point(pos);
                if fb.pattern4(pos, FE_BLACK) == Pattern4::Forbid && !fast {
                    total_false_forbid += 1;
                }
                if fast {
                    total_forbid += 1;
                }
                if fast != oracle {
                    let pats = [
                        fb.pattern(pos, FE_BLACK, 0),
                        fb.pattern(pos, FE_BLACK, 1),
                        fb.pattern(pos, FE_BLACK, 2),
                        fb.pattern(pos, FE_BLACK, 3),
                    ];
                    let mut grid_str = String::new();
                    for y in 0..BOARD_SIZE {
                        for x in 0..BOARD_SIZE {
                            let c = match legacy.grid_rows()[y][x] {
                                1 => 'X',
                                -1 => 'O',
                                _ => '.',
                            };
                            grid_str.push(c);
                        }
                        grid_str.push('\n');
                    }
                    panic!(
                        "fuzz trial {trial} pos=({},{}): fast={fast} oracle={oracle} ({oracle_kind:?}) p4={:?} pats={pats:?}\n{grid_str}",
                        pos % BOARD_SIZE,
                        pos / BOARD_SIZE,
                        fb.pattern4(pos, FE_BLACK)
                    );
                }
            }
            fb.assert_invariants();
        }
        assert!(
            total_forbid > 50,
            "fuzz should hit many forbidden points, got {total_forbid}"
        );
        let _ = total_false_forbid;
    }

    #[test]
    fn fastboard_renju_forbidden_matches_all_hand_fixtures() {
        #[derive(serde::Deserialize)]
        struct Stone {
            x: usize,
            y: usize,
            side: crate::types::Side,
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
            expected: String,
        }

        let text = include_str!("../../cases/renju/forbidden_hand_cases.jsonl");
        let mut count = 0_usize;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fixture: Fixture = serde_json::from_str(line).unwrap();
            let mut legacy = Board::new();
            let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
            for s in &fixture.moves {
                legacy.grid_rows_mut()[s.y][s.x] = s.side;
                let side_idx = if s.side == BLACK { FE_BLACK } else { FE_WHITE };
                fb.current_side = side_idx;
                fb.play_typed::<true, false>(s.y * BOARD_SIZE + s.x);
            }
            fb.current_side = FE_BLACK;
            let cand_pos = fixture.candidate.y * BOARD_SIZE + fixture.candidate.x;
            let expected_forbid = fixture.expected != "none";
            assert_eq!(
                fb.check_forbidden_point(cand_pos),
                expected_forbid,
                "fixture {} expected={}",
                fixture.name,
                fixture.expected
            );
            // Also verify EVERY empty cell on the fixture board matches oracle!
            for pos in 0..BOARD_AREA {
                if !fb.is_empty(pos) {
                    continue;
                }
                let oracle = classify_forbidden_move(&legacy, pos as u16, BLACK, RuleSet::Renju)
                    .unwrap()
                    .is_forbidden();
                assert_eq!(
                    fb.check_forbidden_point(pos),
                    oracle,
                    "fixture {} extra cell ({},{})",
                    fixture.name,
                    pos % BOARD_SIZE,
                    pos / BOARD_SIZE
                );
            }
            fb.assert_invariants();
            count += 1;
        }
        assert!(count >= 10, "expected hand fixtures, got {count}");
    }

    #[test]
    fn fastboard_quick_win_check_and_defence_table() {
        use crate::fast_engine::pattern::lookup_defence;
        use crate::fast_engine::tt::{mate_in, mated_in, VALUE_INFINITE};

        // 1. AFive -> mate_in(ply + 1)
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[
            (4, 7),
            (0, 0),
            (5, 7),
            (0, 2),
            (6, 7),
            (0, 4),
            (7, 7),
            (0, 6),
        ] {
            fb.play(y * BOARD_SIZE + x);
        }
        assert_eq!(fb.quick_win_check::<true>(2, VALUE_INFINITE), mate_in(3));

        // 2. BFlex4 (open three -> open four on next move) -> mate_in(ply + 3)
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(5, 7), (0, 0), (6, 7), (0, 2), (7, 7), (0, 4)] {
            fb.play(y * BOARD_SIZE + x);
        }
        assert_eq!(fb.quick_win_check::<true>(0, VALUE_INFINITE), mate_in(3));

        // 3. Opponent double AFive -> mated_in(ply + 2)
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(5, 7), (0, 0), (6, 7), (0, 2), (7, 7), (0, 4), (8, 7)] {
            fb.play(y * BOARD_SIZE + x);
        }
        // White to move against Black's open four at (4,7) and (9,7).
        assert_eq!(fb.quick_win_check::<true>(1, VALUE_INFINITE), mated_in(3));

        // 4. Defence lookup for F3 cell: two stones at (5,7) and (6,7), so (7,7) has F3.
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(5, 7), (0, 0), (6, 7), (0, 2)] {
            fb.play(y * BOARD_SIZE + x);
        }
        let f3_pos = 7 * BOARD_SIZE + 7;
        assert_eq!(fb.pattern(f3_pos, FE_BLACK, 0), Pattern::F3S);
        let key = fb.get_key_at::<true>(f3_pos, 0);
        let mask = lookup_defence::<true>(fb.tables, key, FE_BLACK);
        // Offsets -3 (x=4) and +1 (x=8) defend the F3 made by playing at (7,7).
        assert_ne!(mask, 0, "F3 defence mask should be non-zero");
        assert_ne!(
            mask & (1 << (3 - 2)),
            0,
            "offset -3 (x=4) should be in defence mask"
        );
        assert_ne!(
            mask & (1 << (4 + 0)),
            0,
            "offset +1 (x=8) should be in defence mask"
        );
    }

    #[test]
    #[ignore]
    fn bench_fastboard_vs_legacy_play_undo() {
        use crate::eval::{recompute_all_for_rule, value_wide_compute_for_rule_no_dgn, EvalCaches};
        use std::hint::black_box;
        use std::time::Instant;

        let mut legacy = Board::new();
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        let setup: &[(usize, usize)] = &[
            (7, 7),
            (7, 6),
            (8, 6),
            (6, 8),
            (6, 6),
            (8, 8),
            (5, 7),
            (9, 7),
            (7, 8),
            (6, 7),
            (8, 7),
            (7, 5),
        ];
        for &(x, y) in setup {
            let m = xy_to_move(x, y).unwrap();
            legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
            fb.play(m as usize);
        }
        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut legacy, &mut caches, RuleSet::Renju);

        let probes: [usize; 8] = [
            6 * BOARD_SIZE + 5,
            8 * BOARD_SIZE + 5,
            5 * BOARD_SIZE + 8,
            9 * BOARD_SIZE + 8,
            5 * BOARD_SIZE + 6,
            9 * BOARD_SIZE + 6,
            6 * BOARD_SIZE + 9,
            8 * BOARD_SIZE + 9,
        ];
        let iters = 200_000_usize;

        let t0 = Instant::now();
        let mut acc_fast = 0_u64;
        for i in 0..iters {
            let p = probes[i & 7];
            fb.play_typed::<true, true>(p);
            acc_fast ^= fb.zobrist_key();
            fb.undo_typed::<true, true>();
        }
        let dt_fast = t0.elapsed();

        let t1 = Instant::now();
        let mut acc_legacy = 0_u64;
        for i in 0..iters {
            let p = probes[i & 7];
            let m = p as u16;
            let rc = (p / BOARD_SIZE, p % BOARD_SIZE);
            let snap = caches.snapshot_nnue();
            legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
            value_wide_compute_for_rule_no_dgn(&mut legacy, &mut caches, rc, RuleSet::Renju);
            acc_legacy ^= legacy.zobrist_key();
            legacy.undo().unwrap();
            caches.restore_snapshot_nnue(&snap);
        }
        let dt_legacy = t1.elapsed();

        black_box((acc_fast, acc_legacy));
        let ns_fast = dt_fast.as_nanos() as f64 / iters as f64;
        let ns_legacy = dt_legacy.as_nanos() as f64 / iters as f64;
        println!(
            "play+undo bench ({iters} iters, Renju): FastBoard={ns_fast:.1} ns/op, Legacy={ns_legacy:.1} ns/op, speedup={:.2}x",
            ns_legacy / ns_fast
        );
    }
}
