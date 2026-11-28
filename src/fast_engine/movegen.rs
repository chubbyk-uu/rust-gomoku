//! Pattern4-driven move generators aligned with Rapfi `game/movegen.cpp`.
//!
//! Includes:
//! - `generate_all`: every empty candidate cell
//! - `generate_winning`: first `A_FIVE` or `B_FLEX4`
//! - `generate_defend_five`: forced defence of opponent `A_FIVE`
//! - `generate_defend_four<INCLUDE_LOSING>`: exact defence of opponent `B_FLEX4`
//! - `generate_defend_b4f3<RENJU>`: exact defence of opponent `C_BLOCK4_FLEX3`
//! - `generate_vcf<RENJU>`: all VCF four-threat moves (including Renju `Forbid` with `>= B4` for White/self)
//! - `generate_neighbors_vcf<COMB>`: `RANGE_SQUARE2_LINE4` neighbor VCF moves for QVCF
//! - `validate_opponent_c_move`: Renju fake-`C_BLOCK4_FLEX3` check

use crate::constants::{BOARD_AREA, BOARD_SIZE};
use crate::fast_engine::board::{step_pos, FastBoard, NONE_MOVE};
use crate::fast_engine::pattern::{lookup_defence, lookup_pattern, Pattern, Pattern4, FE_BLACK};
use crate::rules::RuleSet;

const MAX_FIND_DIST: isize = 4;

/// A candidate move paired with ordering score (4 bytes, 4-byte aligned).
#[derive(Clone, Copy, Debug, Default)]
pub struct ScoredMove {
    pub pos: u16,
    pub score: i16,
}

impl ScoredMove {
    #[inline(always)]
    pub const fn new(pos: u16) -> Self {
        Self { pos, score: 0 }
    }
}

/// `RANGE_SQUARE2_LINE4` `(dx, dy)` offsets used by QVCF neighbor generation
/// (`game/pos.h:355`, 40 cells around center).
const RANGE_SQUARE2_LINE4: [(isize, isize); 40] = {
    let mut arr = [(0_isize, 0_isize); 40];
    let mut idx = 0_usize;
    let mut dy = -4_isize;
    while dy <= 4 {
        let mut dx = -4_isize;
        while dx <= 4 {
            let ax = if dx < 0 { -dx } else { dx };
            let ay = if dy < 0 { -dy } else { dy };
            if !(ax == 0 && ay == 0) {
                let in_sq = ax <= 2 && ay <= 2;
                let on_line = dx == 0 || dy == 0 || dx == dy || dx == -dy;
                if in_sq || on_line {
                    arr[idx] = (dx, dy);
                    idx += 1;
                }
            }
            dx += 1;
        }
        dy += 1;
    }
    arr
};

#[inline(always)]
fn piece_is_side(bits: u8, side: usize) -> bool {
    bits == if side == FE_BLACK { 0b10 } else { 0b01 }
}

/// Locate the first empty candidate with `p4` for `side`.
pub fn find_first_p4_pos(board: &FastBoard, side: usize, p4: Pattern4) -> u16 {
    let mut cands = board.empty_candidates();
    for w_idx in 0..4 {
        let mut w = cands.words[w_idx];
        let base = w_idx << 6;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            w &= w - 1;
            let pos = base + tz;
            if board.pattern4(pos, side) == p4 {
                return pos as u16;
            }
        }
    }
    // Fallback over all empty cells (in case a board was constructed arbitrarily).
    let mut empties = *board.empty_cells();
    for w_idx in 0..4 {
        let mut w = empties.words[w_idx];
        let base = w_idx << 6;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            w &= w - 1;
            let pos = base + tz;
            if board.pattern4(pos, side) == p4 {
                return pos as u16;
            }
        }
    }
    let _ = (&mut cands, &mut empties);
    NONE_MOVE
}

