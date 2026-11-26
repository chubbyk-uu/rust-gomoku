//! Rapfi-aligned dense line pattern, 14-level `Pattern4`, order-independent
//! `PatternCode`, and tactical defence lookup tables.
//!
//! Each line cell is encoded as 2 bits:
//! - `0b00`: wall (off-board)
//! - `0b01`: white stone
//! - `0b10`: black stone
//! - `0b11`: empty cell
//!
//! Notice placing a stone of `color` (`BLACK = 0`, `WHITE = 1`) on an empty cell
//! simply clears bit `color` (`key ^= 1 << (2 * slot + color)`).

use std::sync::OnceLock;

/// Side color in `fast_engine`: `BLACK = 0`, `WHITE = 1` so `side ^ 1` flips sides.
pub const FE_BLACK: usize = 0;
pub const FE_WHITE: usize = 1;

/// Single-direction line pattern at an empty cell, ordered by threat strength.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pattern {
    Dead = 0,
    Ol = 1,
    B1 = 2,
    F1 = 3,
    B2 = 4,
    F2 = 5,
    F2A = 6,
    F2B = 7,
    B3 = 8,
    B3S = 9,
    F3 = 10,
    F3S = 11,
    B4 = 12,
    B4S = 13,
    F4 = 14,
    F5 = 15,
}

pub const PATTERN_NB: usize = 16;

impl Pattern {
    #[inline(always)]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Dead,
            1 => Self::Ol,
            2 => Self::B1,
            3 => Self::F1,
            4 => Self::B2,
            5 => Self::F2,
            6 => Self::F2A,
            7 => Self::F2B,
            8 => Self::B3,
            9 => Self::B3S,
            10 => Self::F3,
            11 => Self::F3S,
            12 => Self::B4,
            13 => Self::B4S,
            14 => Self::F4,
            _ => Self::F5,
        }
    }
}

/// Aggregate 4-direction pattern at an empty cell, ordered from `None` to `AFive`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pattern4 {
    None = 0,
    Forbid = 1,
    LFlex2 = 2,
    KBlock3 = 3,
    JFlex2x2 = 4,
    IBlock3Plus = 5,
    HFlex3 = 6,
    GFlex3Plus = 7,
    FFlex3x2 = 8,
    EBlock4 = 9,
    DBlock4Plus = 10,
    CBlock4Flex3 = 11,
    BFlex4 = 12,
    AFive = 13,
}

pub const PATTERN4_NB: usize = 14;

impl Pattern4 {
    #[inline(always)]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Forbid,
            2 => Self::LFlex2,
            3 => Self::KBlock3,
            4 => Self::JFlex2x2,
            5 => Self::IBlock3Plus,
            6 => Self::HFlex3,
            7 => Self::GFlex3Plus,
            8 => Self::FFlex3x2,
            9 => Self::EBlock4,
            10 => Self::DBlock4Plus,
            11 => Self::CBlock4Flex3,
            12 => Self::BFlex4,
            _ => Self::AFive,
        }
    }
}

/// Packs both colors' single-direction `Pattern` into one byte (low nibble = Black,
/// high nibble = White).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct Pattern2x(pub u8);

impl Pattern2x {
    #[inline(always)]
    pub const fn new(black: Pattern, white: Pattern) -> Self {
        Self((black as u8) | ((white as u8) << 4))
    }

    #[inline(always)]
    pub const fn black(self) -> Pattern {
        Pattern::from_u8(self.0 & 0x0F)
    }

    #[inline(always)]
    pub const fn white(self) -> Pattern {
        Pattern::from_u8(self.0 >> 4)
    }

    #[inline(always)]
    pub const fn black_raw(self) -> usize {
        (self.0 & 0x0F) as usize
    }

    #[inline(always)]
    pub const fn white_raw(self) -> usize {
        (self.0 >> 4) as usize
    }

