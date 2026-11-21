//! Incremental 4-direction pattern bitboard (`Pattern4`) for fast threat queries.
//!
//! Each of the 72 lines on the 15x15 board is stored as a 2-bit-per-cell `u64`
//! with 5 wall sentinel cells on each end (`5 + len + 5 <= 25` cells = 50 bits).
//! For any empty cell, the 10-cell neighborhood (`-5..=-1`, `+1..=+5`) along a
//! line maps in O(1) through a 59,049-entry ternary LUT to the exact 1D threat
//! pattern for both Freestyle (and Renju White) and Renju Black, plus the exact
//! relative completion offset for broken fours (`B4`).
//!
//! Bitsets (`BitBoard225`) use column-major bit index `x * 15 + y` so iterating
//! with `trailing_zeros()` visits points in the exact `(x, y)` order used by
//! `ThreatBoardView::threat_moves` and `first_counter_win`.

use std::sync::OnceLock;

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, EMPTY};
use crate::rules::RuleSet;
use crate::types::{Move, Side};

const PAD: usize = 5;
const NUM_LINES_PER_DIAG: usize = 2 * BOARD_SIZE - 1; // 29
const TOTAL_LINES: usize = BOARD_SIZE + BOARD_SIZE + NUM_LINES_PER_DIAG + NUM_LINES_PER_DIAG; // 88
const TRIT_BASE: usize = 243; // 3^5
const LUT_SIZE: usize = TRIT_BASE * TRIT_BASE; // 3^10 = 59,049

// 2-bit cell encoding inside line bitboards:
// 0 = EMPTY, 1 = BLACK, 2 = WHITE, 3 = WALL.
const CELL_EMPTY: u64 = 0;
const CELL_BLACK: u64 = 1;
const CELL_WHITE: u64 = 2;

/// 1D pattern created by placing `side` at an empty point along one axis.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum P1 {
    None = 0,
    /// Placing here creates a candidate live three (can become a real A4 on a
    /// valid gain square within this line).
    A3 = 1,
    /// Placing here creates a single broken four (exactly 1 valid five-point on
    /// this line; its relative offset `-4..=+4` is stored in `b4_reply_offset`).
    B4 = 2,
    /// Placing here creates a straight open four (`_XXXX_`, or under Freestyle
    /// a single-line double jump-four) with >= 2 valid five-points on this line.
    A4 = 3,
    /// Renju Black only: placing here creates two distinct non-overline fours on
    /// this single line (a 4-4 forbidden move unless another axis makes WIN5).
    DoubleFour1D = 4,
    /// Placing here completes a winning five (exact five for Renju Black, >=5
    /// otherwise).
    Win5 = 5,
    /// Renju Black only: placing here creates a 6+ overline (forbidden unless
    /// another axis makes an exact WIN5).
    Overline = 6,
    /// Renju Black only: not a standard straight `A3` for VCT/VCF, but `rules::forbidden`
    /// recognizes a candidate open-three gain on this axis (must contribute to `maybe_33`).
    RenjuOpen3Candidate = 7,
}

impl P1 {
    #[inline]
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::A3,
            2 => Self::B4,
            3 => Self::A4,
            4 => Self::DoubleFour1D,
            5 => Self::Win5,
            6 => Self::Overline,
            7 => Self::RenjuOpen3Candidate,
            _ => Self::None,
        }
    }
}

/// Combined 4-direction summary for one empty cell and one side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellPattern {
    /// Bit flags:
    /// - bit 0..=2: primary class (`0=None, 1=A3, 2=B4, 3=A4, 4=Win5, 5=Forbidden`)
    /// - bit 3: `maybe_33` (Renju Black has >= 2 axes with `A3`, requires
    ///   recursive forbidden check when verifying legality)
    /// - bit 4..=5: direction index `0..4` holding the `B4` when primary class is `B4`
    packed: u8,
}

const CLASS_MASK: u8 = 0x07;
const CLASS_NONE: u8 = 0;
const CLASS_A3: u8 = 1;
const CLASS_B4: u8 = 2;
const CLASS_A4: u8 = 3;
const CLASS_WIN5: u8 = 4;
const CLASS_FORBIDDEN: u8 = 5;
const FLAG_MAYBE_33: u8 = 1 << 3;
const B4_DIR_SHIFT: u8 = 4;
const FLAG_STRAIGHT_A4: u8 = 1 << 6;

impl CellPattern {
    #[inline]
    pub(crate) fn is_win5(self) -> bool {
        (self.packed & CLASS_MASK) == CLASS_WIN5
    }

    #[inline]
    pub(crate) fn is_forbidden_definite(self) -> bool {
        (self.packed & CLASS_MASK) == CLASS_FORBIDDEN
    }

    #[inline]
    pub(crate) fn maybe_33(self) -> bool {
        (self.packed & FLAG_MAYBE_33) != 0
    }

    #[inline]
    pub(crate) fn has_straight_a4(self) -> bool {
        (self.packed & FLAG_STRAIGHT_A4) != 0
    }

    #[inline]
    pub(crate) fn b4_dir(self) -> usize {
        ((self.packed >> B4_DIR_SHIFT) & 0x03) as usize
    }
}

/// 225-bit set indexed by column-major `idx = x * BOARD_SIZE + y`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BitBoard225 {
    words: [u64; 4],
}

impl BitBoard225 {
    #[inline]
    pub(crate) fn set(&mut self, idx: usize) {
        self.words[idx >> 6] |= 1_u64 << (idx & 63);
    }

    #[inline]
    pub(crate) fn clear(&mut self, idx: usize) {
        self.words[idx >> 6] &= !(1_u64 << (idx & 63));
    }

    #[inline]
    pub(crate) fn contains(&self, idx: usize) -> bool {
        (self.words[idx >> 6] & (1_u64 << (idx & 63))) != 0
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        (self.words[0] | self.words[1] | self.words[2] | self.words[3]) == 0
    }