/// Locate the most recent empty cell with `p4` in `[CBlock4Flex3..=AFive]` using
/// `StateInfo::last_p4_move` fast path, falling back to a candidate scan.
#[inline]
pub fn find_p4_pos(board: &FastBoard, side: usize, p4: Pattern4) -> u16 {
    let pos = board.last_p4_move(side, p4);
    if (pos as usize) < BOARD_AREA
        && board.is_empty(pos as usize)
        && board.pattern4(pos as usize, side) == p4
    {
        return pos;
    }
    find_first_p4_pos(board, side, p4)
}

/// Stack-allocated uninitialized move list (avoids 1,350B `memset` per node).
pub struct MoveList {
    data: [std::mem::MaybeUninit<ScoredMove>; BOARD_AREA],
    len: usize,
}

impl MoveList {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            data: [std::mem::MaybeUninit::uninit(); BOARD_AREA],
            len: 0,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline(always)]
    pub fn push(&mut self, mv: ScoredMove) {
        assert!(
            self.len < BOARD_AREA,
            "MoveList capacity ({BOARD_AREA}) exceeded"
        );
        self.data[self.len].write(mv);
        self.len += 1;
    }

    #[inline(always)]
    pub fn truncate(&mut self, new_len: usize) {
        if new_len < self.len {
            self.len = new_len;
        }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[ScoredMove] {
        // SAFETY: elements `0..self.len` are initialized via `push`.
        unsafe { std::slice::from_raw_parts(self.data.as_ptr() as *const ScoredMove, self.len) }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [ScoredMove] {
        // SAFETY: elements `0..self.len` are initialized via `push`.
        unsafe {
            std::slice::from_raw_parts_mut(self.data.as_mut_ptr() as *mut ScoredMove, self.len)
        }
    }
}

/// Generate every empty candidate move (`generate<ALL>`).
#[inline]
pub fn generate_all_list(board: &FastBoard, out: &mut MoveList) {
    let cands = board.empty_candidates();
    for w_idx in 0..4 {
        let mut w = cands.words[w_idx];
        let base = w_idx << 6;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            w &= w - 1;
            out.push(ScoredMove::new((base + tz) as u16));
        }
    }
}

/// Generate every empty candidate move (`generate<ALL>`).
#[inline]
pub fn generate_all(board: &FastBoard, out: &mut [ScoredMove], start: usize) -> usize {
    let mut len = start;
    let cands = board.empty_candidates();
    for w_idx in 0..4 {
        let mut w = cands.words[w_idx];
        let base = w_idx << 6;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            w &= w - 1;
            out[len] = ScoredMove::new((base + tz) as u16);
            len += 1;
        }
    }
    len
}

/// Generate first direct winning move (`generate<WINNING>`).
pub fn generate_winning(board: &FastBoard, out: &mut [ScoredMove], start: usize) -> usize {
    let self_side = board.side_to_move();
    if board.p4_count(self_side, Pattern4::AFive) > 0 {
        let pos = find_p4_pos(board, self_side, Pattern4::AFive);
        if pos != NONE_MOVE {
            out[start] = ScoredMove::new(pos);
            return start + 1;
        }
    } else if board.p4_count(self_side, Pattern4::BFlex4) > 0 {
        let pos = find_p4_pos(board, self_side, Pattern4::BFlex4);
        if pos != NONE_MOVE {
            out[start] = ScoredMove::new(pos);
            return start + 1;
        }
    }
    start
}

/// Generate the forced defence move against opponent `A_FIVE` (`generate<DEFEND_FIVE>`).
pub fn generate_defend_five(board: &FastBoard, out: &mut [ScoredMove], start: usize) -> usize {
    let oppo = board.side_to_move() ^ 1;
    let pos = find_p4_pos(board, oppo, Pattern4::AFive);
    if pos != NONE_MOVE {
        out[start] = ScoredMove::new(pos);
        return start + 1;
    }
    start
}

