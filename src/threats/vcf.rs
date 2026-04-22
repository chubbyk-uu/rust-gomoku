//! VCF tactical search.

use std::collections::HashMap;

use crate::board::Board;
use crate::threats::threat_board::ThreatBoardView;
use crate::types::{Move, Side};

pub const NO_MOVE: Move = u16::MAX;
pub const VCFM: i32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, Default)]
pub struct VCFSearcher {
    pub memo: HashMap<(Side, Vec<Move>, Vec<Move>), VcfMemoEntry>,
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
        let effective_depth = Self::normalize_begin_depth(depth);
        self.search_begin(
            &mut ThreatBoardView::from_board(board.clone()),
            side,
            effective_depth,
        )
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
        if depth <= 0 {
            return VCFResult {
                move_: None,
                found: false,
                solved: false,
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
            if !opponent_ambiguous {
                view.play(opponent_b4, side);
                let (tx, ty) = crate::board::move_to_xy(opponent_b4).expect("move stays valid");
                if view.board.winner() == side || view.has_a4(tx, ty) {
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
        let ordered_moves = view.threat_moves(side);
        let mut forcing_moves = Vec::new();
        for move_ in ordered_moves {
            view.play(move_, side);
            let (x, y) = crate::board::move_to_xy(move_).expect("move stays valid");
            if view.board.winner() == side || view.has_a4(x, y) {
                view.undo();
                let result = VCFResult {
                    move_: Some(move_),
                    found: true,
                    solved: true,
                };
                self.memo.insert(key, VcfMemoEntry { depth, result });
                return result;
            }
            if view.broken_four_reply(x, y).is_some() {
                forcing_moves.push(move_);
            }
            view.undo();
        }

        for move_ in forcing_moves {
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
        if depth < 0 {
            return VCFResult {
                move_: None,
                found: false,
                solved: false,
            };
        }
        let (x, y) = crate::board::move_to_xy(attacker_move).expect("move stays valid");
        let reply = view.broken_four_reply(x, y);
        if reply.is_none() {
            return VCFResult {
                move_: None,
                found: false,
                solved: true,
            };
        }
        let reply = reply.expect("checked above");
        view.play(reply, -attacker);
        let mut next_defender_moves = defender_moves.to_vec();
        next_defender_moves.push(reply);
        let result =
            self.search_attacker(view, attacker, depth, attacker_moves, &next_defender_moves);
        view.undo();
        result
    }
}
