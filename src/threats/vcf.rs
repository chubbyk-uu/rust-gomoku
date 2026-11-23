//! VCF tactical search.

use std::collections::HashMap;

use crate::board::Board;
use crate::constants::BLACK;
use crate::rules::RuleSet;
use crate::search::budget::MoveBudget;
use crate::threats::threat_board::{LegalityCache, ThreatBoardView};
use crate::threats::vct::MoveSet225;
use crate::types::{Move, Side};

pub const NO_MOVE: Move = u16::MAX;
pub const VCFM: i32 = 5;
pub const DEFAULT_DEEP_VCF_DEPTH: i32 = 16;
pub const DEFAULT_DEEP_OPPONENT_VCF_DEPTH: i32 = 12;
pub const DEFAULT_DEEP_VCF_NODE_LIMIT: usize = 120_000;

const VCF_TT_BITS: usize = 16;
const VCF_TT_SIZE: usize = 1 << VCF_TT_BITS;
const VCF_TT_MASK: usize = VCF_TT_SIZE - 1;
const VCF_TT_FLAG_FOUND: u8 = 1;
const VCF_TT_FLAG_SOLVED: u8 = 2;
const VCF_BLACK_SALT: u64 = 0x517C_C1B7_2722_0A95;
const VCF_WHITE_SALT: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Clone, Copy, Debug, Default)]
struct VcfTtEntry {
    key: u64,
    generation: u32,
    best_move: Move,
    depth: u8,
    flags: u8,
}

#[derive(Debug)]
struct VcfTt {
    entries: Box<[VcfTtEntry]>,
    generation: u32,
}

impl VcfTt {
    fn new() -> Self {
        Self {
            entries: vec![VcfTtEntry::default(); VCF_TT_SIZE].into_boxed_slice(),
            generation: 1,
        }
    }

    fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            for e in self.entries.iter_mut() {
                e.generation = 0;
            }
            self.generation = 1;
        }
    }

    #[inline(always)]
    fn probe(&self, key: u64, depth: i32) -> Option<VCFResult> {
        let e = self.entries[(key as usize) & VCF_TT_MASK];
        if e.generation != self.generation || e.key != key {
            return None;
        }
        let found = (e.flags & VCF_TT_FLAG_FOUND) != 0;
        let solved = (e.flags & VCF_TT_FLAG_SOLVED) != 0;
        if found || solved || i32::from(e.depth) >= depth {
            return Some(VCFResult {
                move_: (e.best_move != NO_MOVE).then_some(e.best_move),
                found,
                solved,
            });
        }
        None
    }

    #[inline(always)]
    fn store(&mut self, key: u64, depth: i32, result: VCFResult) {
        let slot = &mut self.entries[(key as usize) & VCF_TT_MASK];
        let d = depth.clamp(0, u8::MAX as i32) as u8;
        let flags = (u8::from(result.found) * VCF_TT_FLAG_FOUND)
            | (u8::from(result.solved) * VCF_TT_FLAG_SOLVED);
        if slot.generation == self.generation && slot.key == key {
            let old_decided = (slot.flags & (VCF_TT_FLAG_FOUND | VCF_TT_FLAG_SOLVED)) != 0;
            let new_decided = (flags & (VCF_TT_FLAG_FOUND | VCF_TT_FLAG_SOLVED)) != 0;
            if old_decided && !new_decided {
                return;
            }
            if !new_decided && slot.depth > d {
                return;
            }
        }
        *slot = VcfTtEntry {
            key,
            generation: self.generation,
            best_move: result.move_.unwrap_or(NO_MOVE),
            depth: d,
            flags,
        };
    }
}