/// Generate all VCF moves for `self` (`generate<VCF>` / `generate<VCF | RULE_RENJU>`).
pub fn generate_vcf<const RENJU: bool>(
    board: &FastBoard,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let self_side = board.side_to_move();
    let mut len = start;
    let cands = board.empty_candidates();
    for w_idx in 0..4 {
        let mut w = cands.words[w_idx];
        let base = w_idx << 6;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            w &= w - 1;
            let pos = base + tz;
            let p4 = board.pattern4(pos, self_side);
            let ok = if RENJU {
                p4 >= Pattern4::EBlock4
                    || (p4 == Pattern4::Forbid
                        && (board.pattern(pos, self_side, 0) >= Pattern::B4
                            || board.pattern(pos, self_side, 1) >= Pattern::B4
                            || board.pattern(pos, self_side, 2) >= Pattern::B4
                            || board.pattern(pos, self_side, 3) >= Pattern::B4))
            } else {
                p4 >= Pattern4::EBlock4
            };
            if ok {
                out[len] = ScoredMove::new(pos as u16);
                len += 1;
            }
        }
    }
    len
}

/// Generate neighbor VCF moves around `center` in `RANGE_SQUARE2_LINE4`
/// (`generateNeighbors<VCF>` / `generateNeighbors<VCF | COMB>`).
pub fn generate_neighbors_vcf<const COMB: bool>(
    board: &FastBoard,
    out: &mut [ScoredMove],
    start: usize,
    center: u16,
) -> usize {
    if (center as usize) >= BOARD_AREA {
        return start;
    }
    let self_side = board.side_to_move();
    let has_any = if COMB {
        board.p4_count(self_side, Pattern4::DBlock4Plus)
            + board.p4_count(self_side, Pattern4::CBlock4Flex3)
            > 0
    } else {
        board.p4_count(self_side, Pattern4::EBlock4)
            + board.p4_count(self_side, Pattern4::DBlock4Plus)
            + board.p4_count(self_side, Pattern4::CBlock4Flex3)
            > 0
    };
    if !has_any {
        return start;
    }

    let cx = (center as usize % BOARD_SIZE) as isize;
    let cy = (center as usize / BOARD_SIZE) as isize;
    let min_p4 = if COMB {
        Pattern4::DBlock4Plus
    } else {
        Pattern4::EBlock4
    };

    let mut len = start;
    for &(dx, dy) in &RANGE_SQUARE2_LINE4 {
        let nx = cx + dx;
        let ny = cy + dy;
        if nx < 0 || nx >= BOARD_SIZE as isize || ny < 0 || ny >= BOARD_SIZE as isize {
            continue;
        }
        let pos = (ny as usize) * BOARD_SIZE + (nx as usize);
        if board.is_empty_candidate(pos) && board.pattern4(pos, self_side) >= min_p4 {
            out[len] = ScoredMove::new(pos as u16);
            len += 1;
        }
    }
    len
}

fn find_all_pseudo_four_defend_pos(
    board: &FastBoard,
    oppo: usize,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let mut len = start;
    let cands = board.empty_candidates();
    for w_idx in 0..4 {
        let mut w = cands.words[w_idx];
        let base = w_idx << 6;
        while w != 0 {
            let tz = w.trailing_zeros() as usize;
            w &= w - 1;
            let pos = base + tz;
            let p4 = board.pattern4(pos, oppo);
            if p4 >= Pattern4::EBlock4 {
                out[len] = ScoredMove::new(pos as u16);
                len += 1;
            } else if p4 == Pattern4::Forbid {
                for dir in 0..4 {
                    if board.pattern(pos, FE_BLACK, dir) >= Pattern::B4 {
                        out[len] = ScoredMove::new(pos as u16);
                        len += 1;
                        break;
                    }
                }
            }
        }
    }
    len
}

