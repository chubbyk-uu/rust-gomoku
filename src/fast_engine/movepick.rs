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

use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, WHITE};
use crate::fast_engine::board::{FastBoard, NONE_MOVE, PASS_MOVE};
use crate::fast_engine::classical_model::ClassicalPolicyScorer;
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

/// Pack `(score DESC, pos ASC)` into a single comparable `u32` key.
#[inline(always)]
fn scored_key(m: ScoredMove) -> u32 {
    (((m.score as i32 + 32_768) as u32) << 16) | ((u16::MAX - m.pos) as u32)
}

/// Number of top moves selected lazily via O(N) linear scan before sorting the
/// remaining tail. Most search nodes beta-cutoff within the first 1-4 moves,
/// avoiding O(N log N) sorting of ~60 candidates on >90% of nodes while
/// preserving exact `(score DESC, pos ASC)` order.
const LAZY_SELECT_LIMIT: usize = 6;

pub struct MovePicker {
    stage: Stage,
    tt_move: u16,
    allow_plain_b4_in_vcf: bool,
    has_policy: bool,
    tail_sorted: bool,
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
                buf[i].score = s;
            }
            buf[..legal_len]
                .sort_unstable_by(|a, b| b.score.cmp(&a.score).then_with(|| a.pos.cmp(&b.pos)));
        }

        // Empty board fallback: center (7, 7) = 112.
        if legal_len == 0 && board.move_count() == 0 {
            return vec![(7 * 15 + 7) as u16];
        }

        let mut out: Vec<u16> = buf[..legal_len].iter().map(|m| m.pos).collect();
        filter_root_symmetry_moves(board, &mut out);
        out
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
            tail_sorted: true,
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
            tail_sorted: true,
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
                // SAFETY: Both buffers have their first n elements initialized above.
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

        let classical_scorer = if !used_nnue {
            board
                .classical()
                .map(|cm| ClassicalPolicyScorer::new(cm, board))
        } else {
            None
        };

        for i in 0..n {
            let m = &mut slice[i];
            let pos = m.pos as usize;
            let self_p4 = board.pattern4(pos, self_side);
            let raw = if used_nnue {
                let p = unsafe { raw_policy[i].assume_init() };
                clamp_move_score((p * POLICY_SCORE_SCALE) as i32)
            } else if let Some(ref scorer) = classical_scorer {
                let (pb, pw) = board.pcode_pair(pos);
                scorer.score(pb, pw, self_p4, board.pattern4(pos, oppo))
            } else {
                classical_move_score(board, pos, self_side)
            };
            let mut total = raw as i32;

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

        self.tail_sorted = n <= 1;
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
                        let idx = self.cur_idx;
                        if !self.tail_sorted {
                            let slice = self.moves.as_mut_slice();
                            if idx < LAZY_SELECT_LIMIT {
                                let mut best_i = idx;
                                let mut best_k = scored_key(slice[idx]);
                                for j in (idx + 1)..len {
                                    let k = scored_key(slice[j]);
                                    if k > best_k {
                                        best_k = k;
                                        best_i = j;
                                    }
                                }
                                if best_i != idx {
                                    slice.swap(idx, best_i);
                                }
                            } else {
                                slice[idx..]
                                    .sort_unstable_by(|a, b| scored_key(*b).cmp(&scored_key(*a)));
                                self.tail_sorted = true;
                            }
                        }
                        let m = self.moves.as_slice()[idx];
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

#[inline(always)]
fn transform_pos(pos: usize, sym: usize) -> usize {
    let mut x = pos % BOARD_SIZE;
    let mut y = pos / BOARD_SIZE;
    if (sym & 1) != 0 {
        x = BOARD_SIZE - 1 - x;
    }
    if (sym & 2) != 0 {
        y = BOARD_SIZE - 1 - y;
    }
    if (sym & 4) != 0 {
        std::mem::swap(&mut x, &mut y);
    }
    y * BOARD_SIZE + x
}

fn uf_find(parent: &mut [u16; BOARD_AREA], mut x: usize) -> usize {
    while parent[x] as usize != x {
        let p = parent[x] as usize;
        parent[x] = parent[p];
        x = p;
    }
    x
}

fn uf_union(parent: &mut [u16; BOARD_AREA], a: usize, b: usize) {
    let ra = uf_find(parent, a);
    let rb = uf_find(parent, b);
    if ra != rb {
        if ra < rb {
            parent[rb] = ra as u16;
        } else {
            parent[ra] = rb as u16;
        }
    }
}

/// Deduplicate root candidate moves under the board's exact dihedral symmetries
/// (`opponent/rapfi/Rapfi/search/opening.cpp:250-301`), preserving the
/// highest-ranked policy move from each symmetry orbit.
pub fn filter_root_symmetry_moves(board: &FastBoard, moves: &mut Vec<u16>) {
    if moves.len() <= 1 {
        return;
    }

    let mut active_mask = 0_u8;
    for sym in 1..8_usize {
        if (0..BOARD_AREA).all(|p| board.cell_bits(p) == board.cell_bits(transform_pos(p, sym))) {
            active_mask |= 1_u8 << sym;
        }
    }
    if active_mask == 0 {
        return;
    }

    let mut parent = [0_u16; BOARD_AREA];
    for i in 0..BOARD_AREA {
        parent[i] = i as u16;
    }
    for sym in 1..8_usize {
        if (active_mask & (1_u8 << sym)) == 0 {
            continue;
        }
        for pos in 0..BOARD_AREA {
            if board.is_empty(pos) {
                uf_union(&mut parent, pos, transform_pos(pos, sym));
            }
        }
    }

    let mut seen_orbit = [false; BOARD_AREA];
    moves.retain(|&mv| {
        let root = uf_find(&mut parent, mv as usize);
        if seen_orbit[root] {
            false
        } else {
            seen_orbit[root] = true;
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_engine::board::CandRange;

    #[test]
    fn root_symmetry_filter_collapses_symmetric_and_keeps_asymmetric() {
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        fb.play(7 * BOARD_SIZE + 7);
        let mut cands = Vec::new();
        fb.empty_candidates().for_each_bit(|p| cands.push(p as u16));
        let raw_len = cands.len();
        filter_root_symmetry_moves(&fb, &mut cands);
        // 7x7 square around center collapses to (1..=3)*(2..=4)/2 + diagonal = 9 orbits;
        // plus 4 axis-ray dist-4 cells collapsing to 1 orbit => 10 unique orbits.
        assert!(
            cands.len() < raw_len / 4,
            "symmetric board should collapse orbits: {raw_len} -> {}",
            cands.len()
        );

        let mut asym = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        asym.play(7 * BOARD_SIZE + 7);
        asym.play(6 * BOARD_SIZE + 7);
        asym.play(8 * BOARD_SIZE + 6);
        let mut asym_cands = Vec::new();
        asym.empty_candidates()
            .for_each_bit(|p| asym_cands.push(p as u16));
        let expected = asym_cands.clone();
        filter_root_symmetry_moves(&asym, &mut asym_cands);
        assert_eq!(asym_cands, expected);
    }
}