    #[inline]
    pub(crate) fn count(&self) -> u32 {
        self.words[0].count_ones()
            + self.words[1].count_ones()
            + self.words[2].count_ones()
            + self.words[3].count_ones()
    }

    #[inline]
    pub(crate) fn first_idx(&self) -> Option<usize> {
        for (wi, &word) in self.words.iter().enumerate() {
            if word != 0 {
                return Some((wi << 6) + word.trailing_zeros() as usize);
            }
        }
        None
    }

    #[inline]
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            words: [
                self.words[0] | other.words[0],
                self.words[1] | other.words[1],
                self.words[2] | other.words[2],
                self.words[3] | other.words[3],
            ],
        }
    }

    #[inline]
    pub(crate) fn iter_indices(self) -> BitBoard225Iter {
        BitBoard225Iter {
            words: self.words,
            word_idx: 0,
        }
    }
}

pub(crate) struct BitBoard225Iter {
    words: [u64; 4],
    word_idx: usize,
}

impl Iterator for BitBoard225Iter {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        while self.word_idx < 4 {
            let word = self.words[self.word_idx];
            if word != 0 {
                let tz = word.trailing_zeros() as usize;
                self.words[self.word_idx] = word & (word - 1);
                return Some((self.word_idx << 6) + tz);
            }
            self.word_idx += 1;
        }
        None
    }
}

/// Compact 1D defense info for an `A3` pattern along one axis.
/// Up to 2 gain offsets (relative to center `5`, in `-3..=+3`; `0` = unused)
/// and for each gain, its 2 open-four completion offsets (in `-4..=+4`).
/// `gains[0] == i8::MIN` marks rare multi-window lines whose exact `scan_a3`
/// gain selection depends on scan direction (caller falls back to slow path).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct A3LineDefenses {
    pub(crate) gains: [i8; 2],
    pub(crate) completions: [[i8; 2]; 2],
}

impl A3LineDefenses {
    pub(crate) const AMBIGUOUS: Self = Self {
        gains: [i8::MIN, 0],
        completions: [[0; 2]; 2],
    };

    #[inline]
    pub(crate) fn is_ambiguous(self) -> bool {
        self.gains[0] == i8::MIN
    }
}

/// Precomputed 1D pattern lookup tables for ±5 windows.
struct PatternLut {
    /// Maps 5 packed 2-bit cells (`0..1024`) to a 5-trit integer (`0..243`) for
    /// slot 0 (Black) and slot 1 (White).
    half_to_trit: [[u16; 1024]; 2],
    /// Low nibble = Freestyle / Renju White `P1`; high nibble = Renju Black `P1`.
    p1_packed: Vec<u8>,
    /// Exact relative offset `-4..=+4` of the unique five-point when `P1 == B4`,
    /// or `0` otherwise.
    b4_offset_free: Vec<i8>,
    b4_offset_renju: Vec<i8>,
    /// Exact gain and open-four completion offsets when `P1 == A3`.
    a3_def_free: Vec<A3LineDefenses>,
    a3_def_renju: Vec<A3LineDefenses>,
}

static PATTERN_LUT: OnceLock<PatternLut> = OnceLock::new();

fn lut() -> &'static PatternLut {
    PATTERN_LUT.get_or_init(build_pattern_lut)
}

fn build_pattern_lut() -> PatternLut {
    let mut half_to_trit = [[0_u16; 1024]; 2];
    for bits in 0..1024_usize {
        for side_slot in 0..2_usize {
            let self_code = if side_slot == 0 {
                CELL_BLACK
            } else {
                CELL_WHITE
            };
            let mut trit = 0_u16;
            let mut mul = 1_u16;
            for i in 0..5 {
                let cell = ((bits >> (2 * i)) & 3) as u64;
                let digit = if cell == CELL_EMPTY {
                    0
                } else if cell == self_code {
                    1
                } else {
                    2
                };
                trit += digit * mul;
                mul *= 3;
            }
            half_to_trit[side_slot][bits] = trit;
        }
    }

    let mut p1_packed = vec![0_u8; LUT_SIZE];
    let mut b4_offset_free = vec![0_i8; LUT_SIZE];
    let mut b4_offset_renju = vec![0_i8; LUT_SIZE];
    let mut a3_def_free = vec![A3LineDefenses::default(); LUT_SIZE];
    let mut a3_def_renju = vec![A3LineDefenses::default(); LUT_SIZE];

    for left_trit in 0..TRIT_BASE {
        let mut w = [0_u8; 11];
        let mut rem = left_trit;
        for i in 0..5 {
            w[i] = (rem % 3) as u8;
            rem /= 3;
        }
        w[5] = 1; // candidate stone at center
        for right_trit in 0..TRIT_BASE {
            let mut rem = right_trit;
            for i in 0..5 {
                w[6 + i] = (rem % 3) as u8;
                rem /= 3;
            }
            let key = left_trit * TRIT_BASE + right_trit;
            let (p_free, off_free, def_free) = eval_window_1d(&w, false);
            let (p_renju, off_renju, def_renju) = eval_window_1d(&w, true);
            p1_packed[key] = (p_free as u8) | ((p_renju as u8) << 4);
            b4_offset_free[key] = off_free;
            b4_offset_renju[key] = off_renju;
            a3_def_free[key] = def_free;
            a3_def_renju[key] = def_renju;
        }
    }

    PatternLut {
        half_to_trit,
        p1_packed,
        b4_offset_free,
        b4_offset_renju,
        a3_def_free,
        a3_def_renju,
    }
}

/// Contiguous run length of `1`s passing through `pos` inside `w[0..11]`.
fn run_len_through(w: &[u8; 11], pos: usize) -> usize {
    debug_assert_eq!(w[pos], 1);
    let mut l = pos;
    while l > 0 && w[l - 1] == 1 {
        l -= 1;
    }
    let mut r = pos;
    while r + 1 < 11 && w[r + 1] == 1 {
        r += 1;
    }
    r - l + 1
}