fn find_flex4_line_defence(
    board: &mut FastBoard,
    oppo: usize,
    f4_pos: usize,
    dir: usize,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let renju = board.rule() == RuleSet::Renju;
    out[start] = ScoredMove::new(f4_pos as u16);
    let mut slot1 = NONE_MOVE;
    let mut slot2 = NONE_MOVE;

    for step in 1..=MAX_FIND_DIST {
        let Some(pos) = step_pos(f4_pos, dir, -step) else {
            break;
        };
        let bits = board.cell_bits(pos);
        if piece_is_side(bits, oppo) {
            continue;
        } else if bits == 0b11 {
            slot1 = pos as u16;
            if board.pattern(pos, oppo, dir) == Pattern::F4
                && (board.pattern4(pos, oppo) != Pattern4::Forbid
                    || !renju
                    || !board.check_forbidden_point(pos))
            {
                out[start + 1] = ScoredMove::new(slot1);
                return start + 2;
            }
        }
        break;
    }

    for step in 1..=MAX_FIND_DIST {
        let Some(pos) = step_pos(f4_pos, dir, step) else {
            break;
        };
        let bits = board.cell_bits(pos);
        if piece_is_side(bits, oppo) {
            continue;
        } else if bits == 0b11 {
            if board.pattern(pos, oppo, dir) == Pattern::F4
                && (board.pattern4(pos, oppo) != Pattern4::Forbid
                    || !renju
                    || !board.check_forbidden_point(pos))
            {
                out[start + 1] = ScoredMove::new(pos as u16);
                return start + 2;
            } else {
                slot2 = pos as u16;
            }
        }
        break;
    }

    let mut len = start + 1;
    if slot1 != NONE_MOVE {
        out[len] = ScoredMove::new(slot1);
        len += 1;
    }
    if slot2 != NONE_MOVE {
        out[len] = ScoredMove::new(slot2);
        len += 1;
    }
    len
}

fn find_double_b3_defence(
    board: &FastBoard,
    oppo: usize,
    b4_pos: usize,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let mut len = start;
    out[len] = ScoredMove::new(b4_pos as u16);
    len += 1;

    for d in 0..4 {
        let mut back_steps = 0_isize;
        while back_steps < MAX_FIND_DIST {
            let Some(pos) = step_pos(b4_pos, d, -(back_steps + 1)) else {
                break;
            };
            let bits = board.cell_bits(pos);
            if piece_is_side(bits, oppo) {
                back_steps += 1;
                continue;
            } else if bits == 0b11 && board.pattern(pos, oppo, d) >= Pattern::B4 {
                out[len] = ScoredMove::new(pos as u16);
                len += 1;
            }
            break;
        }
        let rem = MAX_FIND_DIST - back_steps;
        for step in 1..=rem {
            let Some(pos) = step_pos(b4_pos, d, step) else {
                break;
            };
            let bits = board.cell_bits(pos);
            if piece_is_side(bits, oppo) {
                continue;
            } else if bits == 0b11 && board.pattern(pos, oppo, d) >= Pattern::B4 {
                out[len] = ScoredMove::new(pos as u16);
                len += 1;
            }
            break;
        }
    }
    len
}

fn find_flex4_completion_square(
    board: &FastBoard,
    oppo: usize,
    attack_pos: usize,
    dir: usize,
) -> Option<usize> {
    for sign in [-1_isize, 1_isize] {
        for step in 1..=MAX_FIND_DIST {
            let Some(pos) = step_pos(attack_pos, dir, sign * step) else {
                break;
            };
            let bits = board.cell_bits(pos);
            if piece_is_side(bits, oppo) {
                continue;
            } else if bits == 0b11 {
                if board.pattern(pos, oppo, dir) == Pattern::F4
                    && board.pattern4(pos, oppo) == Pattern4::BFlex4
                {
                    return Some(pos);
                }
                continue;
            }
            break;
        }
    }
    None
}

