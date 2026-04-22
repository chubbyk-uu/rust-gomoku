//! Victory by Continuous Threats (VCT) search.

use std::collections::HashMap;

use crate::board::Board;
use crate::threats::threat_board::ThreatBoardView;
use crate::threats::types::{AttackMove, ThreatLevel};
use crate::types::{Move, Side};

const OR_NODE: i32 = 0;
const AND_NODE: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VCTResult {
    pub move_: Option<Move>,
    pub found: bool,
    pub solved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VctMemoEntry {
    pub depth: i32,
    pub result: VCTResult,
}

#[derive(Clone, Debug, Default)]
pub struct VCTSearcher {
    pub memo: HashMap<(i32, Side, i32, u64), VctMemoEntry>,
}

impl VCTSearcher {
    pub fn search(&mut self, board: &Board, side: Side, depth: i32) -> VCTResult {
        if depth <= 0 {
            return VCTResult {
                move_: None,
                found: false,
                solved: false,
            };
        }
        if board.winner() != 0 {
            return VCTResult {
                move_: None,
                found: false,
                solved: true,
            };
        }
        let mut view = ThreatBoardView::from_board(board.clone());
        self.memo.clear();
        let mut result = VCTResult {
            move_: None,
            found: false,
            solved: false,
        };
        for d in 1..=depth {
            result = self.or_node(&mut view, side, d);
            if result.found || result.solved {
                return result;
            }
        }
        result
    }

    fn memo_lookup(&self, node: i32, attacker: Side, depth: i32, key: u64) -> Option<VCTResult> {
        if let Some(entry) = self.memo.get(&(node, attacker, depth, key)) {
            return Some(entry.result);
        }
        for d in 1..depth {
            if let Some(entry) = self.memo.get(&(node, attacker, d, key)) {
                if entry.result.found {
                    return Some(entry.result);
                }
            }
        }
        None
    }

    fn store(
        &mut self,
        node: i32,
        attacker: Side,
        depth: i32,
        key: u64,
        result: VCTResult,
    ) -> VCTResult {
        self.memo
            .insert((node, attacker, depth, key), VctMemoEntry { depth, result });
        result
    }

    fn or_node(&mut self, view: &mut ThreatBoardView, attacker: Side, depth: i32) -> VCTResult {
        if depth <= 0 {
            return VCTResult {
                move_: None,
                found: false,
                solved: false,
            };
        }
        if view.board.winner() == attacker {
            return VCTResult {
                move_: None,
                found: true,
                solved: true,
            };
        }
        if view.board.winner() == -attacker {
            return VCTResult {
                move_: None,
                found: false,
                solved: true,
            };
        }

        let key = view.board.zobrist_key();
        if let Some(cached) = self.memo_lookup(OR_NODE, attacker, depth, key) {
            return cached;
        }

        let attacks = view.collect_attack_moves(attacker);
        if attacks.is_empty() {
            return self.store(
                OR_NODE,
                attacker,
                depth,
                key,
                VCTResult {
                    move_: None,
                    found: false,
                    solved: true,
                },
            );
        }

        let mut solved = true;
        for attack in attacks {
            if attack.level >= ThreatLevel::A4 {
                return self.store(
                    OR_NODE,
                    attacker,
                    depth,
                    key,
                    VCTResult {
                        move_: Some(attack.move_),
                        found: true,
                        solved: true,
                    },
                );
            }

            view.play(attack.move_, attacker);
            let defenses = self.collect_defenses(view, &attack, attacker);
            let and_result = self.and_node(view, attacker, depth, &defenses);
            view.undo();

            if and_result.found {
                return self.store(
                    OR_NODE,
                    attacker,
                    depth,
                    key,
                    VCTResult {
                        move_: Some(attack.move_),
                        found: true,
                        solved: true,
                    },
                );
            }
            if !and_result.solved {
                solved = false;
            }
        }

        self.store(
            OR_NODE,
            attacker,
            depth,
            key,
            VCTResult {
                move_: None,
                found: false,
                solved,
            },
        )
    }

    fn and_node(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        depth: i32,
        defenses: &[Move],
    ) -> VCTResult {
        if defenses.is_empty() {
            return VCTResult {
                move_: None,
                found: true,
                solved: true,
            };
        }

        let key = view.board.zobrist_key();
        if let Some(cached) = self.memo_lookup(AND_NODE, attacker, depth, key) {
            return cached;
        }

        let mut solved = true;
        for &d_move in defenses {
            if !view.board.is_legal_move(d_move) {
                continue;
            }

            view.play(d_move, -attacker);
            let (dx, dy) = crate::board::move_to_xy(d_move).expect("move stays valid");
            if view.board.winner() == -attacker || view.has_a4(dx, dy) {
                view.undo();
                return self.store(
                    AND_NODE,
                    attacker,
                    depth,
                    key,
                    VCTResult {
                        move_: None,
                        found: false,
                        solved: true,
                    },
                );
            }

            let or_result = self.or_node(view, attacker, depth - 1);
            view.undo();

            if !or_result.found {
                return self.store(
                    AND_NODE,
                    attacker,
                    depth,
                    key,
                    VCTResult {
                        move_: None,
                        found: false,
                        solved: or_result.solved,
                    },
                );
            }
            if !or_result.solved {
                solved = false;
            }
        }

        self.store(
            AND_NODE,
            attacker,
            depth,
            key,
            VCTResult {
                move_: None,
                found: true,
                solved,
            },
        )
    }

    fn collect_defenses(
        &mut self,
        view: &mut ThreatBoardView,
        attack: &AttackMove,
        attacker: Side,
    ) -> Vec<Move> {
        let defender = -attacker;
        let forced = attack.defenses.clone();
        let mut counter_wins = Vec::new();
        let mut counter_b4 = Vec::new();
        let mut counter_a3 = Vec::new();

        for m in view.threat_moves(defender) {
            if !view.board.is_legal_move(m) {
                continue;
            }
            view.play(m, defender);
            let (dx, dy) = crate::board::move_to_xy(m).expect("move stays valid");
            if view.board.winner() == defender || view.has_a4(dx, dy) {
                counter_wins.push(m);
            } else {
                let b4 = view.b4_count(dx, dy);
                if b4 >= 1 {
                    counter_b4.push(m);
                } else if view.a3r_count(dx, dy) >= 1 {
                    counter_a3.push(m);
                }
            }
            view.undo();
        }

        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for m in counter_wins
            .into_iter()
            .chain(forced)
            .chain(counter_b4)
            .chain(counter_a3)
        {
            if seen.insert(m) && view.board.is_legal_move(m) {
                result.push(m);
            }
        }
        result
    }
}