/// True iff the contiguous run of `1`s through `pos` also covers `center` (`5`).
fn run_covers_center(w: &[u8; 11], pos: usize) -> (usize, bool) {
    let mut l = pos;
    while l > 0 && w[l - 1] == 1 {
        l -= 1;
    }
    let mut r = pos;
    while r + 1 < 11 && w[r + 1] == 1 {
        r += 1;
    }
    (r - l + 1, l <= 5 && 5 <= r)
}

/// Mirrors `crate::patterns::line::scan_a3` on `w[0..11]` centered at index `5`,
/// returning the 1 or 2 gain offsets (relative to `5`, in ascending order; `0` = unused).
fn scan_a3_window_gains(w: &[u8; 11]) -> [i8; 2] {
    for i in 2..=5 {
        let mut ones = 0_u8;
        let mut empties = 0_u8;
        let mut mask = 0_u8;
        for k in 0..4 {
            match w[i + k] {
                1 => {
                    ones += 1;
                    mask |= 1 << (3 - k);
                }
                0 => empties += 1,
                _ => {}
            }
        }
        if ones != 3 || empties != 1 {
            continue;
        }
        if mask == 0x0E {
            if w[i - 1] == 0 && w[i - 2] != 1 && w[i + 4] != 1 {
                if w[i - 2] == 0 && w[i + 4] == 0 {
                    return [(i - 1) as i8 - 5, (i + 3) as i8 - 5];
                }
                if w[i - 2] == 0 {
                    return [(i - 1) as i8 - 5, 0];
                }
                if w[i + 4] == 0 {
                    return [(i + 3) as i8 - 5, 0];
                }
            }
        }
        if mask == 0x0D && w[i - 1] == 0 && w[i + 4] == 0 {
            return [(i + 2) as i8 - 5, 0];
        }
        if mask == 0x0B && w[i - 1] == 0 && w[i + 4] == 0 {
            return [(i + 1) as i8 - 5, 0];
        }
    }
    [0, 0]
}

fn scan_a3_window_gains_reversed(w: &[u8; 11]) -> [i8; 2] {
    let mut rev = [0_u8; 11];
    for i in 0..11 {
        rev[i] = w[10 - i];
    }
    let raw = scan_a3_window_gains(&rev);
    match (raw[0], raw[1]) {
        (0, 0) => [0, 0],
        (a, 0) => [-a, 0],
        (a, b) => {
            let x = -a;
            let y = -b;
            if x < y {
                [x, y]
            } else {
                [y, x]
            }
        }
    }
}

/// Mirrors `rules::forbidden::DirectionalLine::{open_three_gain_indices, has_open_four_through_gain}`
/// on `w[0..11]` (anchor = 5) so `maybe_33` is a guaranteed superset of full Renju double-three.
fn has_renju_forbidden_open3_1d(w: &[u8; 11]) -> bool {
    let mut gains = [usize::MAX; 2];
    let mut gain_len = 0_usize;

    let mut idx = 5_usize;
    for _ in 0..4 {
        if idx == 0 {
            break;
        }
        idx -= 1;
        match w[idx] {
            0 => {
                gains[gain_len] = idx;
                gain_len += 1;
                break;
            }
            1 => {}
            _ => break,
        }
    }

    idx = 5;
    for _ in 0..4 {
        if idx + 1 >= 11 {
            break;
        }
        idx += 1;
        match w[idx] {
            0 => {
                gains[gain_len] = idx;
                gain_len += 1;
                break;
            }
            1 => {}
            _ => break,
        }
    }

    let mut tmp = *w;
    for &gain in &gains[..gain_len] {
        tmp[gain] = 1;
        let mut winning_extensions = 0_usize;
        for ext in 1..=9 {
            if ext == 5 || ext == gain || tmp[ext] != 0 {
                continue;
            }
            tmp[ext] = 1;
            let mut l = ext;
            while l > 0 && tmp[l - 1] == 1 {
                l -= 1;
            }
            let mut r = ext;
            while r + 1 < 11 && tmp[r + 1] == 1 {
                r += 1;
            }
            tmp[ext] = 0;
            if r - l + 1 == 5 && l <= 5 && 5 <= r && l <= gain && gain <= r {
                winning_extensions += 1;
                if winning_extensions >= 2 {
                    return true;
                }
            }
        }
        tmp[gain] = 0;
    }

    false
}