fn find_four_defence<const INCLUDE_LOSING: bool>(
    board: &mut FastBoard,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let oppo = board.side_to_move() ^ 1;
    let mut last = start;

    let last_attack = board.last_flex4_attack(oppo);
    if (last_attack as usize) < BOARD_AREA {
        let atk = last_attack as usize;
        // 1. F3 / F3S lines
        for dir in 0..4 {
            let pat = board.pattern(atk, oppo, dir);
            if pat != Pattern::F3 && pat != Pattern::F3S {
                continue;
            }
            let Some(f4_pos) = find_flex4_completion_square(board, oppo, atk, dir) else {
                continue;
            };
            if !INCLUDE_LOSING && last > start {
                return start;
            }
            last = find_flex4_line_defence(board, oppo, f4_pos, dir, out, last);
        }
        if last > start {
            return last;
        }

        // 2. Double B3 lines
        for dir in 0..4 {
            let pat = board.pattern(atk, oppo, dir);
            if pat != Pattern::B3 && pat != Pattern::B3S {
                continue;
            }
            let mut back_steps = 0_isize;
            let mut empty_seen = 0_u8;
            while back_steps < MAX_FIND_DIST {
                let Some(pos) = step_pos(atk, dir, -(back_steps + 1)) else {
                    break;
                };
                let bits = board.cell_bits(pos);
                if piece_is_side(bits, oppo) {
                    back_steps += 1;
                    continue;
                } else if bits == 0b11 {
                    if board.pattern4(pos, oppo) >= Pattern4::BFlex4 {
                        let p = board.pattern(pos, oppo, dir);
                        if p == Pattern::F4 {
                            return find_flex4_line_defence(board, oppo, pos, dir, out, last);
                        } else if p == Pattern::B4 || p == Pattern::B4S {
                            return find_double_b3_defence(board, oppo, pos, out, last);
                        }
                    }
                    empty_seen += 1;
                    if empty_seen >= 2 {
                        break;
                    }
                    back_steps += 1;
                    continue;
                }
                break;
            }
            let rem = MAX_FIND_DIST - back_steps;
            for step in 1..=rem {
                let Some(pos) = step_pos(atk, dir, step) else {
                    break;
                };
                let bits = board.cell_bits(pos);
                if piece_is_side(bits, oppo) {
                    continue;
                } else if bits == 0b11 {
                    if board.pattern4(pos, oppo) >= Pattern4::BFlex4 {
                        let p = board.pattern(pos, oppo, dir);
                        if p == Pattern::F4 {
                            return find_flex4_line_defence(board, oppo, pos, dir, out, last);
                        } else if p == Pattern::B4 || p == Pattern::B4S {
                            return find_double_b3_defence(board, oppo, pos, out, last);
                        }
                    }
                    continue;
                }
                break;
            }
        }
    }

    find_all_pseudo_four_defend_pos(board, oppo, out, last)
}

/// Generate deduplicated non-VCF defence moves against opponent `B_FLEX4`.
pub fn generate_defend_four<const INCLUDE_LOSING: bool>(
    board: &mut FastBoard,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let mut end = find_four_defence::<INCLUDE_LOSING>(board, out, start);
    if end == start {
        return start;
    }
    let slice = &mut out[start..end];
    slice.sort_unstable_by_key(|m| m.pos);
    let self_side = board.side_to_move();
    let mut write = start;
    let mut prev = NONE_MOVE;
    for read in start..end {
        let mv = out[read];
        if mv.pos == prev {
            continue;
        }
        prev = mv.pos;
        if board.pattern4(mv.pos as usize, self_side) >= Pattern4::EBlock4 {
            continue;
        }
        out[write] = mv;
        write += 1;
    }
    end = write;
    end
}