    #[inline(always)]
    pub const fn for_side(self, side: usize) -> Pattern {
        Pattern::from_u8((self.0 >> (side * 4)) & 0x0F)
    }
}

pub const PCODE_BITS: u32 = 12;
pub const PCODE_MASK: u16 = (1 << PCODE_BITS) - 1;
pub const PCODE_NB: usize = 3876; // C(16 + 4 - 1, 4)

/// Fused 16-bit entry carrying the order-independent 4-direction `PatternCode`
/// in bits `0..12` and the aggregate `Pattern4` in bits `12..16`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PCodeP4(pub u16);

impl PCodeP4 {
    #[inline(always)]
    pub const fn new(pcode: u16, p4: Pattern4) -> Self {
        Self((pcode & PCODE_MASK) | ((p4 as u16) << PCODE_BITS))
    }

    #[inline(always)]
    pub const fn pcode(self) -> u16 {
        self.0 & PCODE_MASK
    }

    #[inline(always)]
    pub const fn pattern4(self) -> Pattern4 {
        Pattern4::from_u8((self.0 >> PCODE_BITS) as u8)
    }

    #[inline(always)]
    pub const fn pattern4_raw(self) -> u8 {
        (self.0 >> PCODE_BITS) as u8
    }
}

pub const HALF_LEN_FREESTYLE: usize = 4;
pub const HALF_LEN_RENJU: usize = 5;

pub const DENSE_HALF_FREESTYLE: usize = 121; // (3^5 - 1) / 2
pub const DENSE_KEY_FREESTYLE: usize = DENSE_HALF_FREESTYLE * DENSE_HALF_FREESTYLE; // 14,641

pub const DENSE_HALF_RENJU: usize = 364; // (3^6 - 1) / 2
pub const DENSE_KEY_RENJU: usize = DENSE_HALF_RENJU * DENSE_HALF_RENJU; // 132,496

const fn dense_half_code(h: usize, near_at_low_bits: bool, half: u32) -> i32 {
    let mut code = 0_i32;
    let mut pow3 = 1_i32;
    let mut wall_seen = false;
    let mut i = 0_usize;
    while i < h {
        let shift = if near_at_low_bits {
            2 * i
        } else {
            2 * (h - 1 - i)
        };
        let cell = (half >> shift) & 0b11;
        if cell == 0b00 {
            wall_seen = true;
            i += 1;
            continue;
        }
        if wall_seen {
            return -1;
        }
        let digit = if cell == 0b11 {
            0
        } else if cell == 0b10 {
            1
        } else {
            2
        };
        code += digit * pow3;
        pow3 *= 3;
        i += 1;
    }
    (pow3 - 1) / 2 + code
}

const fn make_half_table_u16<const N: usize>(
    h: usize,
    near_at_low_bits: bool,
    scale: i32,
) -> [u16; N] {
    let mut table = [0_u16; N];
    let mut half = 0_usize;
    while half < N {
        let code = dense_half_code(h, near_at_low_bits, half as u32);
        table[half] = if code < 0 { 0 } else { (code * scale) as u16 };
        half += 1;
    }
    table
}

const fn make_half_table_u32<const N: usize>(
    h: usize,
    near_at_low_bits: bool,
    scale: i32,
) -> [u32; N] {
    let mut table = [0_u32; N];
    let mut half = 0_usize;
    while half < N {
        let code = dense_half_code(h, near_at_low_bits, half as u32);
        table[half] = if code < 0 { 0 } else { (code * scale) as u32 };
        half += 1;
    }
    table
}

static HALF_CODE_LO_F: [u16; 256] =
    make_half_table_u16::<256>(HALF_LEN_FREESTYLE, false, DENSE_HALF_FREESTYLE as i32);
static HALF_CODE_HI_F: [u16; 256] = make_half_table_u16::<256>(HALF_LEN_FREESTYLE, true, 1);
static HALF_CODE_LO_R: [u32; 1024] =
    make_half_table_u32::<1024>(HALF_LEN_RENJU, false, DENSE_HALF_RENJU as i32);