/// Evaluates the exact 1D pattern at `w[5] == 1` for either Freestyle (`renju_black = false`)
/// or Renju Black (`renju_black = true`).
fn eval_window_1d(w: &[u8; 11], renju_black: bool) -> (P1, i8, A3LineDefenses) {
    let empty_def = A3LineDefenses::default();
    let center_run = run_len_through(w, 5);
    if renju_black {
        if center_run == 5 {
            return (P1::Win5, 0, empty_def);
        }
        if center_run >= 6 {
            return (P1::Overline, 0, empty_def);
        }
    } else if center_run >= 5 {
        return (P1::Win5, 0, empty_def);
    }

    // Collect empty points k in 1..=9 (offsets -4..=+4) that complete a valid five
    // passing through center (index 5).
    let mut five_points = [0_usize; 9];
    let mut five_count = 0_usize;
    let mut tmp = *w;
    for k in 1..=9 {
        if k == 5 || tmp[k] != 0 {
            continue;
        }
        tmp[k] = 1;
        let (len, covers_center) = run_covers_center(&tmp, k);
        tmp[k] = 0;
        let is_win = covers_center && if renju_black { len == 5 } else { len >= 5 };
        if is_win {
            five_points[five_count] = k;
            five_count += 1;
        }
    }

    if five_count == 1 {
        return (P1::B4, five_points[0] as i8 - 5, empty_def);
    }
    if five_count >= 2 {
        // Check if this is a single straight open four (`_XXXX_`): exactly 2
        // completions separated by 5 with all 4 middle cells equal to 1.
        let is_straight_open_four = five_count == 2
            && five_points[1] - five_points[0] == 5
            && (five_points[0] + 1..five_points[1]).all(|i| w[i] == 1);
        if is_straight_open_four {
            return (P1::A4, 0, empty_def);
        }
        return (P1::DoubleFour1D, 0, empty_def);
    }

    // Collect all valid live-three gain points `g` in `2..=8` (`g != 5`) that
    // connect with `5` to form a straight open four (`_XXXX_`) whose two open
    // ends are both valid five-points.
    let mut a3_def = A3LineDefenses::default();
    let mut gain_count = 0_usize;
    for g in 2..=8 {
        if g == 5 || tmp[g] != 0 {
            continue;
        }
        tmp[g] = 1;
        let (run_len, covers_center) = run_covers_center(&tmp, g);
        if covers_center && run_len == 4 {
            let mut l = g;
            while l > 0 && tmp[l - 1] == 1 {
                l -= 1;
            }
            let r = l + 3;
            if l >= 2 && r <= 8 && tmp[l - 1] == 0 && tmp[r + 1] == 0 {
                let left_ok = !renju_black || tmp[l - 2] != 1;
                let right_ok = !renju_black || (r + 2 < 11 && tmp[r + 2] != 1);
                if left_ok && right_ok {
                    let g_valid = if renju_black {
                        let mut g_fives = 0_usize;
                        for k in 1..=9 {
                            if tmp[k] != 0 {
                                continue;
                            }
                            tmp[k] = 1;
                            let k_len = run_len_through(&tmp, k);
                            tmp[k] = 0;
                            if k_len == 5 {
                                g_fives += 1;
                            }
                        }
                        g_fives == 2
                    } else {
                        true
                    };
                    if g_valid {
                        if gain_count < 2 {
                            a3_def.gains[gain_count] = g as i8 - 5;
                            a3_def.completions[gain_count] = [(l - 1) as i8 - 5, (r + 1) as i8 - 5];
                        }
                        gain_count += 1;
                    }
                }
            }
        }
        tmp[g] = 0;
    }

    if gain_count > 0 {
        let fwd_gains = scan_a3_window_gains(w);
        let rev_gains = scan_a3_window_gains_reversed(w);
        if gain_count > 2 || a3_def.gains != fwd_gains || a3_def.gains != rev_gains {
            return (P1::A3, 0, A3LineDefenses::AMBIGUOUS);
        }
        return (P1::A3, 0, a3_def);
    }

    if renju_black && has_renju_forbidden_open3_1d(w) {
        return (P1::RenjuOpen3Candidate, 0, empty_def);
    }

    (P1::None, 0, empty_def)
}

// Direction definitions matching ThreatBoardView:
// dir 0: vertical   (x fixed, y = 0..15)  -> dx = 0,  dy = 1
// dir 1: horizontal (y fixed, x = 0..15)  -> dx = 1,  dy = 0
// dir 2: anti-diag  (x + y = p, pos = y)  -> dx = -1, dy = 1
// dir 3: main-diag  (14 - y + x = p, pos = 14 - y) -> dx = 1, dy = 1 (wait: let's match exact step!)
// Let's keep explicit, simple (dx, dy) steps so coordinate decoding is trivial and bug-free:
// dir 0: (dx=0, dy=1),  line_id = x,                        pos = y,            len = 15
// dir 1: (dx=1, dy=0),  line_id = 15 + y,                   pos = x,            len = 15
// dir 2: (dx=-1, dy=1), line_id = 30 + (x + y),             pos = y,            x0 = p, y0 = 0 (clipped)
// dir 3: (dx=1, dy=1),  line_id = 59 + (14 + x - y),        pos = x,            ...
// Even simpler: for dir 2 and dir 3, use `pos = y` (dir 2: dx=-1, dy=+1) and `pos = x` (dir 3: dx=+1, dy=+1)
// with board-range bounds automatically encoded as WALL bits inside each line's u64!
const DIR_STEPS: [(isize, isize); 4] = [(0, 1), (1, 0), (-1, 1), (1, 1)];

#[inline]
fn line_id_and_pos(x: usize, y: usize, dir: usize) -> (usize, usize) {
    match dir {
        0 => (x, y),
        1 => (BOARD_SIZE + y, x),
        2 => (2 * BOARD_SIZE + (x + y), y),
        _ => (
            2 * BOARD_SIZE + NUM_LINES_PER_DIAG + (BOARD_SIZE - 1 + x - y),
            x,
        ),
    }
}

#[inline]
pub(crate) fn col_major_idx(x: usize, y: usize) -> usize {
    x * BOARD_SIZE + y
}

#[inline]
pub(crate) fn idx_to_xy(idx: usize) -> (usize, usize) {
    (idx / BOARD_SIZE, idx % BOARD_SIZE)
}

#[inline]
pub(crate) fn idx_to_move(idx: usize) -> Move {
    let (x, y) = idx_to_xy(idx);
    xy_to_move(x, y).expect("idx stays in bounds")
}

#[inline]
fn side_slot(side: Side) -> usize {
    if side == BLACK {
        0
    } else {
        1
    }
}

/// Incremental 4-direction pattern and threat-bitset state synchronized with `Board`.
#[derive(Clone)]
pub(crate) struct Pattern4State {
    table: &'static PatternLut,
    rule_set: RuleSet,
    /// 88 lines, each holding 5 wall cells + 15 board/wall slots + 5 wall cells
    /// (`25 * 2 = 50` bits in a `u64`).
    lines: [u64; TOTAL_LINES],
    /// Occupied mask (bit `idx = x * 15 + y` is 1 iff cell is non-empty).
    occupied: BitBoard225,
    /// Per-side, per-cell, per-direction `P1` code (indexed by `[side_slot][idx][dir]`).
    p1: [[[u8; 4]; BOARD_AREA]; 2],
    /// Per-side, per-cell combined 4-direction summary.
    cell_pat: [[CellPattern; BOARD_AREA]; 2],
    /// Per-side threat bitsets (column-major `idx = x * 15 + y`).
    pub(crate) win5_bits: [BitBoard225; 2],
    pub(crate) a4_bits: [BitBoard225; 2],
    pub(crate) b4_bits: [BitBoard225; 2],
    pub(crate) a3_bits: [BitBoard225; 2],
}

