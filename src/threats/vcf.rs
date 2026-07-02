//! VCF tactical search.

use std::collections::HashMap;

use crate::board::Board;
use crate::rules::RuleSet;
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
    pub multi_reply: bool,
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
        let effective_depth = Self::normalize_begin_depth(depth);
        self.search_begin(
            &mut ThreatBoardView::from_board_with_rule(board.clone(), rule),
            side,
            effective_depth,
        )
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
        let ordered_moves = view.threat_moves(side);
        let mut forcing_moves = Vec::new();
        for move_ in ordered_moves {
            view.play(move_, side);
            let (x, y) = crate::board::move_to_xy(move_).expect("move stays valid");
            if view.board.winner() == side || view.has_winning_a4(x, y) {
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
}