static HALF_CODE_HI_R: [u16; 1024] = make_half_table_u16::<1024>(HALF_LEN_RENJU, true, 1);

/// Compute dense table index for a runtime line key with center cell at bit `2*H`.
#[inline(always)]
pub fn dense_key_freestyle(key: u64) -> usize {
    let lo = (key as usize) & 0xFF;
    let hi = ((key >> 10) as usize) & 0xFF;
    (HALF_CODE_LO_F[lo] as usize) + (HALF_CODE_HI_F[hi] as usize)
}

#[inline(always)]
pub fn dense_key_renju(key: u64) -> usize {
    let lo = (key as usize) & 0x3FF;
    let hi = ((key >> 12) as usize) & 0x3FF;
    (HALF_CODE_LO_R[lo] as usize) + (HALF_CODE_HI_R[hi] as usize)
}

#[inline(always)]
fn dense_fused_index<const RENJU: bool>(key: u64) -> usize {
    if RENJU {
        let lo = (key as usize) & 0x3FF;
        let hi = ((key >> 10) as usize) & 0x3FF;
        (HALF_CODE_LO_R[lo] as usize) + (HALF_CODE_HI_R[hi] as usize)
    } else {
        let lo = (key as usize) & 0xFF;
        let hi = ((key >> 8) as usize) & 0xFF;
        (HALF_CODE_LO_F[lo] as usize) + (HALF_CODE_HI_F[hi] as usize)
    }
}

/// Precomputed global lookup tables for `FastBoard`.
pub struct PatternTables {
    pub pattern2x_freestyle: Box<[Pattern2x; DENSE_KEY_FREESTYLE]>,
    pub pattern2x_renju: Box<[Pattern2x; DENSE_KEY_RENJU]>,
    pub defence_freestyle: Box<[[u8; 2]; DENSE_KEY_FREESTYLE]>,
    pub defence_renju: Box<[[u8; 2]; DENSE_KEY_RENJU]>,
    pub pcode: Box<[PCodeP4; 65_536]>,
    pub pcode_renju_black: Box<[PCodeP4; 65_536]>,
}

static TABLES: OnceLock<PatternTables> = OnceLock::new();

#[inline]
pub fn tables() -> &'static PatternTables {
    TABLES.get_or_init(PatternTables::build)
}

#[inline(always)]
pub fn lookup_pattern<const RENJU: bool>(t: &PatternTables, key: u64) -> Pattern2x {
    // SAFETY: `HALF_CODE_LO_*[lo] + HALF_CODE_HI_*[hi]` is constructed at compile time
    // to stay strictly within `0..DENSE_KEY_*`.
    unsafe {
        if RENJU {
            *t.pattern2x_renju.get_unchecked(dense_key_renju(key))
        } else {
            *t.pattern2x_freestyle
                .get_unchecked(dense_key_freestyle(key))
        }
    }
}

#[inline(always)]
pub fn lookup_defence<const RENJU: bool>(t: &PatternTables, key: u64, attacker: usize) -> u8 {
    unsafe {
        if RENJU {
            *t.defence_renju
                .get_unchecked(dense_key_renju(key))
                .get_unchecked(attacker)
        } else {
            *t.defence_freestyle
                .get_unchecked(dense_key_freestyle(key))
                .get_unchecked(attacker)
        }
    }
}

#[inline(always)]
pub fn pcode_table_for<const RENJU: bool>(t: &PatternTables, side: usize) -> &[PCodeP4; 65_536] {
    if RENJU && side == FE_BLACK {
        &t.pcode_renju_black
    } else {
        &t.pcode
    }
}

// ---------------------------------------------------------------------------
// Table construction (exact DP matching Rapfi `game/pattern.cpp`)

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ColorFlag {
    SelfStone = 0,
    Oppo = 1,
    Empty = 2,
}

const NIL_PAT: u8 = 0xFF;

