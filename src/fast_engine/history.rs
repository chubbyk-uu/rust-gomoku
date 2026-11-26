//! Move-ordering history tables aligned with Rapfi `search/history.h` and
//! `search/ab/history.{h,cpp}`.

use crate::constants::BOARD_AREA;
use crate::fast_engine::board::{FastBoard, NONE_MOVE};
use crate::fast_engine::pattern::Pattern4;
use crate::rules::RuleSet;

pub const HIST_ATTACK: usize = 0;
pub const HIST_QUIET: usize = 1;
const MAIN_HIST_RANGE: i32 = 10_692;
pub const WEIGHT_SCALE: i32 = 256 * 256;

/// Rule index matching Rapfi `enum Rule { FREESTYLE=0, STANDARD=1, RENJU=2 }`.
#[inline(always)]
pub const fn rule_idx(rule: RuleSet) -> usize {
    match rule {
        RuleSet::Freestyle => 0,
        RuleSet::Renju => 2,
    }
}

#[inline(always)]
pub const fn history_table_index(rule: RuleSet, side: usize) -> usize {
    match rule {
        RuleSet::Freestyle => 0,
        RuleSet::Renju => 2 + side,
    }
}

/// Graded history bonus (`statBonus` in Rapfi `search/ab/history.cpp:31`).
#[inline(always)]
pub fn stat_bonus(rule: RuleSet, depth: f32) -> i32 {
    const QUAD: [i32; 3] = [29, 24, 23];
    let q = QUAD[rule_idx(rule)] as f32;
    let raw = (q * depth * depth + 99.0 * depth - 164.0) as i32;
    raw.clamp(-MAIN_HIST_RANGE, 8927)
}

#[inline(always)]
fn update_graded(entry: &mut i16, bonus: i32) {
    let b = bonus.clamp(-MAIN_HIST_RANGE, MAIN_HIST_RANGE);
    let v = *entry as i32;
    let next = v + b - (v * b.abs()) / MAIN_HIST_RANGE;
    *entry = next.clamp(-MAIN_HIST_RANGE, MAIN_HIST_RANGE) as i16;
}

/// `[side][pos][HIST_ATTACK | HIST_QUIET]`
#[derive(Clone)]
pub struct MainHistory {
    pub table: [[[i16; 2]; BOARD_AREA]; 2],
}

impl Default for MainHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl MainHistory {
    pub fn new() -> Self {
        Self {
            table: [[[0; 2]; BOARD_AREA]; 2],
        }
    }

    pub fn clear(&mut self) {
        self.table = [[[0; 2]; BOARD_AREA]; 2];
    }

    #[inline(always)]
    pub fn get(&self, side: usize, pos: usize, kind: usize) -> i16 {
        self.table[side][pos][kind]
    }

    #[inline(always)]
    pub fn update(&mut self, side: usize, pos: usize, kind: usize, bonus: i32) {
        update_graded(&mut self.table[side][pos][kind], bonus);
    }

    /// `statScore` in Rapfi `search/ab/parameter.h:290`.
    #[inline(always)]
    pub fn stat_score(&self, side: usize, pos: usize) -> i32 {
        let atk = self.table[side][pos][HIST_ATTACK] as i32;
        let qui = self.table[side][pos][HIST_QUIET] as i32;
        atk + (qui * 778) / 1024 - 3323
    }
}

/// `[oppo_side][last_move_pos] -> (counter_move_pos, required_self_p4)`
#[derive(Clone)]
pub struct CounterMoveHistory {
    pub table: [[(u16, Pattern4); BOARD_AREA]; 2],
}

impl Default for CounterMoveHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl CounterMoveHistory {
    pub fn new() -> Self {
        Self {
            table: [[(NONE_MOVE, Pattern4::None); BOARD_AREA]; 2],
        }
    }

    pub fn clear(&mut self) {
        self.table = [[(NONE_MOVE, Pattern4::None); BOARD_AREA]; 2];
    }

    #[inline(always)]
    pub fn get(&self, oppo_side: usize, last_pos: usize) -> (u16, Pattern4) {
        self.table[oppo_side][last_pos]
    }

    #[inline(always)]
    pub fn set(&mut self, oppo_side: usize, last_pos: usize, mv: u16, p4: Pattern4) {
        self.table[oppo_side][last_pos] = (mv, p4);
    }
}

#[derive(Clone, Copy)]
pub struct MoveHistoryWeights {
    pub attack_weight: i32,
    pub quiet_weight: i32,
    pub counter_move_bonus: i32,
}

#[inline(always)]
pub const fn move_history_weights(rule: RuleSet, side: usize) -> MoveHistoryWeights {
    const ATTACK_W: [i32; 4] = [492, 531, 511, 520];
    const QUIET_W: [i32; 4] = [265, 270, 270, 268];
    const CM_BONUS: [i32; 4] = [38, 33, 31, 33];
    let idx = history_table_index(rule, side);
    MoveHistoryWeights {
        attack_weight: ATTACK_W[idx],
        quiet_weight: QUIET_W[idx],
        counter_move_bonus: CM_BONUS[idx],
    }
}