#[inline(always)]
fn vcf_pos_key(board: &Board, side: Side) -> u64 {
    board.zobrist_key()
        ^ if side == BLACK {
            VCF_BLACK_SALT
        } else {
            VCF_WHITE_SALT
        }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VCFResult {
    pub move_: Option<Move>,
    pub found: bool,
    pub solved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VcfMemoEntry {
    pub depth: i32,
    pub result: VCFResult,
}

#[derive(Debug, Default)]
pub struct VCFSearcher {
    pub(crate) budget: MoveBudget,
    pub memo: HashMap<(Side, Vec<Move>, Vec<Move>), VcfMemoEntry>,
    pub multi_reply: bool,
    legality_cache: Option<Box<LegalityCache>>,
    tt: Option<VcfTt>,
}

impl Clone for VCFSearcher {
    fn clone(&self) -> Self {
        Self {
            budget: self.budget.clone(),
            memo: self.memo.clone(),
            multi_reply: self.multi_reply,
            legality_cache: None,
            tt: None,
        }
    }
}

impl VCFSearcher {
    pub fn canonical_sequence_key(
        attacker_moves: &[Move],
        defender_moves: &[Move],
    ) -> (Vec<Move>, Vec<Move>) {
        let mut attacker = attacker_moves.to_vec();
        let mut defender = defender_moves.to_vec();
        attacker.sort_unstable();
        defender.sort_unstable();
        (attacker, defender)
    }

    pub fn search(&mut self, board: &Board, side: Side, depth: i32) -> VCFResult {
        self.search_for_rule(board, side, depth, RuleSet::Freestyle)
    }

    pub fn search_for_rule(
        &mut self,
        board: &Board,
        side: Side,
        depth: i32,
        rule: RuleSet,
    ) -> VCFResult {
        if self.budget.stopped() {
            return VCFResult::default();
        }
        let effective_depth = Self::normalize_begin_depth(depth);
        let cache = (rule == RuleSet::Renju)
            .then(|| self.legality_cache.take())
            .flatten();
        let mut view = ThreatBoardView::from_board_with_rule_and_cache(board.clone(), rule, cache);
        let result = self.search_begin(&mut view, side, effective_depth);
        if let Some(cache) = view.take_legality_cache() {
            self.legality_cache = Some(cache);
        }
        result
    }

    pub fn search_with_multi_reply(
        &mut self,
        board: &Board,
        side: Side,
        depth: i32,
        multi_reply: bool,
    ) -> VCFResult {
        self.search_with_multi_reply_for_rule(board, side, depth, multi_reply, RuleSet::Freestyle)
    }

    pub fn search_with_multi_reply_for_rule(
        &mut self,
        board: &Board,
        side: Side,
        depth: i32,
        multi_reply: bool,
        rule: RuleSet,
    ) -> VCFResult {
        let previous = self.multi_reply;
        self.multi_reply = multi_reply;
        let result = self.search_for_rule(board, side, depth, rule);
        self.multi_reply = previous;
        result
    }

    pub fn normalize_begin_depth(depth: i32) -> i32 {
        if depth <= 0 {
            return depth;
        }
        if depth == 8 {
            return depth.min(VCFM);
        }
        depth.min(VCFM - 1)
    }

    pub fn search_begin(
        &mut self,
        view: &mut ThreatBoardView,
        side: Side,
        depth: i32,
    ) -> VCFResult {
        if depth <= 0 {
            return VCFResult {
                move_: None,
                found: false,
                solved: false,
            };
        }
        self.memo.clear();
        let shallower = self.search_begin(view, side, depth - 1);
        if shallower.found {
            return shallower;
        }
        if shallower.solved {
            return VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
        }
        self.search_attacker(view, side, depth, &[], &[])
    }

    pub fn search_attacker(
        &mut self,
        view: &mut ThreatBoardView,
        side: Side,
        depth: i32,
        attacker_moves: &[Move],
        defender_moves: &[Move],
    ) -> VCFResult {
        if !self.budget.enter() || depth <= 0 {
            return VCFResult {
                move_: None,
                found: false,
                solved: false,
            };
        }
        if view.board.winner() == side {
            return VCFResult {
                move_: None,
                found: true,
                solved: true,
            };
        }
        if view.board.winner() == -side {
            return VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
        }

        let sequence_key = Self::canonical_sequence_key(attacker_moves, defender_moves);
        let key = (side, sequence_key.0, sequence_key.1);
        if let Some(memoized) = self.memo.get(&key) {
            if memoized.result.found || memoized.result.solved {
                return memoized.result;
            }
            if memoized.depth == depth {
                return memoized.result;
            }
        }

        let (direct_b4, _) = view.broken_four_point_for_side(side);
        if let Some(direct_b4) = direct_b4 {
            let result = VCFResult {
                move_: Some(direct_b4),
                found: true,
                solved: true,
            };
            self.memo.insert(key, VcfMemoEntry { depth, result });
            return result;
        }

        let (opponent_b4, opponent_ambiguous) = view.broken_four_point_for_side(-side);
        if let Some(opponent_b4) = opponent_b4 {
            if !opponent_ambiguous && view.is_rule_legal(opponent_b4, side) {
                view.play(opponent_b4, side);
                let (tx, ty) = crate::board::move_to_xy(opponent_b4).expect("move stays valid");
                if view.board.winner() == side || view.has_winning_a4(tx, ty) {
                    view.undo();
                    let result = VCFResult {
                        move_: Some(opponent_b4),
                        found: true,
                        solved: true,
                    };
                    self.memo.insert(key, VcfMemoEntry { depth, result });
                    return result;
                }
                let mut next_attacker_moves = attacker_moves.to_vec();
                next_attacker_moves.push(opponent_b4);
                let defender = self.search_defender(
                    view,
                    side,
                    depth - 1,
                    opponent_b4,
                    &next_attacker_moves,
                    defender_moves,
                );
                view.undo();
                if defender.found {
                    let result = VCFResult {
                        move_: Some(opponent_b4),
                        found: true,
                        solved: true,
                    };
                    self.memo.insert(key, VcfMemoEntry { depth, result });
                    return result;
                }
                if !defender.solved {
                    let result = VCFResult {
                        move_: None,
                        found: false,
                        solved: false,
                    };
                    self.memo.insert(key, VcfMemoEntry { depth, result });
                    return result;
                }
                let result = VCFResult {
                    move_: None,
                    found: false,
                    solved: true,
                };
                self.memo.insert(key, VcfMemoEntry { depth, result });
                return result;
            }
            let result = VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
            self.memo.insert(key, VcfMemoEntry { depth, result });
            return result;
        }

        let mut solved = true;
        let mut forcing_moves = Vec::new();
        let four_bits = view.pat4.a4_bits(side).union(view.pat4.b4_bits(side));
        for idx in four_bits.iter_indices() {
            if self.budget.stopped() {
                return VCFResult::default();
            }
            let move_ = crate::threats::pattern4::idx_to_move(idx);
            if !view.fast_rule_legal_empty(move_, idx, side) {
                continue;
            }
            if view.pat4.cell_pattern(side, idx).has_straight_a4() {
                let result = VCFResult {
                    move_: Some(move_),
                    found: true,
                    solved: true,
                };
                self.memo.insert(key, VcfMemoEntry { depth, result });
                return result;
            }
            forcing_moves.push(move_);
        }

        for move_ in forcing_moves {
            if self.budget.stopped() {
                return VCFResult::default();
            }
            view.play(move_, side);
            let mut next_attacker_moves = attacker_moves.to_vec();
            next_attacker_moves.push(move_);
            let defender = self.search_defender(
                view,
                side,
                depth - 1,
                move_,
                &next_attacker_moves,
                defender_moves,
            );
            view.undo();
            if defender.found {
                let result = VCFResult {
                    move_: Some(move_),
                    found: true,
                    solved: true,
                };
                self.memo.insert(key, VcfMemoEntry { depth, result });
                return result;
            }
            if !defender.solved {
                solved = false;
            }
        }

        let result = VCFResult {
            move_: None,
            found: false,
            solved,
        };
        self.memo.insert(key, VcfMemoEntry { depth, result });
        result
    }

    pub fn search_defender(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        depth: i32,
        attacker_move: Move,
        attacker_moves: &[Move],
        defender_moves: &[Move],
    ) -> VCFResult {
        if !self.budget.enter() || depth < 0 {
            return VCFResult {
                move_: None,
                found: false,
                solved: false,
            };
        }
        let (x, y) = crate::board::move_to_xy(attacker_move).expect("move stays valid");
        let replies = view.broken_four_legal_replies_for_side(x, y, -attacker);
        if replies.is_empty() {
            // No legal reply can mean two very different things: either the
            // attacker move did not actually create a four (the forcing chain
            // is broken), or a four exists but every block is forbidden for
            // the defender (Renju black), in which case the attacker completes
            // five on the next move and wins. Gate the win claim explicitly on
            // the Renju black-defender case; in every other configuration an
            // empty reply set falls back to the conservative "chain broken".
            if view.rule_set() == RuleSet::Renju
                && -attacker == crate::constants::BLACK
                && view.broken_four_reply(x, y).is_some()
            {
                return VCFResult {
                    move_: None,
                    found: true,
                    solved: true,
                };
            }
            return VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
        }
        if !self.multi_reply {
            let reply = replies.first().expect("checked above");
            view.play(reply, -attacker);
            let mut next_defender_moves = defender_moves.to_vec();
            next_defender_moves.push(reply);
            let result =
                self.search_attacker(view, attacker, depth, attacker_moves, &next_defender_moves);
            view.undo();
            return result;
        }

        let mut found_move = None;
        let mut any_unsolved = false;
        let mut next_defender_moves = defender_moves.to_vec();
        for &reply in replies.as_slice() {
            if self.budget.stopped() {
                return VCFResult::default();
            }
            view.play(reply, -attacker);
            next_defender_moves.push(reply);
            let result =
                self.search_attacker(view, attacker, depth, attacker_moves, &next_defender_moves);
            next_defender_moves.pop();
            view.undo();

            if result.found {
                found_move = found_move.or(result.move_);
                continue;
            }
            if result.solved {
                return VCFResult {
                    move_: None,
                    found: false,
                    solved: true,
                };
            }
            any_unsolved = true;
        }

        if any_unsolved {
            VCFResult {
                move_: None,
                found: false,
                solved: false,
            }
        } else {
            VCFResult {
                move_: found_move,
                found: true,
                solved: true,
            }
        }
    }

    fn ensure_tt(&mut self) {
        if self.tt.is_none() {
            self.tt = Some(VcfTt::new());
        }
    }

    /// Deep Zobrist-TT VCF solver without the legacy `VCFM = 5` cap and with
    /// zero per-node `Vec` allocations. Protected by a local `node_limit` in
    /// addition to `self.budget`.
    pub fn search_deep_for_rule(
        &mut self,
        board: &Board,
        side: Side,
        max_depth: i32,
        multi_reply: bool,
        rule: RuleSet,
        node_limit: usize,
    ) -> VCFResult {
        let (result, _) =
            self.search_deep_internal(board, side, max_depth, multi_reply, rule, node_limit, false);
        result
    }

    /// Deep Zobrist-TT VCF solver that also collects the winning proof-set
    /// squares (all attacker moves, defender replies, and terminal open-four
    /// endpoints) when a VCF is found.
    pub(crate) fn search_deep_with_proof_set(
        &mut self,
        board: &Board,
        side: Side,
        max_depth: i32,
        multi_reply: bool,
        rule: RuleSet,
        node_limit: usize,
    ) -> (VCFResult, MoveSet225) {
        self.search_deep_internal(board, side, max_depth, multi_reply, rule, node_limit, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_deep_internal(
        &mut self,
        board: &Board,
        side: Side,
        max_depth: i32,
        multi_reply: bool,
        rule: RuleSet,
        node_limit: usize,
        collect_proof: bool,
    ) -> (VCFResult, MoveSet225) {
        let mut proof_set = MoveSet225::new();
        if max_depth <= 0 || self.budget.stopped() {
            return (VCFResult::default(), proof_set);
        }
        let cache = (rule == RuleSet::Renju)
            .then(|| self.legality_cache.take())
            .flatten();
        let mut view = ThreatBoardView::from_board_with_rule_and_cache(board.clone(), rule, cache);
        let result = self.search_deep_on_view(
            &mut view,
            side,
            max_depth,
            multi_reply,
            node_limit,
            collect_proof.then_some(&mut proof_set),
        );
        if let Some(cache) = view.take_legality_cache() {
            self.legality_cache = Some(cache);
        }
        (result, proof_set)
    }

    /// Run deep Zobrist-TT VCF directly on an existing `ThreatBoardView`
    /// (enabling zero-clone `play/undo` trial filtering at the root).
    pub(crate) fn search_deep_on_view(
        &mut self,
        view: &mut ThreatBoardView,
        side: Side,
        max_depth: i32,
        multi_reply: bool,
        node_limit: usize,
        mut proof_set: Option<&mut MoveSet225>,
    ) -> VCFResult {
        if max_depth <= 0 || self.budget.stopped() {
            return VCFResult::default();
        }
        self.ensure_tt();
        self.tt.as_mut().expect("tt initialized").next_generation();
        let previous_multi = self.multi_reply;
        self.multi_reply = multi_reply;
        let mut remaining_nodes = node_limit;
        let mut final_result = VCFResult::default();
        for depth in 1..=max_depth {
            if self.budget.stopped() || remaining_nodes == 0 {
                break;
            }
            if let Some(ps) = proof_set.as_deref_mut() {
                *ps = MoveSet225::new();
            }
            let res = self.search_attacker_deep(
                view,
                side,
                depth,
                &mut remaining_nodes,
                proof_set.as_deref_mut(),
            );
            if res.found || res.solved {
                final_result = res;
                break;
            }
            final_result = res;
        }
        self.multi_reply = previous_multi;
        final_result
    }

    fn search_attacker_deep(
        &mut self,
        view: &mut ThreatBoardView,
        side: Side,
        depth: i32,
        remaining_nodes: &mut usize,
        mut proof_set: Option<&mut MoveSet225>,
    ) -> VCFResult {
        if *remaining_nodes == 0 || !self.budget.enter() || depth <= 0 {
            return VCFResult::default();
        }
        *remaining_nodes -= 1;

        if view.board.winner() == side {
            return VCFResult {
                move_: None,
                found: true,
                solved: true,
            };
        }
        if view.board.winner() == -side {
            return VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
        }

        let key = vcf_pos_key(&view.board, side);
        // When collecting a proof set, bypass positive TT hits so we actually
        // walk the winning proof tree and record all its moves.
        if let Some(hit) = self.tt.as_ref().expect("tt initialized").probe(key, depth) {
            if !(proof_set.is_some() && hit.found) {
                return hit;
            }
        }

        let (direct_b4, _) = view.broken_four_point_for_side(side);
        if let Some(direct_b4) = direct_b4 {
            if let Some(ps) = proof_set.as_deref_mut() {
                ps.insert(direct_b4);
            }
            let result = VCFResult {
                move_: Some(direct_b4),
                found: true,
                solved: true,
            };
            self.tt
                .as_mut()
                .expect("tt initialized")
                .store(key, depth, result);
            return result;
        }

        let (opponent_b4, opponent_ambiguous) = view.broken_four_point_for_side(-side);
        if let Some(opponent_b4) = opponent_b4 {
            if !opponent_ambiguous && view.is_rule_legal(opponent_b4, side) {
                view.play(opponent_b4, side);
                let (tx, ty) = crate::board::move_to_xy(opponent_b4).expect("move stays valid");
                if view.board.winner() == side || view.has_winning_a4(tx, ty) {
                    if let Some(ps) = proof_set.as_deref_mut() {
                        ps.insert(opponent_b4);
                        let ends = view.broken_four_legal_replies_for_side(tx, ty, -side);
                        for &end_mv in ends.as_slice() {
                            ps.insert(end_mv);
                        }
                    }
                    view.undo();
                    let result = VCFResult {
                        move_: Some(opponent_b4),
                        found: true,
                        solved: true,
                    };
                    self.tt
                        .as_mut()
                        .expect("tt initialized")
                        .store(key, depth, result);
                    return result;
                }
                let mut child_proof = proof_set.as_ref().map(|_| MoveSet225::new());
                let defender = self.search_defender_deep(
                    view,
                    side,
                    depth - 1,
                    opponent_b4,
                    remaining_nodes,
                    child_proof.as_mut(),
                );
                view.undo();
                let result = if defender.found {
                    if let Some(ps) = proof_set.as_deref_mut() {
                        ps.insert(opponent_b4);
                        if let Some(cp) = child_proof {
                            for mv in cp.iter() {
                                ps.insert(mv);
                            }
                        }
                    }
                    VCFResult {
                        move_: Some(opponent_b4),
                        found: true,
                        solved: true,
                    }
                } else {
                    VCFResult {
                        move_: None,
                        found: false,
                        solved: defender.solved,
                    }
                };
                self.tt
                    .as_mut()
                    .expect("tt initialized")
                    .store(key, depth, result);
                return result;
            }
            let result = VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
            self.tt
                .as_mut()
                .expect("tt initialized")
                .store(key, depth, result);
            return result;
        }

        let mut solved = true;
        let mut forcing_moves = [0_u16; 64];
        let mut forcing_len = 0_usize;
        let four_bits = view.pat4.a4_bits(side).union(view.pat4.b4_bits(side));
        for idx in four_bits.iter_indices() {
            if self.budget.stopped() || *remaining_nodes == 0 {
                return VCFResult::default();
            }
            let move_ = crate::threats::pattern4::idx_to_move(idx);
            if !view.fast_rule_legal_empty(move_, idx, side) {
                continue;
            }
            if view.pat4.cell_pattern(side, idx).has_straight_a4() {
                if let Some(ps) = proof_set.as_deref_mut() {
                    ps.insert(move_);
                    view.play(move_, side);
                    let (tx, ty) = crate::board::move_to_xy(move_).expect("move stays valid");
                    let ends = view.broken_four_legal_replies_for_side(tx, ty, -side);
                    for &end_mv in ends.as_slice() {
                        ps.insert(end_mv);
                    }
                    view.undo();
                }
                let result = VCFResult {
                    move_: Some(move_),
                    found: true,
                    solved: true,
                };
                self.tt
                    .as_mut()
                    .expect("tt initialized")
                    .store(key, depth, result);
                return result;
            }
            if forcing_len < forcing_moves.len() {
                forcing_moves[forcing_len] = move_;
                forcing_len += 1;
            }
        }

        for &move_ in &forcing_moves[..forcing_len] {
            if self.budget.stopped() || *remaining_nodes == 0 {
                return VCFResult::default();
            }
            view.play(move_, side);
            let mut child_proof = proof_set.as_ref().map(|_| MoveSet225::new());
            let defender = self.search_defender_deep(
                view,
                side,
                depth - 1,
                move_,
                remaining_nodes,
                child_proof.as_mut(),
            );
            view.undo();
            if defender.found {
                if let Some(ps) = proof_set.as_deref_mut() {
                    ps.insert(move_);
                    if let Some(cp) = child_proof {
                        for mv in cp.iter() {
                            ps.insert(mv);
                        }
                    }
                }
                let result = VCFResult {
                    move_: Some(move_),
                    found: true,
                    solved: true,
                };
                self.tt
                    .as_mut()
                    .expect("tt initialized")
                    .store(key, depth, result);
                return result;
            }
            if !defender.solved {
                solved = false;
            }
        }

        let result = VCFResult {
            move_: None,
            found: false,
            solved,
        };
        if !self.budget.stopped() && *remaining_nodes > 0 {
            self.tt
                .as_mut()
                .expect("tt initialized")
                .store(key, depth, result);
        }
        result
    }

    fn search_defender_deep(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        depth: i32,
        attacker_move: Move,
        remaining_nodes: &mut usize,
        mut proof_set: Option<&mut MoveSet225>,
    ) -> VCFResult {
        if *remaining_nodes == 0 || !self.budget.enter() || depth < 0 {
            return VCFResult::default();
        }
        *remaining_nodes -= 1;

        let (x, y) = crate::board::move_to_xy(attacker_move).expect("move stays valid");
        let replies = view.broken_four_legal_replies_for_side(x, y, -attacker);
        if replies.is_empty() {
            if view.rule_set() == RuleSet::Renju
                && -attacker == BLACK
                && view.broken_four_reply(x, y).is_some()
            {
                return VCFResult {
                    move_: None,
                    found: true,
                    solved: true,
                };
            }
            return VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
        }
        if !self.multi_reply {
            let reply = replies.first().expect("checked above");
            view.play(reply, -attacker);
            let result = self.search_attacker_deep(
                view,
                attacker,
                depth,
                remaining_nodes,
                proof_set.as_deref_mut(),
            );
            view.undo();
            if result.found {
                if let Some(ps) = proof_set.as_deref_mut() {
                    ps.insert(reply);
                }
            }
            return result;
        }

        let mut found_move = None;
        let mut any_unsolved = false;
        let mut acc_proof = proof_set.as_ref().map(|_| MoveSet225::new());
        for &reply in replies.as_slice() {
            if self.budget.stopped() || *remaining_nodes == 0 {
                return VCFResult::default();
            }
            view.play(reply, -attacker);
            let mut branch_proof = proof_set.as_ref().map(|_| MoveSet225::new());
            let result = self.search_attacker_deep(
                view,
                attacker,
                depth,
                remaining_nodes,
                branch_proof.as_mut(),
            );
            view.undo();

            if result.found {
                found_move = found_move.or(result.move_);
                if let Some(ap) = acc_proof.as_mut() {
                    ap.insert(reply);
                    if let Some(bp) = branch_proof {
                        for mv in bp.iter() {
                            ap.insert(mv);
                        }
                    }
                }
                continue;
            }
            if result.solved {
                return VCFResult {
                    move_: None,
                    found: false,
                    solved: true,
                };
            }
            any_unsolved = true;
        }

        if any_unsolved {
            VCFResult::default()
        } else {
            if let (Some(ps), Some(ap)) = (proof_set, acc_proof) {
                for mv in ap.iter() {
                    ps.insert(mv);
                }
            }
            VCFResult {
                move_: found_move,
                found: true,
                solved: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::constants::{BLACK, WHITE};
    use crate::rules::{classify_forbidden_move, ForbiddenKind};

    // Renju: white (3..5,7) with the left extension blocked by black, so white
    // has no direct open four. The only win is 6,7 -> four (3..6,7) whose sole
    // five point (7,7) is a black double-three forbidden point: black cannot
    // legally block, so white completes five next move. The VCF must report
    // this as a win instead of treating "no legal reply" as a broken chain.
    #[test]
    fn renju_vcf_finds_win_through_four_whose_only_block_is_forbidden() {
        let mut board = Board::new();
        for (x, y, side) in [
            (3, 7, WHITE),
            (4, 7, WHITE),
            (5, 7, WHITE),
            (2, 7, BLACK),
            (6, 6, BLACK),
            (8, 6, BLACK),
            (6, 8, BLACK),
            (8, 8, BLACK),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }
        board.force_side_to_move(WHITE).unwrap();
        assert_eq!(
            classify_forbidden_move(&board, xy_to_move(7, 7).unwrap(), BLACK, RuleSet::Renju)
                .unwrap(),
            ForbiddenKind::DoubleThree
        );

        // In freestyle black can block (7,7) legally, so there is no VCF.
        let freestyle = VCFSearcher::default().search_with_multi_reply(&board, WHITE, 8, true);
        assert!(!freestyle.found, "freestyle: black blocks the four legally");

        let renju = VCFSearcher::default().search_with_multi_reply_for_rule(
            &board,
            WHITE,
            8,
            true,
            RuleSet::Renju,
        );
        assert!(
            renju.found,
            "renju: black's only block of the four is forbidden, white wins"
        );
        assert_eq!(renju.move_, Some(xy_to_move(6, 7).unwrap()));
    }

    #[test]
    fn deep_vcf_proof_set_matches_bruteforce_refutations_for_both_rules() {
        // Construct a position where WHITE has a 2-four VCF:
        // 1) (7,7) creates horizontal B4 (4..7,7) forcing BLACK to (8,7);
        // 2) (7,8) completes vertical open-four (7,5..8) with (7,4) and (7,9) open.
        let mut board = Board::new();
        for (x, y, side) in [
            (4, 7, WHITE),
            (5, 7, WHITE),
            (6, 7, WHITE),
            (3, 7, BLACK),
            (9, 7, BLACK),
            (7, 5, WHITE),
            (7, 6, WHITE),
            (0, 0, BLACK),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }
        board.force_side_to_move(BLACK).unwrap();

        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            let mut vcf = VCFSearcher::default();
            let (res, mut relevant) =
                vcf.search_deep_with_proof_set(&board, WHITE, 10, true, rule, 50_000);
            assert!(res.found, "{rule:?}: expected WHITE VCF to be found");

            let mut view = ThreatBoardView::from_board_with_rule(board.clone(), rule);
            let counter_bits = view
                .pat4
                .win5_bits(BLACK)
                .union(view.pat4.a4_bits(BLACK))
                .union(view.pat4.b4_bits(BLACK));
            for idx in counter_bits.iter_indices() {
                relevant.insert(crate::threats::pattern4::idx_to_move(idx));
            }

            let mut proof_refutations = std::collections::HashSet::new();
            for m in relevant.iter() {
                if !view.is_rule_legal(m, BLACK) {
                    continue;
                }
                view.play(m, BLACK);
                if !vcf
                    .search_deep_on_view(&mut view, WHITE, 10, true, 30_000, None)
                    .found
                {
                    proof_refutations.insert(m);
                }
                view.undo();
            }

            let mut brute_refutations = std::collections::HashSet::new();
            for idx in 0..crate::constants::BOARD_AREA {
                let m = idx as Move;
                if !board.is_legal_move_for_rule(m, BLACK, rule) {
                    continue;
                }
                let mut trial = board.clone();
                trial
                    .play_assuming_rule_legal(m, Some(BLACK), rule)
                    .unwrap();
                if !vcf
                    .search_deep_for_rule(&trial, WHITE, 10, true, rule, 30_000)
                    .found
                {
                    brute_refutations.insert(m);
                }
            }

            assert_eq!(
                proof_refutations, brute_refutations,
                "{rule:?}: proof-set refutations must exactly match brute-force 225-cell filter"
            );
        }
    }

    #[test]
    fn deep_vcf_respects_node_limit_and_matches_legacy_on_shallow_win() {
        let mut board = Board::new();
        for (x, y, side) in [
            (3, 7, BLACK),
            (0, 0, WHITE),
            (4, 7, BLACK),
            (1, 0, WHITE),
            (5, 7, BLACK),
        ] {
            board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
        }
        let mut vcf = VCFSearcher::default();
        let res = vcf.search_deep_for_rule(&board, BLACK, 16, true, RuleSet::Freestyle, 10_000);
        assert!(res.found && res.solved);
        assert!(matches!(
            crate::board::move_to_xy(res.move_.unwrap()).unwrap(),
            (2, 7) | (6, 7)
        ));
    }
}