fn raw_half_from_dense(h: usize, near_at_low_bits: bool, mut code: usize) -> u32 {
    let mut v = 0_usize;
    let mut block_size = 1_usize;
    while code >= block_size {
        code -= block_size;
        block_size *= 3;
        v += 1;
    }
    debug_assert!(v <= h);
    const CELL_BITS: [u32; 3] = [0b11, 0b10, 0b01]; // empty, black, white
    let mut half = 0_u32;
    for k in 0..v {
        let shift = if near_at_low_bits {
            2 * k
        } else {
            2 * (h - 1 - k)
        };
        half |= CELL_BITS[code % 3] << shift;
        code /= 3;
    }
    half
}

fn decode_line<const LEN: usize>(key: u64, self_side: usize) -> [ColorFlag; LEN] {
    let mid = LEN / 2;
    let mut line = [ColorFlag::Empty; LEN];
    for i in 0..LEN {
        if i == mid {
            line[i] = ColorFlag::SelfStone;
            continue;
        }
        let bit_pos = if i < mid { 2 * i } else { 2 * (i - 1) };
        let bits = (key >> bit_pos) & 0b11;
        let c_black = (bits & 1) != 0;
        let c_white = (bits & 2) != 0;
        let opp_bit = if self_side == FE_BLACK {
            c_white
        } else {
            c_black
        };
        line[i] = if c_black && c_white {
            ColorFlag::Empty
        } else if opp_bit {
            ColorFlag::SelfStone
        } else {
            ColorFlag::Oppo
        };
    }
    line
}

fn line_code<const LEN: usize>(line: &[ColorFlag; LEN]) -> usize {
    let mut code = 0_usize;
    for &c in line {
        code = code * 3 + (c as usize);
    }
    code
}

fn count_line<const LEN: usize>(line: &[ColorFlag; LEN]) -> (usize, usize, usize, usize) {
    let mid = LEN / 2;
    let mut real_len = 1_usize;
    let mut full_len = 1_usize;
    let mut real_inc = 1_usize;
    let mut start = mid;
    let mut end = mid;

    for i in (0..mid).rev() {
        match line[i] {
            ColorFlag::SelfStone => real_len += real_inc,
            ColorFlag::Oppo => break,
            ColorFlag::Empty => real_inc = 0,
        }
        full_len += 1;
        start = i;
    }

    real_inc = 1;
    for i in (mid + 1)..LEN {
        match line[i] {
            ColorFlag::SelfStone => real_len += real_inc,
            ColorFlag::Oppo => break,
            ColorFlag::Empty => real_inc = 0,
        }
        full_len += 1;
        end = i;
    }

    (real_len, full_len, start, end)
}

fn shift_line<const LEN: usize>(line: &[ColorFlag; LEN], center: usize) -> [ColorFlag; LEN] {
    let mid = LEN / 2;
    let mut out = [ColorFlag::Oppo; LEN];
    for j in 0..LEN {
        let idx = j as isize + center as isize - mid as isize;
        if idx >= 0 && (idx as usize) < LEN {
            out[j] = line[idx as usize];
        }
    }
    out
}