const MAX_ATTACKS: usize = 48;
const MAX_QUIETS: usize = 192;

/// Per-node searched move tracker (`HistoryTracker` in Rapfi `search/ab/history.cpp`).
/// Uses uninitialized stack buffers to avoid 480B `memset` on every search node.
pub struct HistoryTracker {
    attack_count: usize,
    quiet_count: usize,
    attacks: [std::mem::MaybeUninit<u16>; MAX_ATTACKS],
    quiets: [std::mem::MaybeUninit<u16>; MAX_QUIETS],
}

impl HistoryTracker {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            attack_count: 0,
            quiet_count: 0,
            attacks: [std::mem::MaybeUninit::uninit(); MAX_ATTACKS],
            quiets: [std::mem::MaybeUninit::uninit(); MAX_QUIETS],
        }
    }

    #[inline(always)]
    pub fn add_searched_move(
        &mut self,
        board: &FastBoard,
        mv: u16,
        current_best: u16,
        self_p4: Pattern4,
    ) {
        if mv == current_best {
            return;
        }
        let oppo = board.side_to_move() ^ 1;
        let oppo4 =
            board.p4_count(oppo, Pattern4::AFive) > 0 || board.p4_count(oppo, Pattern4::BFlex4) > 0;
        if self_p4 >= Pattern4::HFlex3 && self.attack_count < MAX_ATTACKS {
            self.attacks[self.attack_count].write(mv);
            self.attack_count += 1;
        } else if !oppo4 && self_p4 < Pattern4::HFlex3 && self.quiet_count < MAX_QUIETS {
            self.quiets[self.quiet_count].write(mv);
            self.quiet_count += 1;
        }
    }

    pub fn update_bestmove_stats(
        &self,
        board: &FastBoard,
        main_hist: &mut MainHistory,
        cm_hist: &mut CounterMoveHistory,
        killers: &mut [u16; 2],
        depth: f32,
        best_move: u16,
    ) {
        let self_side = board.side_to_move();
        let oppo = self_side ^ 1;
        let oppo5 = board.p4_count(oppo, Pattern4::AFive) > 0;
        let oppo4 = oppo5 || board.p4_count(oppo, Pattern4::BFlex4) > 0;
        let self_p4 = board.pattern4(best_move as usize, self_side);
        let bonus = stat_bonus(board.rule(), depth);

        if self_p4 >= Pattern4::HFlex3 {
            main_hist.update(self_side, best_move as usize, HIST_ATTACK, bonus);
        } else if !oppo4 {
            main_hist.update(self_side, best_move as usize, HIST_QUIET, bonus);
            set_killer(killers, best_move);
            for i in 0..self.quiet_count {
                // SAFETY: slots `0..self.quiet_count` were initialized in `add_searched_move`.
                let q = unsafe { self.quiets[i].assume_init() };
                main_hist.update(self_side, q as usize, HIST_QUIET, -bonus);
            }
        }

        for i in 0..self.attack_count {
            // SAFETY: slots `0..self.attack_count` were initialized in `add_searched_move`.
            let a = unsafe { self.attacks[i].assume_init() };
            main_hist.update(self_side, a as usize, HIST_ATTACK, -bonus);
        }

        let last_mv = board.last_move();
        if !oppo5 && (last_mv as usize) < BOARD_AREA {
            cm_hist.set(oppo, last_mv as usize, best_move, self_p4);
        }
    }

    pub fn update_tt_move_stats(
        board: &mut FastBoard,
        main_hist: &mut MainHistory,
        killers: &mut [u16; 2],
        depth: f32,
        tt_move: u16,
        tt_value: i32,
        beta: i32,
    ) {
        if !board.is_legal_non_pass(tt_move) {
            return;
        }
        let self_side = board.side_to_move();
        let oppo = self_side ^ 1;
        let oppo4 =
            board.p4_count(oppo, Pattern4::AFive) > 0 || board.p4_count(oppo, Pattern4::BFlex4) > 0;
        let self_p4 = board.pattern4(tt_move as usize, self_side);
        if !oppo4 && self_p4 < Pattern4::HFlex3 {
            let bonus = stat_bonus(board.rule(), depth);
            if tt_value >= beta {
                main_hist.update(self_side, tt_move as usize, HIST_QUIET, bonus);
                set_killer(killers, tt_move);
            } else {
                main_hist.update(self_side, tt_move as usize, HIST_QUIET, -bonus);
            }
        }
    }
}

#[inline(always)]
pub fn set_killer(killers: &mut [u16; 2], mv: u16) {
    if killers[0] != mv {
        killers[1] = killers[0];
        killers[0] = mv;
    }
}