impl std::fmt::Debug for Pattern4State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pattern4State")
            .field("rule_set", &self.rule_set)
            .field("occupied", &self.occupied)
            .finish()
    }
}

impl Pattern4State {
    pub(crate) fn from_board(board: &Board, rule_set: RuleSet) -> Self {
        let table = lut();
        let mut lines = [0_u64; TOTAL_LINES];
        // Initialize every slot in all 88 lines to WALL (`11`), then fill valid
        // board squares with EMPTY / BLACK / WHITE.
        let all_walls = (1_u64 << 50) - 1;
        lines.fill(all_walls);

        let mut occupied = BitBoard225::default();
        let grid = board.grid_rows();
        for x in 0..BOARD_SIZE {
            for y in 0..BOARD_SIZE {
                let stone = grid[y][x];
                let code = match stone {
                    EMPTY => CELL_EMPTY,
                    BLACK => CELL_BLACK,
                    _ => CELL_WHITE,
                };
                if stone != EMPTY {
                    occupied.set(col_major_idx(x, y));
                }
                for dir in 0..4 {
                    let (lid, pos) = line_id_and_pos(x, y, dir);
                    let shift = 2 * (pos + PAD);
                    lines[lid] = (lines[lid] & !(3_u64 << shift)) | (code << shift);
                }
            }
        }

        let mut state = Self {
            table,
            rule_set,
            lines,
            occupied,
            p1: [[[0_u8; 4]; BOARD_AREA]; 2],
            cell_pat: [[CellPattern::default(); BOARD_AREA]; 2],
            win5_bits: [BitBoard225::default(); 2],
            a4_bits: [BitBoard225::default(); 2],
            b4_bits: [BitBoard225::default(); 2],
            a3_bits: [BitBoard225::default(); 2],
        };

        for x in 0..BOARD_SIZE {
            for y in 0..BOARD_SIZE {
                let idx = col_major_idx(x, y);
                if state.occupied.contains(idx) {
                    continue;
                }
                state.recompute_empty_cell_all_dirs(x, y, idx);
            }
        }
        state
    }

    #[inline]
    fn lookup_key(&self, table: &PatternLut, lid: usize, pos: usize, slot: usize) -> usize {
        let bits = self.lines[lid];
        // Center is at `pos + PAD`. Left 5 cells start at `pos`, right 5 start at `pos + PAD + 1`.
        let left_bits = ((bits >> (2 * pos)) & 0x3FF) as usize;
        let right_bits = ((bits >> (2 * (pos + PAD + 1))) & 0x3FF) as usize;
        let lt = table.half_to_trit[slot][left_bits] as usize;
        let rt = table.half_to_trit[slot][right_bits] as usize;
        lt * TRIT_BASE + rt
    }

    #[inline]
    fn lookup_p1(&self, table: &PatternLut, lid: usize, pos: usize, slot: usize) -> u8 {
        let key = self.lookup_key(table, lid, pos, slot);
        let packed = table.p1_packed[key];
        if self.rule_set == RuleSet::Renju && slot == 0 {
            packed >> 4
        } else {
            packed & 0x0F
        }
    }

    #[inline]
    fn combine_cell_pattern(&self, slot: usize, p1s: [u8; 4]) -> CellPattern {
        let renju_black = self.rule_set == RuleSet::Renju && slot == 0;
        let mut has_win5 = false;
        let mut has_overline = false;
        let mut has_a4 = false;
        let mut four_count = 0_u8;
        let mut b4_dir = 0_u8;
        let mut a3_count = 0_u8;
        let mut open3_count = 0_u8;

        for (dir, &code) in p1s.iter().enumerate() {
            match P1::from_u8(code) {
                P1::None => {}
                P1::A3 => {
                    a3_count += 1;
                    open3_count += 1;
                }
                P1::RenjuOpen3Candidate => {
                    open3_count += 1;
                }
                P1::B4 => {
                    four_count += 1;
                    b4_dir = dir as u8;
                }
                P1::A4 => {
                    has_a4 = true;
                    four_count += 1;
                }
                P1::DoubleFour1D => {
                    four_count += 2;
                }
                P1::Win5 => has_win5 = true,
                P1::Overline => has_overline = true,
            }
        }

        if has_win5 {
            return CellPattern { packed: CLASS_WIN5 };
        }
        if renju_black {
            if has_overline || four_count >= 2 {
                return CellPattern {
                    packed: CLASS_FORBIDDEN,
                };
            }
            let maybe_33 = if open3_count >= 2 { FLAG_MAYBE_33 } else { 0 };
            if has_a4 {
                return CellPattern {
                    packed: CLASS_A4 | maybe_33 | FLAG_STRAIGHT_A4,
                };
            }
            if four_count == 1 {
                return CellPattern {
                    packed: CLASS_B4 | maybe_33 | (b4_dir << B4_DIR_SHIFT),
                };
            }
            if a3_count >= 1 {
                return CellPattern {
                    packed: CLASS_A3 | maybe_33,
                };
            }
            return CellPattern {
                packed: CLASS_NONE | maybe_33,
            };
        }

        // Freestyle or Renju White: no forbidden moves.
        let straight_flag = if has_a4 { FLAG_STRAIGHT_A4 } else { 0 };
        if has_a4 || four_count >= 2 {
            return CellPattern {
                packed: CLASS_A4 | straight_flag,
            };
        }
        if four_count == 1 {
            return CellPattern {
                packed: CLASS_B4 | (b4_dir << B4_DIR_SHIFT),
            };
        }
        if a3_count >= 1 {
            return CellPattern { packed: CLASS_A3 };
        }
        CellPattern { packed: CLASS_NONE }
    }