fn get_pattern<const LEN: usize, const RENJU_BLACK: bool>(
    memo: &mut [u8],
    line: &[ColorFlag; LEN],
) -> Pattern {
    let code = line_code(line);
    if memo[code] != NIL_PAT {
        return Pattern::from_u8(memo[code]);
    }

    let mid = LEN / 2;
    let (real_len, full_len, start, end) = count_line(line);

    let pat = if RENJU_BLACK && real_len >= 6 {
        Pattern::Ol
    } else if real_len >= 5 {
        Pattern::F5
    } else if full_len < 5 {
        Pattern::Dead
    } else {
        let mut pat_cnt = [0_u8; PATTERN_NB];
        let mut f5_idx = [0_usize; 2];

        for i in start..=end {
            if line[i] == ColorFlag::Empty {
                let mut sl = shift_line(line, i);
                sl[mid] = ColorFlag::SelfStone;
                let slp = get_pattern::<LEN, RENJU_BLACK>(memo, &sl);
                if slp == Pattern::F5 && (pat_cnt[Pattern::F5 as usize] as usize) < f5_idx.len() {
                    f5_idx[pat_cnt[Pattern::F5 as usize] as usize] = i;
                }
                pat_cnt[slp as usize] += 1;
            }
        }

        if pat_cnt[Pattern::F5 as usize] >= 2 {
            if RENJU_BLACK && f5_idx[1] - f5_idx[0] < 5 {
                // Single-line double-four (e.g. OXX_X*X_XXO) is forbidden for Renju Black.
                Pattern::Ol
            } else {
                Pattern::F4
            }
        } else if pat_cnt[Pattern::F5 as usize] == 1 {
            let mut blocked = *line;
            blocked[f5_idx[0]] = ColorFlag::Oppo;
            if get_pattern::<LEN, RENJU_BLACK>(memo, &blocked) >= Pattern::B3 {
                Pattern::B4S
            } else {
                Pattern::B4
            }
        } else if pat_cnt[Pattern::F4 as usize] >= 2 {
            Pattern::F3S
        } else if pat_cnt[Pattern::F4 as usize] > 0 {
            Pattern::F3
        } else if pat_cnt[Pattern::B4S as usize] > 0 {
            Pattern::B3S
        } else if pat_cnt[Pattern::B4 as usize] > 0 {
            Pattern::B3
        } else if pat_cnt[Pattern::F3S as usize] + pat_cnt[Pattern::F3 as usize] >= 4 {
            Pattern::F2B
        } else if pat_cnt[Pattern::F3S as usize] + pat_cnt[Pattern::F3 as usize] >= 3 {
            Pattern::F2A
        } else if pat_cnt[Pattern::F3S as usize] + pat_cnt[Pattern::F3 as usize] > 0 {
            Pattern::F2
        } else if pat_cnt[Pattern::B3 as usize] + pat_cnt[Pattern::B3S as usize] > 0 {
            Pattern::B2
        } else if pat_cnt[Pattern::F2 as usize]
            + pat_cnt[Pattern::F2A as usize]
            + pat_cnt[Pattern::F2B as usize]
            > 0
        {
            Pattern::F1
        } else if pat_cnt[Pattern::B2 as usize] > 0 {
            Pattern::B1
        } else {
            Pattern::Dead
        }
    };

    memo[code] = pat as u8;
    pat
}

fn fill_pattern2x<const RENJU: bool, const N: usize>() -> Box<[Pattern2x; N]> {
    let mut table = vec![Pattern2x::default(); N].into_boxed_slice();
    if RENJU {
        let mut memo_b = vec![NIL_PAT; 177_147]; // 3^11
        let mut memo_w = vec![NIL_PAT; 177_147];
        for lo in 0..DENSE_HALF_RENJU {
            let lo_half = raw_half_from_dense(HALF_LEN_RENJU, false, lo) as u64;
            for hi in 0..DENSE_HALF_RENJU {
                let hi_half = raw_half_from_dense(HALF_LEN_RENJU, true, hi) as u64;
                let key = lo_half | (hi_half << (2 * HALF_LEN_RENJU));
                let idx = lo * DENSE_HALF_RENJU + hi;
                debug_assert_eq!(idx, dense_fused_index::<true>(key));
                let lb = decode_line::<11>(key, FE_BLACK);
                let lw = decode_line::<11>(key, FE_WHITE);
                let pb = get_pattern::<11, true>(&mut memo_b, &lb);
                let pw = get_pattern::<11, false>(&mut memo_w, &lw);
                table[idx] = Pattern2x::new(pb, pw);
            }
        }
    } else {
        let mut memo_b = vec![NIL_PAT; 19_683]; // 3^9
        let mut memo_w = vec![NIL_PAT; 19_683];
        for lo in 0..DENSE_HALF_FREESTYLE {
            let lo_half = raw_half_from_dense(HALF_LEN_FREESTYLE, false, lo) as u64;
            for hi in 0..DENSE_HALF_FREESTYLE {
                let hi_half = raw_half_from_dense(HALF_LEN_FREESTYLE, true, hi) as u64;
                let key = lo_half | (hi_half << (2 * HALF_LEN_FREESTYLE));
                let idx = lo * DENSE_HALF_FREESTYLE + hi;
                debug_assert_eq!(idx, dense_fused_index::<false>(key));
                let lb = decode_line::<9>(key, FE_BLACK);
                let lw = decode_line::<9>(key, FE_WHITE);
                let pb = get_pattern::<9, false>(&mut memo_b, &lb);
                let pw = get_pattern::<9, false>(&mut memo_w, &lw);
                table[idx] = Pattern2x::new(pb, pw);
            }
        }
    }
    table.try_into().ok().expect("exact length")
}