fn find_f3_line_defence<const RENJU: bool>(
    board: &mut FastBoard,
    oppo: usize,
    f3_pos: usize,
    dir: usize,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let key = board.get_key_at::<RENJU>(f3_pos, dir);
    let defence_mask = lookup_defence::<RENJU>(board.tables(), key, oppo);
    let check_renju = RENJU && oppo == FE_BLACK;

    let mut run = |b: &mut FastBoard| -> usize {
        let mut len = start;
        let mut left_renju_def = NONE_MOVE;
        let mut right_renju_def = NONE_MOVE;
        let mut prev_found = false;
        let mut found_left_forbid = false;
        let mut found_right_forbid = false;

        for i in 0..4_isize {
            let Some(pos) = step_pos(f3_pos, dir, -(i + 1)) else {
                break;
            };
            if ((defence_mask >> (3 - i)) & 1) != 0 {
                if b.is_empty(pos) {
                    out[len] = ScoredMove::new(pos as u16);
                    len += 1;
                    prev_found = true;
                    if check_renju && !found_left_forbid {
                        found_left_forbid = b.check_forbidden_point(pos);
                    }
                }
            } else if check_renju && prev_found && b.is_empty(pos) {
                left_renju_def = pos as u16;
                prev_found = false;
            }
        }

        prev_found = false;
        for i in 0..4_isize {
            let Some(pos) = step_pos(f3_pos, dir, i + 1) else {
                break;
            };
            if ((defence_mask >> (4 + i)) & 1) != 0 {
                if b.is_empty(pos) {
                    out[len] = ScoredMove::new(pos as u16);
                    len += 1;
                    prev_found = true;
                    if check_renju && !found_right_forbid {
                        found_right_forbid = b.check_forbidden_point(pos);
                    }
                }
            } else if check_renju && prev_found && b.is_empty(pos) {
                right_renju_def = pos as u16;
                prev_found = false;
            }
        }

        if check_renju {
            if found_left_forbid && right_renju_def != NONE_MOVE {
                out[len] = ScoredMove::new(right_renju_def);
                len += 1;
            }
            if found_right_forbid && left_renju_def != NONE_MOVE {
                out[len] = ScoredMove::new(left_renju_def);
                len += 1;
            }
        }
        len
    };

    if check_renju {
        board.with_probe_move::<RENJU, _>(oppo, f3_pos, run)
    } else {
        run(board)
    }
}

fn find_b4_in_line<const RENJU: bool>(
    board: &mut FastBoard,
    oppo: usize,
    b4_pos: usize,
    dir: usize,
) -> Option<usize> {
    let check_renju_f4 = |b: &FastBoard, pos: usize| -> bool {
        let key = b.get_key_at::<false>(pos, dir);
        lookup_pattern::<false>(b.tables(), key).for_side(FE_BLACK) >= Pattern::B4
    };
    let check_not_overline_b4 = |b: &mut FastBoard, pos: usize| -> bool {
        b.with_probe_move::<RENJU, _>(oppo, pos, |b2| b2.pattern(b4_pos, oppo, dir) == Pattern::F5)
    };

    let mut back_steps = 0_isize;
    while back_steps < MAX_FIND_DIST {
        let Some(pos) = step_pos(b4_pos, dir, -(back_steps + 1)) else {
            break;
        };
        let bits = board.cell_bits(pos);
        if piece_is_side(bits, oppo) {
            back_steps += 1;
            continue;
        } else if bits == 0b11 {
            let pat = board.pattern(pos, oppo, dir);
            if pat == Pattern::B4
                || pat == Pattern::B4S
                || (RENJU
                    && board.pattern4(pos, oppo) == Pattern4::Forbid
                    && check_renju_f4(board, pos))
            {
                if !RENJU || check_not_overline_b4(board, pos) {
                    return Some(pos);
                }
            }
        }
        break;
    }

    let rem = MAX_FIND_DIST - back_steps;
    for step in 1..=rem {
        let Some(pos) = step_pos(b4_pos, dir, step) else {
            break;
        };
        let bits = board.cell_bits(pos);
        if piece_is_side(bits, oppo) {
            continue;
        } else if bits == 0b11 {
            let pat = board.pattern(pos, oppo, dir);
            if pat == Pattern::B4
                || pat == Pattern::B4S
                || (RENJU
                    && board.pattern4(pos, oppo) == Pattern4::Forbid
                    && check_renju_f4(board, pos))
            {
                if !RENJU || check_not_overline_b4(board, pos) {
                    return Some(pos);
                }
            }
        }
        break;
    }
    None
}

