//! 5-stage `MovePicker` aligned with Rapfi `search/movepick.{h,cpp}`.
//!
//! Stages:
//! - `MAIN_TT -> MAIN_MOVES -> ALLMOVES`
//! - `DEFENDFIVE_TT -> DEFENDFIVE_MOVES -> ALLMOVES`
//! - `DEFENDFOUR_TT -> DEFENDFOUR_MOVES -> ALLMOVES`
//! - `DEFENDB4F3_TT -> DEFENDB4F3_MOVES -> ALLMOVES`
//! - `QVCF_TT -> QVCF_MOVES -> ALLMOVES`
//!
//! Scoring combines `Mix9svqEvaluator::evaluate_policy_moves` (`ScoreScale = 32`)
//! with `MainHistory` (`HIST_ATTACK` / `HIST_QUIET`) and `CounterMoveHistory`,
//! using fast `Pattern4`-based classical scoring for `QVCF`.

use crate::constants::{BLACK, BOARD_AREA, WHITE};
use crate::fast_engine::board::{FastBoard, NONE_MOVE, PASS_MOVE};
use crate::fast_engine::history::{
    move_history_weights, CounterMoveHistory, MainHistory, HIST_ATTACK, HIST_QUIET, WEIGHT_SCALE,
};
use crate::fast_engine::movegen::{
    generate_all, generate_all_list, generate_defend_b4f3, generate_defend_five,
    generate_defend_four, generate_neighbors_vcf, generate_vcf, generate_winning,
    validate_opponent_c_move, MoveList, ScoredMove,
};
use crate::fast_engine::pattern::{Pattern4, FE_BLACK, FE_WHITE};
use crate::fast_engine::tt::DEPTH_QVCF_FULL;
use crate::nnue::Mix9svqEvaluator;
use crate::rules::RuleSet;

/// Rapfi `PolicyBuffer::ScoreScale = 32`.
pub const POLICY_SCORE_SCALE: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Stage {
    MainTt = 0,
    MainMoves = 1,
    DefendFiveTt = 2,
    DefendFiveMoves = 3,
    DefendFourTt = 4,
    DefendFourMoves = 5,
    DefendB4F3Tt = 6,
    DefendB4F3Moves = 7,
    QvcfTt = 8,
    QvcfMoves = 9,
    AllMoves = 10,
}