fn fill_defence<const RENJU: bool, const N: usize>(
    pattern2xs: &[Pattern2x; N],
) -> Box<[[u8; 2]; N]> {
    let h = if RENJU {
        HALF_LEN_RENJU
    } else {
        HALF_LEN_FREESTYLE
    };
    let half_cnt = if RENJU {
        DENSE_HALF_RENJU
    } else {
        DENSE_HALF_FREESTYLE
    };
    let mut table = vec![[0_u8; 2]; N].into_boxed_slice();
    for lo in 0..half_cnt {
        let lo_half = raw_half_from_dense(h, false, lo) as u64;
        for hi in 0..half_cnt {
            let hi_half = raw_half_from_dense(h, true, hi) as u64;
            let key = lo_half | (hi_half << (2 * h));
            let idx = lo * half_cnt + hi;
            for attacker in [FE_BLACK, FE_WHITE] {
                let mut mask = 0_u32;
                if pattern2xs[idx].for_side(attacker) >= Pattern::F3 {
                    let defender = attacker ^ 1;
                    let place_xor = (1_u64 + defender as u64) as u64;
                    for i in 0..(2 * h) {
                        let cell_mask = 0b11_u64 << (2 * i);
                        if (key & cell_mask) != cell_mask {
                            continue;
                        }
                        let probe_key = key ^ (place_xor << (2 * i));
                        if pattern2xs[dense_fused_index::<RENJU>(probe_key)].for_side(attacker)
                            < Pattern::F3
                        {
                            mask |= 1 << i;
                        }
                    }
                    mask = (mask >> (h - 4)) & 0xFF;
                }
                table[idx][attacker] = mask as u8;
            }
        }
    }
    table.try_into().ok().expect("exact length")
}