fn find_all_b3_counter_defence<const RENJU: bool>(
    board: &FastBoard,
    oppo: usize,
    b4_pos: usize,
    dir: usize,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let self_side = oppo ^ 1;
    let is_pseudo_forbid_b4 =
        RENJU && self_side == FE_BLACK && board.pattern4(b4_pos, self_side) == Pattern4::Forbid;
    let mut len = start;

    for sign in [-1_isize, 1_isize] {
        for step in 1..=MAX_FIND_DIST {
            let Some(pos) = step_pos(b4_pos, dir, sign * step) else {
                break;
            };
            let bits = board.cell_bits(pos);
            if piece_is_side(bits, self_side) {
                continue;
            } else if bits == 0b11
                && (is_pseudo_forbid_b4 || board.pattern(pos, self_side, dir) >= Pattern::B3)
            {
                out[len] = ScoredMove::new(pos as u16);
                len += 1;
                continue;
            }
            break;
        }
    }
    len
}

fn find_b4f3_defence<const RENJU: bool>(
    board: &mut FastBoard,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let oppo = board.side_to_move() ^ 1;
    let b4f3_u16 = find_p4_pos(board, oppo, Pattern4::CBlock4Flex3);
    if (b4f3_u16 as usize) >= BOARD_AREA {
        return start;
    }
    let b4f3_pos = b4f3_u16 as usize;
    let mut last = start;
    out[last] = ScoredMove::new(b4f3_u16);
    last += 1;

    for dir in 0..4 {
        let pat = board.pattern(b4f3_pos, oppo, dir);
        if pat == Pattern::F3 || pat == Pattern::F3S {
            last = find_f3_line_defence::<RENJU>(board, oppo, b4f3_pos, dir, out, last);
        } else if pat == Pattern::B4 || pat == Pattern::B4S {
            let Some(b4_pos) = find_b4_in_line::<RENJU>(board, oppo, b4f3_pos, dir) else {
                // Defensive fallback: if B4 square cannot be uniquely found, fall back to full move list.
                return start;
            };
            if board.pattern4(b4_pos, oppo ^ 1) >= Pattern4::EBlock4 {
                return start;
            }
            out[last] = ScoredMove::new(b4_pos as u16);
            last += 1;
            for d in 0..4 {
                last = find_all_b3_counter_defence::<RENJU>(board, oppo, b4_pos, d, out, last);
            }
        }
    }
    last
}

/// Generate deduplicated non-VCF defence moves against opponent `C_BLOCK4_FLEX3`.
/// Returns `start` (empty) when no direct defence is needed because we have a B4 counter.
pub fn generate_defend_b4f3<const RENJU: bool>(
    board: &mut FastBoard,
    out: &mut [ScoredMove],
    start: usize,
) -> usize {
    let end = find_b4f3_defence::<RENJU>(board, out, start);
    if end == start {
        return start;
    }
    let slice = &mut out[start..end];
    slice.sort_unstable_by_key(|m| m.pos);
    let self_side = board.side_to_move();
    let mut write = start;
    let mut prev = NONE_MOVE;
    for read in start..end {
        let mv = out[read];
        if mv.pos == prev {
            continue;
        }
        prev = mv.pos;
        if board.pattern4(mv.pos as usize, self_side) >= Pattern4::EBlock4 {
            continue;
        }
        out[write] = mv;
        write += 1;
    }
    write
}