    #[inline]
    fn apply_cell_pattern(&mut self, slot: usize, idx: usize, new_pat: CellPattern) {
        let old_pat = self.cell_pat[slot][idx];
        if old_pat == new_pat {
            return;
        }
        match old_pat.packed & CLASS_MASK {
            CLASS_WIN5 => self.win5_bits[slot].clear(idx),
            CLASS_A4 => self.a4_bits[slot].clear(idx),
            CLASS_B4 => self.b4_bits[slot].clear(idx),
            CLASS_A3 => self.a3_bits[slot].clear(idx),
            _ => {}
        }
        self.cell_pat[slot][idx] = new_pat;
        match new_pat.packed & CLASS_MASK {
            CLASS_WIN5 => self.win5_bits[slot].set(idx),
            CLASS_A4 => self.a4_bits[slot].set(idx),
            CLASS_B4 => self.b4_bits[slot].set(idx),
            CLASS_A3 => self.a3_bits[slot].set(idx),
            _ => {}
        }
    }

    fn recompute_empty_cell_all_dirs(&mut self, x: usize, y: usize, idx: usize) {
        let table = self.table;
        for dir in 0..4 {
            let (lid, pos) = line_id_and_pos(x, y, dir);
            self.p1[0][idx][dir] = self.lookup_p1(table, lid, pos, 0);
            self.p1[1][idx][dir] = self.lookup_p1(table, lid, pos, 1);
        }
        for slot in 0..2 {
            let pat = self.combine_cell_pattern(slot, self.p1[slot][idx]);
            self.apply_cell_pattern(slot, idx, pat);
        }
    }

    #[inline]
    fn refresh_neighbors_along_axes(&mut self, x: usize, y: usize, idx: usize) {
        const IDX_STEPS: [isize; 4] = [
            1,
            BOARD_SIZE as isize,
            -(BOARD_SIZE as isize) + 1,
            BOARD_SIZE as isize + 1,
        ];
        let table = self.table;
        let xi = x as isize;
        let yi = y as isize;
        let max_coord = (BOARD_SIZE - 1) as isize;
        let renju = self.rule_set == RuleSet::Renju;

        for dir in 0..4 {
            let (lid, pos) = line_id_and_pos(x, y, dir);
            let bits = self.lines[lid];
            let (min_step, max_step) = match dir {
                0 => ((-yi).max(-5), (max_coord - yi).min(5)),
                1 => ((-xi).max(-5), (max_coord - xi).min(5)),
                2 => (
                    (-(max_coord - xi).min(yi)).max(-5),
                    xi.min(max_coord - yi).min(5),
                ),
                _ => (
                    (-xi.min(yi)).max(-5),
                    (max_coord - xi).min(max_coord - yi).min(5),
                ),
            };
            let idx_step = IDX_STEPS[dir];
            for step in min_step..=max_step {
                if step == 0 {
                    continue;
                }
                let nidx = (idx as isize + step * idx_step) as usize;
                if self.occupied.contains(nidx) {
                    continue;
                }
                let npos = (pos as isize + step) as usize;
                let left_bits = ((bits >> (2 * npos)) & 0x3FF) as usize;
                let right_bits = ((bits >> (2 * (npos + PAD + 1))) & 0x3FF) as usize;

                // Slot 0 (Black)
                let key0 = table.half_to_trit[0][left_bits] as usize * TRIT_BASE
                    + table.half_to_trit[0][right_bits] as usize;
                let packed0 = table.p1_packed[key0];
                let new_p1_0 = if renju { packed0 >> 4 } else { packed0 & 0x0F };
                if new_p1_0 != self.p1[0][nidx][dir] {
                    self.p1[0][nidx][dir] = new_p1_0;
                    let new_pat = self.combine_cell_pattern(0, self.p1[0][nidx]);
                    self.apply_cell_pattern(0, nidx, new_pat);
                }

                // Slot 1 (White)
                let key1 = table.half_to_trit[1][left_bits] as usize * TRIT_BASE
                    + table.half_to_trit[1][right_bits] as usize;
                let new_p1_1 = table.p1_packed[key1] & 0x0F;
                if new_p1_1 != self.p1[1][nidx][dir] {
                    self.p1[1][nidx][dir] = new_p1_1;
                    let new_pat = self.combine_cell_pattern(1, self.p1[1][nidx]);
                    self.apply_cell_pattern(1, nidx, new_pat);
                }
            }
        }
    }

    pub(crate) fn play(&mut self, move_: Move, side: Side) {
        let (x, y) = move_to_xy(move_).expect("move stays in range");
        let idx = col_major_idx(x, y);
        let code = if side == BLACK {
            CELL_BLACK
        } else {
            CELL_WHITE
        };
        self.occupied.set(idx);
        for dir in 0..4 {
            let (lid, pos) = line_id_and_pos(x, y, dir);
            let shift = 2 * (pos + PAD);
            self.lines[lid] = (self.lines[lid] & !(3_u64 << shift)) | (code << shift);
        }
        for slot in 0..2 {
            self.p1[slot][idx] = [0; 4];
            self.apply_cell_pattern(slot, idx, CellPattern::default());
        }
        self.refresh_neighbors_along_axes(x, y, idx);
    }

    pub(crate) fn undo(&mut self, move_: Move) {
        let (x, y) = move_to_xy(move_).expect("move stays in range");
        let idx = col_major_idx(x, y);
        self.occupied.clear(idx);
        for dir in 0..4 {
            let (lid, pos) = line_id_and_pos(x, y, dir);
            let shift = 2 * (pos + PAD);
            self.lines[lid] &= !(3_u64 << shift);
        }
        self.recompute_empty_cell_all_dirs(x, y, idx);
        self.refresh_neighbors_along_axes(x, y, idx);
    }

    #[inline]
    pub(crate) fn win5_bits(&self, side: Side) -> BitBoard225 {
        self.win5_bits[side_slot(side)]
    }

    #[inline]
    pub(crate) fn a4_bits(&self, side: Side) -> BitBoard225 {
        self.a4_bits[side_slot(side)]
    }

    #[inline]
    pub(crate) fn b4_bits(&self, side: Side) -> BitBoard225 {
        self.b4_bits[side_slot(side)]
    }

    #[inline]
    pub(crate) fn a3_bits(&self, side: Side) -> BitBoard225 {
        self.a3_bits[side_slot(side)]
    }