fn classify_pattern4<const FORBID: bool>(
    p1: Pattern,
    p2: Pattern,
    p3: Pattern,
    p4: Pattern,
) -> Pattern4 {
    let mut n = [0_u8; PATTERN_NB];
    n[p1 as usize] += 1;
    n[p2 as usize] += 1;
    n[p3 as usize] += 1;
    n[p4 as usize] += 1;

    n[Pattern::B4 as usize] += n[Pattern::B4S as usize];
    n[Pattern::B3 as usize] += n[Pattern::B3S as usize];

    if n[Pattern::F5 as usize] >= 1 {
        return Pattern4::AFive;
    }

    if FORBID {
        if n[Pattern::Ol as usize] >= 1 {
            return Pattern4::Forbid;
        }
        if n[Pattern::F4 as usize] + n[Pattern::B4 as usize] >= 2 {
            return Pattern4::Forbid;
        }
        if n[Pattern::F3 as usize] + n[Pattern::F3S as usize] >= 2 {
            return Pattern4::Forbid;
        }
    }

    let f3_all = n[Pattern::F3 as usize] + n[Pattern::F3S as usize];
    let f2_all = n[Pattern::F2 as usize] + n[Pattern::F2A as usize] + n[Pattern::F2B as usize];

    if n[Pattern::B4 as usize] >= 2 || n[Pattern::F4 as usize] >= 1 {
        return Pattern4::BFlex4;
    }
    if n[Pattern::B4 as usize] >= 1 {
        if f3_all >= 1 {
            return Pattern4::CBlock4Flex3;
        }
        if n[Pattern::B3 as usize] >= 1 || f2_all >= 1 {
            return Pattern4::DBlock4Plus;
        }
        return Pattern4::EBlock4;
    }
    if f3_all >= 1 {
        if f3_all >= 2 {
            return Pattern4::FFlex3x2;
        }
        if n[Pattern::B3 as usize] >= 1 || f2_all >= 1 {
            return Pattern4::GFlex3Plus;
        }
        return Pattern4::HFlex3;
    }
    if n[Pattern::B3 as usize] >= 1 && (n[Pattern::B3 as usize] >= 2 || f2_all >= 1) {
        return Pattern4::IBlock3Plus;
    }
    if f2_all >= 2 {
        return Pattern4::JFlex2x2;
    }
    if n[Pattern::B3 as usize] >= 1 {
        return Pattern4::KBlock3;
    }
    if f2_all >= 1 {
        return Pattern4::LFlex2;
    }
    Pattern4::None
}

fn fill_pcode_table<const FORBID: bool>() -> Box<[PCodeP4; 65_536]> {
    let mut table = vec![PCodeP4::default(); 65_536].into_boxed_slice();
    let mut code = 0_u16;
    for a in 0..PATTERN_NB {
        for b in a..PATTERN_NB {
            for c in b..PATTERN_NB {
                for d in c..PATTERN_NB {
                    let p4 = classify_pattern4::<FORBID>(
                        Pattern::from_u8(a as u8),
                        Pattern::from_u8(b as u8),
                        Pattern::from_u8(c as u8),
                        Pattern::from_u8(d as u8),
                    );
                    let idx = (a << 12) | (b << 8) | (c << 4) | d;
                    table[idx] = PCodeP4::new(code, p4);
                    code += 1;
                }
            }
        }
    }
    debug_assert_eq!(code as usize, PCODE_NB);

    for i in 0..PATTERN_NB {
        for j in 0..PATTERN_NB {
            for m in 0..PATTERN_NB {
                for n in 0..PATTERN_NB {
                    let mut v = [i, j, m, n];
                    if v[0] > v[1] {
                        v.swap(0, 1);
                    }
                    if v[2] > v[3] {
                        v.swap(2, 3);
                    }
                    if v[0] > v[2] {
                        v.swap(0, 2);
                    }
                    if v[1] > v[3] {
                        v.swap(1, 3);
                    }
                    if v[1] > v[2] {
                        v.swap(1, 2);
                    }
                    let src = (v[0] << 12) | (v[1] << 8) | (v[2] << 4) | v[3];
                    let dst = (i << 12) | (j << 8) | (m << 4) | n;
                    table[dst] = table[src];
                }
            }
        }
    }

    table.try_into().ok().expect("exact length")
}

impl PatternTables {
    fn build() -> Self {
        let pcode = fill_pcode_table::<false>();
        let pcode_renju_black = fill_pcode_table::<true>();
        let pattern2x_freestyle = fill_pattern2x::<false, DENSE_KEY_FREESTYLE>();
        let pattern2x_renju = fill_pattern2x::<true, DENSE_KEY_RENJU>();
        let defence_freestyle = fill_defence::<false, DENSE_KEY_FREESTYLE>(&pattern2x_freestyle);
        let defence_renju = fill_defence::<true, DENSE_KEY_RENJU>(&pattern2x_renju);
        Self {
            pattern2x_freestyle,
            pattern2x_renju,
            defence_freestyle,
            defence_renju,
            pcode,
            pcode_renju_black,
        }
    }
}