#[inline(always)]
fn clamp_move_score(score: i32) -> i16 {
    score.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Fast Pattern4-based classical move score used in QVCF (and if no NNUE evaluator).
#[inline(always)]
fn classical_move_score(board: &FastBoard, pos: usize, self_side: usize) -> i16 {
    const SELF_WEIGHT: [i32; 14] = [
        0, 0, 18, 22, 55, 65, 130, 175, 290, 210, 275, 420, 700, 1200,
    ];
    const OPPO_WEIGHT: [i32; 14] = [
        0, 0, 14, 18, 45, 55, 110, 150, 240, 180, 230, 360, 600, 1000,
    ];
    let oppo = self_side ^ 1;
    let s = board.pattern4(pos, self_side) as usize;
    let o = board.pattern4(pos, oppo) as usize;
    clamp_move_score(SELF_WEIGHT[s] + OPPO_WEIGHT[o])
}

/// Rapfi-aligned `fastPartialSort` (`movepick.cpp:72`), descending by `(score, Reverse(pos))`.
fn fast_partial_sort(slice: &mut [ScoredMove]) {
    let n = slice.len();
    if n <= 1 {
        return;
    }
    const INSERTION_LIMIT: usize = 56;
    const PARTIAL_LIMIT: usize = 150;
    let cmp =
        |a: &ScoredMove, b: &ScoredMove| b.score.cmp(&a.score).then_with(|| a.pos.cmp(&b.pos));

    if n <= INSERTION_LIMIT {
        // Insertion sort for small lists (Rapfi's `limit = 0` partial insertion sort
        // keeps moves with `score >= 0` sorted at the front, leaving `< 0` unsorted).
        let mut sorted_end = 0_usize;
        for p in 1..n {
            if slice[p].score >= 0 {
                let tmp = slice[p];
                sorted_end += 1;
                slice[p] = slice[sorted_end];
                let mut q = sorted_end;
                while q > 0 && cmp(&tmp, &slice[q - 1]).is_lt() {
                    slice[q] = slice[q - 1];
                    q -= 1;
                }
                slice[q] = tmp;
            }
        }
        // Also ensure slice[0] is ordered properly if slice[0].score < 0 and something >= 0 arrived.
        // Wait: the loop above inserts into `0..=sorted_end` comparing against `slice[q-1]`,
        // which naturally pushes a negative `slice[0]` down past every non-negative move!
    } else if n <= PARTIAL_LIMIT {
        slice.sort_unstable_by(cmp);
    } else {
        slice.select_nth_unstable_by(PARTIAL_LIMIT - 1, cmp);
        slice[..PARTIAL_LIMIT].sort_unstable_by(cmp);
    }
}

pub struct MovePicker {
    stage: Stage,
    tt_move: u16,
    allow_plain_b4_in_vcf: bool,
    has_policy: bool,
    cur_score: i16,
    cur_idx: usize,
    moves: MoveList,
}

impl MovePicker {
    /// Construct root legal move list (matching `MovePicker::MovePicker<ROOT>`).
    pub fn generate_root_moves(
        board: &mut FastBoard,
        evaluator: &mut Mix9svqEvaluator,
    ) -> Vec<u16> {
        let renju = board.rule() == RuleSet::Renju;
        let self_side = board.side_to_move();
        let oppo = self_side ^ 1;
        let mut buf = [ScoredMove::default(); BOARD_AREA];

        let end = if board.p4_count(self_side, Pattern4::AFive) > 0 {
            generate_winning(board, &mut buf, 0)
        } else if board.p4_count(oppo, Pattern4::AFive) > 0 {
            generate_defend_five(board, &mut buf, 0)
        } else if board.p4_count(self_side, Pattern4::BFlex4) > 0 {
            generate_winning(board, &mut buf, 0)
        } else if board.p4_count(oppo, Pattern4::BFlex4) > 0 {
            let e = generate_defend_four::<true>(board, &mut buf, 0);
            if renju {
                generate_vcf::<true>(board, &mut buf, e)
            } else {
                generate_vcf::<false>(board, &mut buf, e)
            }
        } else if board.p4_count(oppo, Pattern4::CBlock4Flex3) > 0
            && (!renju || validate_opponent_c_move(board))
        {
            let e = if renju {
                generate_defend_b4f3::<true>(board, &mut buf, 0)
            } else {
                generate_defend_b4f3::<false>(board, &mut buf, 0)
            };
            if e == 0 {
                generate_all(board, &mut buf, 0)
            } else if renju {
                generate_vcf::<true>(board, &mut buf, e)
            } else {
                generate_vcf::<false>(board, &mut buf, e)
            }
        } else {
            generate_all(board, &mut buf, 0)
        };

        // Score root moves with NNUE policy and filter Renju forbidden moves.
        let mut raw_moves = [0_u16; BOARD_AREA];
        let mut raw_scores = [0.0_f32; BOARD_AREA];
        let check_forbid = renju && self_side == FE_BLACK;
        let mut legal_len = 0_usize;
        for i in 0..end {
            let p = buf[i].pos;
            if check_forbid && board.check_forbidden_point(p as usize) {
                continue;
            }
            buf[legal_len] = ScoredMove::new(p);
            raw_moves[legal_len] = p;
            legal_len += 1;
        }

        if legal_len > 1 {
            let side_val = if self_side == FE_BLACK { BLACK } else { WHITE };
            evaluator.evaluate_policy_moves(
                side_val,
                &raw_moves[..legal_len],
                &mut raw_scores[..legal_len],
            );
            for i in 0..legal_len {
                let s = clamp_move_score((raw_scores[i] * POLICY_SCORE_SCALE) as i32);
                buf[i].raw_score = s;
                buf[i].score = s;
            }
            buf[..legal_len]
                .sort_unstable_by(|a, b| b.score.cmp(&a.score).then_with(|| a.pos.cmp(&b.pos)));
        }

        // Empty board fallback: center (7, 7) = 112.
        if legal_len == 0 && board.move_count() == 0 {
            return vec![(7 * 15 + 7) as u16];
        }

        buf[..legal_len].iter().map(|m| m.pos).collect()
    }

    /// Construct a `MAIN` search `MovePicker` (matching `MovePicker::MovePicker<MAIN>`).
    pub fn new_main<const RENJU: bool>(board: &mut FastBoard, mut tt_move: u16) -> Self {
        let self_side = board.side_to_move();
        let oppo = self_side ^ 1;
        if tt_move == PASS_MOVE || (tt_move as usize) >= BOARD_AREA {
            tt_move = NONE_MOVE;
        }

        let (base_stage, mut ttm_valid) = if board.p4_count(oppo, Pattern4::AFive) > 0 {
            let ok =
                tt_move != NONE_MOVE && board.pattern4(tt_move as usize, oppo) == Pattern4::AFive;
            (Stage::DefendFiveTt, ok)
        } else if board.p4_count(oppo, Pattern4::BFlex4) > 0 {
            let ok = tt_move != NONE_MOVE
                && (board.pattern4(tt_move as usize, FE_BLACK) >= Pattern4::EBlock4
                    || board.pattern4(tt_move as usize, FE_BLACK) == Pattern4::Forbid
                    || board.pattern4(tt_move as usize, FE_WHITE) >= Pattern4::EBlock4);
            (Stage::DefendFourTt, ok)
        } else if board.p4_count(oppo, Pattern4::CBlock4Flex3) > 0
            && (!RENJU || validate_opponent_c_move(board))
        {
            (Stage::DefendB4F3Tt, tt_move != NONE_MOVE)
        } else {
            (Stage::MainTt, tt_move != NONE_MOVE)
        };

        if ttm_valid {
            ttm_valid = board.is_legal_non_pass(tt_move);
        }

        let stage = if ttm_valid {
            base_stage
        } else {
            // Advance from *_TT to *_MOVES.
            match base_stage {
                Stage::MainTt => Stage::MainMoves,
                Stage::DefendFiveTt => Stage::DefendFiveMoves,
                Stage::DefendFourTt => Stage::DefendFourMoves,
                Stage::DefendB4F3Tt => Stage::DefendB4F3Moves,
                _ => Stage::AllMoves,
            }
        };

        Self {
            stage,
            tt_move: if ttm_valid { tt_move } else { NONE_MOVE },
            allow_plain_b4_in_vcf: false,
            has_policy: false,
            cur_score: 0,
            cur_idx: 0,
            moves: MoveList::new(),
        }
    }

    /// Construct a `QVCF` search `MovePicker` (matching `MovePicker::MovePicker<QVCF>`).
    pub fn new_qvcf(
        board: &mut FastBoard,
        mut tt_move: u16,
        depth: f32,
        prev_self_p4: [Pattern4; 2],
    ) -> Self {
        let self_side = board.side_to_move();
        let oppo = self_side ^ 1;
        if tt_move == PASS_MOVE || (tt_move as usize) >= BOARD_AREA {
            tt_move = NONE_MOVE;
        }

        let allow_plain_b4 = depth >= DEPTH_QVCF_FULL
            || (prev_self_p4[0] >= Pattern4::DBlock4Plus
                && prev_self_p4[1] >= Pattern4::DBlock4Plus);

        let (base_stage, mut ttm_valid) = if board.p4_count(oppo, Pattern4::AFive) > 0 {
            let ok =
                tt_move != NONE_MOVE && board.pattern4(tt_move as usize, oppo) == Pattern4::AFive;
            (Stage::DefendFiveTt, ok)
        } else {
            let ok = tt_move != NONE_MOVE
                && board.pattern4(tt_move as usize, self_side) >= Pattern4::EBlock4;
            (Stage::QvcfTt, ok)
        };

        if ttm_valid {
            ttm_valid = board.is_legal_non_pass(tt_move);
        }

        let stage = if ttm_valid {
            base_stage
        } else {
            match base_stage {
                Stage::DefendFiveTt => Stage::DefendFiveMoves,
                Stage::QvcfTt => Stage::QvcfMoves,
                _ => Stage::AllMoves,
            }
        };

        Self {
            stage,
            tt_move: if ttm_valid { tt_move } else { NONE_MOVE },
            allow_plain_b4_in_vcf: allow_plain_b4,
            has_policy: false,
            cur_score: 0,
            cur_idx: 0,
            moves: MoveList::new(),
        }
    }

    #[inline(always)]
    pub fn has_policy_score(&self) -> bool {
        self.has_policy
    }

    #[inline(always)]
    pub fn cur_move_score(&self) -> i16 {
        self.cur_score
    }

    fn score_and_sort_moves<
        const USE_POLICY: bool,
        const USE_MAIN_HIST: bool,
        const USE_CM: bool,
    >(
        &mut self,
        board: &FastBoard,
        evaluator: Option<&mut Mix9svqEvaluator>,
        main_hist: Option<&MainHistory>,
        cm_hist: Option<&CounterMoveHistory>,
    ) {
        let slice = &mut self.moves.as_mut_slice()[self.cur_idx..];
        let n = slice.len();
        if n == 0 {
            return;
        }
        let self_side = board.side_to_move();
        let oppo = self_side ^ 1;
        let weights = move_history_weights(board.rule(), self_side);

        let mut raw_policy = [std::mem::MaybeUninit::<f32>::uninit(); BOARD_AREA];
        let used_nnue = if USE_POLICY {
            if let Some(eval) = evaluator {
                let mut move_list = [std::mem::MaybeUninit::<u16>::uninit(); BOARD_AREA];
                for i in 0..n {
                    move_list[i].write(slice[i].pos);
                    raw_policy[i].write(0.0);
                }
                let side_val = if self_side == FE_BLACK { BLACK } else { WHITE };
                // SAFETY: `move_list[..n]` and `raw_policy[..n]` are initialized above.
                let ml_slice =
                    unsafe { std::slice::from_raw_parts(move_list.as_ptr() as *const u16, n) };
                let rp_slice = unsafe {
                    std::slice::from_raw_parts_mut(raw_policy.as_mut_ptr() as *mut f32, n)
                };
                eval.evaluate_policy_moves(side_val, ml_slice, rp_slice);
                self.has_policy = true;
                true
            } else {
                false
            }
        } else {
            false
        };

        let last_mv = board.last_move();
        let cm_entry = if USE_CM {
            cm_hist.and_then(|cm| {
                if (last_mv as usize) < BOARD_AREA {
                    Some(cm.get(oppo, last_mv as usize))
                } else {
                    None
                }
            })
        } else {
            None
        };

        for i in 0..n {
            let m = &mut slice[i];
            let pos = m.pos as usize;
            let raw = if used_nnue {
                let p = unsafe { raw_policy[i].assume_init() };
                clamp_move_score((p * POLICY_SCORE_SCALE) as i32)
            } else {
                classical_move_score(board, pos, self_side)
            };
            m.raw_score = raw;

            let mut total = raw as i32;
            let self_p4 = board.pattern4(pos, self_side);

            if USE_MAIN_HIST {
                if let Some(mh) = main_hist {
                    if self_p4 >= Pattern4::HFlex3 {
                        total += (mh.get(self_side, pos, HIST_ATTACK) as i32
                            * weights.attack_weight)
                            / WEIGHT_SCALE;
                    } else {
                        total += (mh.get(self_side, pos, HIST_QUIET) as i32 * weights.quiet_weight)
                            / WEIGHT_SCALE;
                    }
                }
            }

            if let Some((cm_pos, cm_p4)) = cm_entry {
                if cm_pos == m.pos && cm_p4 <= self_p4 {
                    total += weights.counter_move_bonus;
                }
            }

            m.score = clamp_move_score(total);
        }

        fast_partial_sort(slice);
    }

    /// Pick next legal move, or `NONE_MOVE` when exhausted.
    #[inline]
    pub fn next_move<const RENJU: bool>(
        &mut self,
        board: &mut FastBoard,
        mut evaluator: Option<&mut Mix9svqEvaluator>,
        main_hist: Option<&MainHistory>,
        cm_hist: Option<&CounterMoveHistory>,
    ) -> u16 {
        loop {
            match self.stage {
                Stage::MainTt => {
                    self.stage = Stage::MainMoves;
                    return self.tt_move;
                }
                Stage::DefendFiveTt => {
                    self.stage = Stage::DefendFiveMoves;
                    return self.tt_move;
                }
                Stage::DefendFourTt => {
                    self.stage = Stage::DefendFourMoves;
                    return self.tt_move;
                }
                Stage::DefendB4F3Tt => {
                    self.stage = Stage::DefendB4F3Moves;
                    return self.tt_move;
                }
                Stage::QvcfTt => {
                    self.stage = Stage::QvcfMoves;
                    return self.tt_move;
                }
                Stage::MainMoves => {
                    self.cur_idx = 0;
                    self.moves.clear();
                    generate_all_list(board, &mut self.moves);
                    self.score_and_sort_moves::<true, true, true>(
                        board,
                        evaluator.take(),
                        main_hist,
                        cm_hist,
                    );
                    self.stage = Stage::AllMoves;
                }
                Stage::DefendFiveMoves => {
                    self.cur_idx = 0;
                    self.moves.clear();
                    if self.tt_move == NONE_MOVE {
                        let mut tmp = [ScoredMove::default(); 2];
                        let n = generate_defend_five(board, &mut tmp, 0);
                        for &m in &tmp[..n] {
                            self.moves.push(m);
                        }
                    }
                    self.stage = Stage::AllMoves;
                }
                Stage::DefendFourMoves => {
                    self.cur_idx = 0;
                    self.moves.clear();
                    let mut tmp = [ScoredMove::default(); BOARD_AREA];
                    let e = generate_defend_four::<false>(board, &mut tmp, 0);
                    let end = generate_vcf::<RENJU>(board, &mut tmp, e);
                    for &m in &tmp[..end] {
                        self.moves.push(m);
                    }
                    self.score_and_sort_moves::<true, true, false>(
                        board,
                        evaluator.take(),
                        main_hist,
                        None,
                    );
                    self.stage = Stage::AllMoves;
                }
                Stage::DefendB4F3Moves => {
                    self.cur_idx = 0;
                    self.moves.clear();
                    let mut tmp = [ScoredMove::default(); BOARD_AREA];
                    let e = generate_defend_b4f3::<RENJU>(board, &mut tmp, 0);
                    if e == 0 {
                        self.stage = Stage::MainMoves;
                        continue;
                    }
                    let end = generate_vcf::<RENJU>(board, &mut tmp, e);
                    for &m in &tmp[..end] {
                        self.moves.push(m);
                    }
                    self.score_and_sort_moves::<true, true, false>(
                        board,
                        evaluator.take(),
                        main_hist,
                        None,
                    );
                    self.stage = Stage::AllMoves;
                }
                Stage::QvcfMoves => {
                    self.cur_idx = 0;
                    self.moves.clear();
                    let self_last = board.last_actual_move(board.side_to_move());
                    let mut tmp = [ScoredMove::default(); 40];
                    let end = if self.allow_plain_b4_in_vcf {
                        generate_neighbors_vcf::<false>(board, &mut tmp, 0, self_last)
                    } else {
                        generate_neighbors_vcf::<true>(board, &mut tmp, 0, self_last)
                    };
                    for &m in &tmp[..end] {
                        self.moves.push(m);
                    }
                    self.score_and_sort_moves::<false, false, false>(board, None, None, None);
                    self.stage = Stage::AllMoves;
                }
                Stage::AllMoves => {
                    let check_forbid = RENJU && board.side_to_move() == FE_BLACK;
                    let len = self.moves.len();
                    while self.cur_idx < len {
                        let m = self.moves.as_slice()[self.cur_idx];
                        self.cur_idx += 1;
                        if m.pos == self.tt_move {
                            continue;
                        }
                        if check_forbid && board.check_forbidden_point(m.pos as usize) {
                            continue;
                        }
                        self.cur_score = m.score;
                        return m.pos;
                    }
                    return NONE_MOVE;
                }
            }
        }
    }
}