    #[inline]
    pub(crate) fn cell_pattern(&self, side: Side, idx: usize) -> CellPattern {
        self.cell_pat[side_slot(side)][idx]
    }

    #[inline]
    pub(crate) fn p1_code(&self, side: Side, idx: usize, dir: usize) -> P1 {
        P1::from_u8(self.p1[side_slot(side)][idx][dir])
    }

    #[inline]
    pub(crate) fn dir_step(dir: usize) -> (isize, isize) {
        DIR_STEPS[dir]
    }

    /// Returns the exact 1D gain and open-four completion offsets for an empty
    /// cell `idx` whose `dir` pattern is `P1::A3`.
    #[inline]
    pub(crate) fn a3_line_defenses(&self, side: Side, idx: usize, dir: usize) -> A3LineDefenses {
        let slot = side_slot(side);
        let (x, y) = idx_to_xy(idx);
        let (lid, pos) = line_id_and_pos(x, y, dir);
        let table = self.table;
        let key = self.lookup_key(table, lid, pos, slot);
        if self.rule_set == RuleSet::Renju && slot == 0 {
            table.a3_def_renju[key]
        } else {
            table.a3_def_free[key]
        }
    }

    /// For Renju Black, evaluates whether playing `gain` (at `gidx`) after
    /// placing a Black stone on axis `a3_dir` (which turns `a3_dir` at `gidx`
    /// into a single `A4`) is definitely legal (`Some(true)`), definitely
    /// forbidden (`Some(false)`), or requires recursive 3-3 verification (`None`).
    /// The other 3 axes through `gidx` are unaffected by placing on `a3_dir`.
    #[inline]
    pub(crate) fn renju_black_a3_gain_legality(&self, gidx: usize, a3_dir: usize) -> Option<bool> {
        let p1s = self.p1[0][gidx];
        let mut a3_count = 0_u8;
        let mut has_four_or_overline = false;
        for (d, &code) in p1s.iter().enumerate() {
            if d == a3_dir {
                continue;
            }
            match P1::from_u8(code) {
                P1::Win5 => return Some(true),
                P1::Overline | P1::B4 | P1::A4 | P1::DoubleFour1D => {
                    has_four_or_overline = true;
                }
                P1::A3 | P1::RenjuOpen3Candidate => a3_count += 1,
                P1::None => {}
            }
        }
        if has_four_or_overline {
            return Some(false);
        }
        if a3_count >= 2 {
            return None;
        }
        Some(true)
    }

    /// Checks whether a defender-black `a3` gain at `gidx` would trigger
    /// `a5test` (i.e. `WIN5` or `Overline`) on any axis other than `a3_dir`.
    #[inline]
    pub(crate) fn black_gain_has_other_a5(&self, gidx: usize, a3_dir: usize) -> bool {
        let p1s = self.p1[0][gidx];
        for (d, &code) in p1s.iter().enumerate() {
            if d == a3_dir {
                continue;
            }
            if matches!(P1::from_u8(code), P1::Win5 | P1::Overline) {
                return true;
            }
        }
        false
    }