/// Exact Renju `validateOpponentCMove` (`movegen.cpp:784`):
/// returns `true` if opponent's `C_BLOCK4_FLEX3` is a genuine threat (for Black in Renju,
/// probes placing Black there and checks if `p4_count(BLACK, B_FLEX4) > 0`).
pub fn validate_opponent_c_move(board: &mut FastBoard) -> bool {
    if board.side_to_move() == FE_BLACK {
        return true;
    }
    let pos = find_p4_pos(board, FE_BLACK, Pattern4::CBlock4Flex3);
    if (pos as usize) >= BOARD_AREA {
        return false;
    }
    board.with_probe_move::<true, _>(FE_BLACK, pos as usize, |b| {
        b.p4_count(FE_BLACK, Pattern4::BFlex4) > 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_engine::board::CandRange;

    #[test]
    fn movegen_defend_four_and_b4f3_exact_defences() {
        // 1. Black creates a straight open three at (5,7),(6,7),(7,7); White to defend B_FLEX4.
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(5, 7), (0, 0), (6, 7), (0, 2), (7, 7)] {
            fb.play(y * BOARD_SIZE + x);
        }
        assert!(fb.p4_count(FE_BLACK, Pattern4::BFlex4) > 0);
        let mut buf = [ScoredMove::default(); BOARD_AREA];
        let n = generate_defend_four::<false>(&mut fb, &mut buf, 0);
        let coords: Vec<(usize, usize)> = buf[..n]
            .iter()
            .map(|m| (m.pos as usize % BOARD_SIZE, m.pos as usize / BOARD_SIZE))
            .collect();
        // Defences of _OOO_ are (4,7) and (8,7)!
        assert!(coords.contains(&(4, 7)), "missing (4,7) in {coords:?}");
        assert!(coords.contains(&(8, 7)), "missing (8,7) in {coords:?}");
        assert_eq!(coords.len(), 2, "expected exact 2 defences, got {coords:?}");

        // 2. Broken open three: Black at (5,7),(6,7),(8,7); White to defend B_FLEX4.
        let mut fb2 = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(5, 7), (0, 0), (6, 7), (0, 2), (8, 7)] {
            fb2.play(y * BOARD_SIZE + x);
        }
        assert!(fb2.p4_count(FE_BLACK, Pattern4::BFlex4) > 0);
        let n2 = generate_defend_four::<false>(&mut fb2, &mut buf, 0);
        let coords2: Vec<(usize, usize)> = buf[..n2]
            .iter()
            .map(|m| (m.pos as usize % BOARD_SIZE, m.pos as usize / BOARD_SIZE))
            .collect();
        // Defences of _OO_O_ are (4,7), (7,7), and (9,7)!
        assert!(coords2.contains(&(4, 7)), "missing (4,7) in {coords2:?}");
        assert!(coords2.contains(&(7, 7)), "missing (7,7) in {coords2:?}");
        assert!(coords2.contains(&(9, 7)), "missing (9,7) in {coords2:?}");
        assert_eq!(
            coords2.len(),
            3,
            "expected exact 3 defences, got {coords2:?}"
        );

        // 3. C_BLOCK4_FLEX3 threat: Black has (5,7),(6,7) and (7,5),(7,6),(7,9) -> (7,7) is B4 + F3!
        // Wait: vertical (7,5),(7,6), _, (7,8)? Let's put (7,4),(7,5),(7,6) with White blocker at (7,3)
        // so (7,7) makes vertical B4 (completing at (7,8)) + horizontal F3S ((5,7),(6,7),(7,7)).
        let mut fb3 = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[
            (5, 7),
            (7, 3),
            (6, 7),
            (0, 0),
            (7, 4),
            (0, 2),
            (7, 5),
            (0, 4),
            (7, 6),
        ] {
            fb3.play(y * BOARD_SIZE + x);
        }
        assert!(fb3.p4_count(FE_BLACK, Pattern4::CBlock4Flex3) > 0);
        assert!(validate_opponent_c_move(&mut fb3));
        let n3 = generate_defend_b4f3::<true>(&mut fb3, &mut buf, 0);
        let coords3: Vec<(usize, usize)> = buf[..n3]
            .iter()
            .map(|m| (m.pos as usize % BOARD_SIZE, m.pos as usize / BOARD_SIZE))
            .collect();
        assert!(
            coords3.contains(&(7, 7)),
            "missing B4F3 sq (7,7) in {coords3:?}"
        );
        assert!(
            coords3.contains(&(7, 8)),
            "missing B4 sq (7,8) in {coords3:?}"
        );
        assert!(
            coords3.contains(&(4, 7)),
            "missing F3 def (4,7) in {coords3:?}"
        );
        assert!(
            coords3.contains(&(8, 7)),
            "missing F3 def (8,7) in {coords3:?}"
        );
    }

    #[test]
    #[should_panic(expected = "MoveList capacity")]
    fn move_list_push_panics_cleanly_at_capacity() {
        let mut list = MoveList::new();
        for i in 0..=BOARD_AREA {
            list.push(ScoredMove::new(i as u16));
        }
    }
}