    /// Returns the exact winning five-point created by playing `side` at empty
    /// cell `idx` when `cell_pattern(side, idx)` is `CLASS_B4`.
    #[inline]
    pub(crate) fn b4_reply_move(&self, side: Side, idx: usize) -> Option<Move> {
        let slot = side_slot(side);
        let pat = self.cell_pat[slot][idx];
        debug_assert_eq!(pat.packed & CLASS_MASK, CLASS_B4);
        let dir = pat.b4_dir();
        let (x, y) = idx_to_xy(idx);
        let (lid, pos) = line_id_and_pos(x, y, dir);
        let table = self.table;
        let key = self.lookup_key(table, lid, pos, slot);
        let offset = if self.rule_set == RuleSet::Renju && slot == 0 {
            table.b4_offset_renju[key]
        } else {
            table.b4_offset_free[key]
        };
        if offset == 0 {
            return None;
        }
        let (dx, dy) = DIR_STEPS[dir];
        let tx = x as isize + dx * offset as isize;
        let ty = y as isize + dy * offset as isize;
        if tx >= 0 && ty >= 0 && tx < BOARD_SIZE as isize && ty < BOARD_SIZE as isize {
            xy_to_move(tx as usize, ty as usize).ok()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::WHITE;
    use crate::threats::ThreatBoardView;

    #[test]
    fn incremental_play_undo_matches_fresh_rebuild_for_both_rules() {
        let mut seed = 0x51a7_c0de_cafe_1234_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            let mut board = Board::new();
            let mut state = Pattern4State::from_board(&board, rule);
            let mut played = Vec::new();
            let mut side = BLACK;

            for _ in 0..70 {
                let mut candidates = Vec::new();
                for x in 0..BOARD_SIZE {
                    for y in 0..BOARD_SIZE {
                        let m = xy_to_move(x, y).unwrap();
                        if board.is_legal_move_for_rule(m, side, rule) {
                            candidates.push(m);
                        }
                    }
                }
                if candidates.is_empty() || board.winner() != EMPTY {
                    break;
                }
                let m = candidates[(next() as usize) % candidates.len()];
                board.play_for_rule(m, Some(side), rule).unwrap();
                state.play(m, side);
                played.push(m);

                let fresh = Pattern4State::from_board(&board, rule);
                assert_eq!(state.win5_bits, fresh.win5_bits);
                assert_eq!(state.a4_bits, fresh.a4_bits);
                assert_eq!(state.b4_bits, fresh.b4_bits);
                assert_eq!(state.a3_bits, fresh.a3_bits);
                assert_eq!(state.cell_pat, fresh.cell_pat);
                side = -side;
            }

            while let Some(m) = played.pop() {
                board.undo().unwrap();
                state.undo(m);
                let fresh = Pattern4State::from_board(&board, rule);
                assert_eq!(state.win5_bits, fresh.win5_bits);
                assert_eq!(state.a4_bits, fresh.a4_bits);
                assert_eq!(state.b4_bits, fresh.b4_bits);
                assert_eq!(state.a3_bits, fresh.a3_bits);
                assert_eq!(state.cell_pat, fresh.cell_pat);
            }
        }
    }

    #[test]
    fn pattern4_win5_a4_b4_and_b4_reply_match_threat_board_oracle() {
        let mut seed = 0xfeed_face_0123_4567_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            for trial in 0..1000 {
                let mut board = Board::new();
                let stones = 10 + (next() % 35) as usize;
                let mut side = BLACK;
                for _ in 0..stones {
                    if board.winner() != EMPTY {
                        break;
                    }
                    let x = (next() % BOARD_SIZE as u64) as usize;
                    let y = (next() % BOARD_SIZE as u64) as usize;
                    let m = xy_to_move(x, y).unwrap();
                    if board.is_legal_move_for_rule(m, side, rule) {
                        board.play_for_rule(m, Some(side), rule).unwrap();
                        side = -side;
                    }
                }
                if board.winner() != EMPTY {
                    continue;
                }

                let state = Pattern4State::from_board(&board, rule);
                let mut view = ThreatBoardView::from_board_with_rule(board.clone(), rule);

                for check_side in [BLACK, WHITE] {
                    let slot = side_slot(check_side);
                    for x in 0..BOARD_SIZE {
                        for y in 0..BOARD_SIZE {
                            if board.grid_rows()[y][x] != EMPTY {
                                continue;
                            }
                            let m = xy_to_move(x, y).unwrap();
                            let idx = col_major_idx(x, y);
                            let pat = state.cell_pattern(check_side, idx);

                            // 1. WIN5 must match `is_winning_move_for_rule` exactly.
                            let oracle_win5 =
                                board.is_winning_move_for_rule(x, y, check_side, rule);
                            assert_eq!(
                                state.win5_bits[slot].contains(idx),
                                oracle_win5,
                                "win5 mismatch rule={rule:?} trial={trial} side={check_side} ({x},{y})"
                            );

                            // 2. Definite forbidden must be illegal; non-forbidden without
                            //    maybe_33 must be legal.
                            let oracle_legal = board.is_legal_move_for_rule(m, check_side, rule);
                            if pat.is_forbidden_definite() {
                                assert!(
                                    !oracle_legal,
                                    "false definite forbidden rule={rule:?} trial={trial} ({x},{y})"
                                );
                                continue;
                            }
                            if !pat.maybe_33() {
                                assert!(
                                    oracle_legal,
                                    "missed forbidden rule={rule:?} trial={trial} ({x},{y})"
                                );
                            }
                            if !oracle_legal || oracle_win5 {
                                continue;
                            }

                            // 3. For legal non-WIN5 moves, compare A4 / B4 / B4-reply when
                            //    the side has no pre-existing winning five elsewhere.
                            if !state.win5_bits[slot].is_empty() {
                                continue;
                            }
                            view.play(m, check_side);
                            let oracle_attack = view.classify_attack_at(x, y, check_side, m);
                            view.undo();

                            let is_a4 = state.a4_bits[slot].contains(idx);
                            let is_b4 = state.b4_bits[slot].contains(idx);
                            let is_a3 = state.a3_bits[slot].contains(idx);

                            if is_a4 {
                                // In Freestyle, legacy `scan_b4p` misses rare single-line
                                // asymmetric double-jump-fours (`WW_WWW_W`) and reports B4
                                // even though two distinct winning five-points exist. Verify
                                // `legal_win_completions >= 2` matches in that case.
                                if rule == RuleSet::Freestyle
                                    && oracle_attack.as_ref().map(|a| a.level)
                                        == Some(crate::threats::ThreatLevel::B4)
                                {
                                    view.play(m, check_side);
                                    let mut wins = 0;
                                    for rx in 0..BOARD_SIZE {
                                        for ry in 0..BOARD_SIZE {
                                            if view.board.grid_rows()[ry][rx] == EMPTY
                                                && view.board.is_winning_move_for_rule(
                                                    rx, ry, check_side, rule,
                                                )
                                            {
                                                wins += 1;
                                            }
                                        }
                                    }
                                    view.undo();
                                    assert!(
                                        wins >= 2,
                                        "false A4 rule={rule:?} trial={trial} side={check_side} ({x},{y})"
                                    );
                                } else {
                                    assert_eq!(
                                        oracle_attack.as_ref().map(|a| a.level),
                                        Some(crate::threats::ThreatLevel::A4),
                                        "A4 mismatch rule={rule:?} trial={trial} side={check_side} ({x},{y}) p1={:?}",
                                        state.p1[slot][idx],
                                    );
                                }
                            } else if is_b4 {
                                let reply = state
                                    .b4_reply_move(check_side, idx)
                                    .expect("B4 must decode a reply");
                                let expected_level =
                                    if board.is_legal_move_for_rule(reply, -check_side, rule) {
                                        crate::threats::ThreatLevel::B4
                                    } else {
                                        crate::threats::ThreatLevel::A4
                                    };
                                assert_eq!(
                                    oracle_attack.as_ref().map(|a| a.level),
                                    Some(expected_level),
                                    "B4 level mismatch rule={rule:?} trial={trial} side={check_side} ({x},{y})"
                                );
                                if expected_level == crate::threats::ThreatLevel::B4 {
                                    assert_eq!(
                                        oracle_attack.as_ref().unwrap().defenses,
                                        vec![reply],
                                        "B4 reply mismatch rule={rule:?} trial={trial} side={check_side} ({x},{y})"
                                    );
                                }
                            } else if let Some(atk) = oracle_attack {
                                // Any A3 from oracle must be covered by a3_bits (sound superset).
                                assert_eq!(
                                    atk.level,
                                    crate::threats::ThreatLevel::A3,
                                    "missed four/win5 rule={rule:?} trial={trial} side={check_side} ({x},{y}): {atk:?}"
                                );
                                assert!(
                                    is_a3,
                                    "missed A3 candidate rule={rule:?} trial={trial} side={check_side} ({x},{y})"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
