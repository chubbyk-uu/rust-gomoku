//! Victory by Continuous Threats (VCT) search.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::board::Board;
use crate::rules::RuleSet;
use crate::threats::threat_board::ThreatBoardView;
use crate::threats::types::{AttackMove, ThreatLevel};
use crate::types::{Move, Side};

const OR_NODE: i32 = 0;
const AND_NODE: i32 = 1;
const MAX_AND_MEMO_COLLISION_SAMPLES: usize = 8;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VCTAndMemoCollisionSample {
    pub observed_depth: i32,
    pub current_depth: i32,
    pub board_key: u64,
    pub observed_signature: u64,
    pub current_signature: u64,
    pub attack_move: Move,
    pub attack_level: u8,
    pub defenses: Vec<Move>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VCTDepthStats {
    pub depth: i32,
    pub elapsed_us: u128,
    pub found: bool,
    pub solved: bool,
    pub or_nodes: usize,
    pub and_nodes: usize,
    pub memo_exact_hits: usize,
    pub memo_shallow_found_hits: usize,
    pub memo_shallow_solved_hits: usize,
    pub attacks_generated: usize,
    pub defenses_generated: usize,
    pub max_attack_count: usize,
    pub max_defense_count: usize,
    pub and_memo_context_observations: usize,
    pub and_memo_context_collisions: usize,
    pub and_memo_context_collision_keys: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct VCTStatsSnapshot {
    or_nodes: usize,
    and_nodes: usize,
    memo_exact_hits: usize,
    memo_shallow_found_hits: usize,
    memo_shallow_solved_hits: usize,
    attacks_generated: usize,
    defenses_generated: usize,
    and_memo_context_observations: usize,
    and_memo_context_collisions: usize,
    and_memo_context_collision_keys: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VCTStats {
    pub depth_limit: i32,
    pub depth_completed: i32,
    pub elapsed_us: u128,
    pub or_nodes: usize,
    pub and_nodes: usize,
    pub memo_exact_hits: usize,
    pub memo_shallow_found_hits: usize,
    pub memo_shallow_solved_hits: usize,
    pub attacks_generated: usize,
    pub defenses_generated: usize,
    pub max_attack_count: usize,
    pub max_defense_count: usize,
    pub and_memo_context_observations: usize,
    pub and_memo_context_collisions: usize,
    pub and_memo_context_collision_keys: usize,
    pub and_memo_context_collision_samples: Vec<VCTAndMemoCollisionSample>,
    pub depth_stats: Vec<VCTDepthStats>,
    current_depth_max_attack_count: usize,
    current_depth_max_defense_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DefenseStageResult {
    refutation: Option<VCTResult>,
    solved: bool,
    searched: bool,
}

impl VCTStats {
    fn snapshot(&self) -> VCTStatsSnapshot {
        VCTStatsSnapshot {
            or_nodes: self.or_nodes,
            and_nodes: self.and_nodes,
            memo_exact_hits: self.memo_exact_hits,
            memo_shallow_found_hits: self.memo_shallow_found_hits,
            memo_shallow_solved_hits: self.memo_shallow_solved_hits,
            attacks_generated: self.attacks_generated,
            defenses_generated: self.defenses_generated,
            and_memo_context_observations: self.and_memo_context_observations,
            and_memo_context_collisions: self.and_memo_context_collisions,
            and_memo_context_collision_keys: self.and_memo_context_collision_keys,
        }
    }

    fn depth_delta(
        &self,
        before: VCTStatsSnapshot,
        depth: i32,
        elapsed_us: u128,
        result: VCTResult,
    ) -> VCTDepthStats {
        VCTDepthStats {
            depth,
            elapsed_us,
            found: result.found,
            solved: result.solved,
            or_nodes: self.or_nodes.saturating_sub(before.or_nodes),
            and_nodes: self.and_nodes.saturating_sub(before.and_nodes),
            memo_exact_hits: self.memo_exact_hits.saturating_sub(before.memo_exact_hits),
            memo_shallow_found_hits: self
                .memo_shallow_found_hits
                .saturating_sub(before.memo_shallow_found_hits),
            memo_shallow_solved_hits: self
                .memo_shallow_solved_hits
                .saturating_sub(before.memo_shallow_solved_hits),
            attacks_generated: self
                .attacks_generated
                .saturating_sub(before.attacks_generated),
            defenses_generated: self
                .defenses_generated
                .saturating_sub(before.defenses_generated),
            max_attack_count: self.current_depth_max_attack_count,
            max_defense_count: self.current_depth_max_defense_count,
            and_memo_context_observations: self
                .and_memo_context_observations
                .saturating_sub(before.and_memo_context_observations),
            and_memo_context_collisions: self
                .and_memo_context_collisions
                .saturating_sub(before.and_memo_context_collisions),
            and_memo_context_collision_keys: self
                .and_memo_context_collision_keys
                .saturating_sub(before.and_memo_context_collision_keys),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct VCTSearcher {
    pub memo: HashMap<(i32, Side, i32, u64, u64), VctMemoEntry>,
    pub stats: VCTStats,
    pub memo_diagnostics_enabled: bool,
    pub strict_and_memo_key: bool,
    and_memo_context_signatures: HashMap<(Side, i32, u64), u64>,
    and_memo_context_collision_keys: HashSet<(Side, i32, u64)>,
}

impl VCTSearcher {
    pub fn search(&mut self, board: &Board, side: Side, depth: i32) -> VCTResult {
        self.search_for_rule(board, side, depth, RuleSet::Freestyle)
    }

    pub fn search_for_rule(
        &mut self,
        board: &Board,
        side: Side,
        depth: i32,
        rule: RuleSet,
    ) -> VCTResult {
        self.stats = VCTStats {
            depth_limit: depth.max(0),
            ..VCTStats::default()
        };
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
        let mut view = ThreatBoardView::from_board_with_rule(board.clone(), rule);
        self.memo.clear();
        self.and_memo_context_signatures.clear();
        self.and_memo_context_collision_keys.clear();
        let total_start = Instant::now();
        let mut result = VCTResult {
            move_: None,
            found: false,
            solved: false,
        };
        for d in 1..=depth {
            let before = self.stats.snapshot();
            self.stats.current_depth_max_attack_count = 0;
            self.stats.current_depth_max_defense_count = 0;
            let depth_start = Instant::now();
            result = self.or_node(&mut view, side, d);
            self.stats.depth_completed = d;
            self.stats.depth_stats.push(self.stats.depth_delta(
                before,
                d,
                depth_start.elapsed().as_micros(),
                result,
            ));
            if result.found || result.solved {
                self.stats.elapsed_us = total_start.elapsed().as_micros();
                return result;
            }
        }
        self.stats.elapsed_us = total_start.elapsed().as_micros();
        result
    }

    fn memo_lookup(
        &mut self,
        node: i32,
        attacker: Side,
        depth: i32,
        key: u64,
        context: u64,
    ) -> Option<VCTResult> {
        if let Some(result) = self
            .memo
            .get(&(node, attacker, depth, key, context))
            .map(|entry| entry.result)
        {
            self.stats.memo_exact_hits += 1;
            return Some(result);
        }
        for d in 1..depth {
            if let Some(result) = self
                .memo
                .get(&(node, attacker, d, key, context))
                .map(|entry| entry.result)
            {
                if result.found {
                    self.stats.memo_shallow_found_hits += 1;
                    return Some(result);
                }
                if result.solved {
                    self.stats.memo_shallow_solved_hits += 1;
                    return Some(result);
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
        context: u64,
        result: VCTResult,
    ) -> VCTResult {
        self.memo.insert(
            (node, attacker, depth, key, context),
            VctMemoEntry { depth, result },
        );
        result
    }

    fn or_node(&mut self, view: &mut ThreatBoardView, attacker: Side, depth: i32) -> VCTResult {
        self.stats.or_nodes += 1;
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
        if let Some(cached) = self.memo_lookup(OR_NODE, attacker, depth, key, 0) {
            return cached;
        }

        let attacks = view.collect_attack_moves(attacker);
        self.stats.attacks_generated += attacks.len();
        self.stats.max_attack_count = self.stats.max_attack_count.max(attacks.len());
        self.stats.current_depth_max_attack_count =
            self.stats.current_depth_max_attack_count.max(attacks.len());
        if attacks.is_empty() {
            return self.store(
                OR_NODE,
                attacker,
                depth,
                key,
                0,
                VCTResult {
                    move_: None,
                    found: false,
                    solved: true,
                },
            );
        }

        let mut solved = true;
        for attack in attacks {
            if attack.level >= ThreatLevel::A4
                && self.a4_attack_wins_immediately(view, attacker, &attack)
            {
                return self.store(
                    OR_NODE,
                    attacker,
                    depth,
                    key,
                    0,
                    VCTResult {
                        move_: Some(attack.move_),
                        found: true,
                        solved: true,
                    },
                );
            }

            view.play(attack.move_, attacker);
            let and_key = view.board.zobrist_key();
            let and_context = self.and_memo_context(&attack);
            self.observe_and_memo_context(attacker, depth, and_key, &attack);
            let and_result = if let Some(cached) =
                self.memo_lookup(AND_NODE, attacker, depth, and_key, and_context)
            {
                cached
            } else {
                self.and_node_for_attack(view, attacker, depth, &attack)
            };
            view.undo();

            if and_result.found {
                return self.store(
                    OR_NODE,
                    attacker,
                    depth,
                    key,
                    0,
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
            0,
            VCTResult {
                move_: None,
                found: false,
                solved,
            },
        )
    }

    /// WIN5 wins on the spot. An open or double four (A4) only wins immediately
    /// when the defender cannot answer with a five of their own: the defender
    /// moves next, so any four they already hold (that this attack does not
    /// block) completes first. Attacker moves never create defender fours, so
    /// this also covers the "defender had no four" case. When a counter-five
    /// survives, the attack must go through the AND node instead.
    fn a4_attack_wins_immediately(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        attack: &AttackMove,
    ) -> bool {
        if attack.level >= ThreatLevel::WIN5 {
            return true;
        }
        view.play(attack.move_, attacker);
        let defender_five = view.broken_four_point_for_side(-attacker).0;
        view.undo();
        defender_five.is_none()
    }

    fn observe_and_memo_context(
        &mut self,
        attacker: Side,
        current_depth: i32,
        board_key: u64,
        attack: &AttackMove,
    ) {
        if !self.memo_diagnostics_enabled {
            return;
        }
        self.stats.and_memo_context_observations += 1;
        let current_signature = attack_signature(attack);
        for observed_depth in 1..=current_depth {
            let old_key = (attacker, observed_depth, board_key);
            let Some(&observed_signature) = self.and_memo_context_signatures.get(&old_key) else {
                continue;
            };
            if observed_signature == current_signature {
                continue;
            }
            self.stats.and_memo_context_collisions += 1;
            if self.and_memo_context_collision_keys.insert(old_key) {
                self.stats.and_memo_context_collision_keys += 1;
            }
            if self.stats.and_memo_context_collision_samples.len() < MAX_AND_MEMO_COLLISION_SAMPLES
            {
                self.stats
                    .and_memo_context_collision_samples
                    .push(VCTAndMemoCollisionSample {
                        observed_depth,
                        current_depth,
                        board_key,
                        observed_signature,
                        current_signature,
                        attack_move: attack.move_,
                        attack_level: attack.level as u8,
                        defenses: attack.defenses.clone(),
                    });
            }
        }
        self.and_memo_context_signatures
            .entry((attacker, current_depth, board_key))
            .or_insert(current_signature);
    }

    fn and_memo_context(&self, attack: &AttackMove) -> u64 {
        if self.strict_and_memo_key {
            attack_signature(attack)
        } else {
            0
        }
    }

    fn and_node_for_attack(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        depth: i32,
        attack: &AttackMove,
    ) -> VCTResult {
        self.stats.and_nodes += 1;

        let key = view.board.zobrist_key();
        let context = self.and_memo_context(attack);
        if let Some(cached) = self.memo_lookup(AND_NODE, attacker, depth, key, context) {
            return cached;
        }

        let mut seen = std::collections::HashSet::new();
        let mut searched_any = false;
        let mut solved = true;

        let counter_wins = self.collect_counter_wins(view, attacker);
        let win_stage =
            self.search_defense_stage(view, attacker, depth, attack.level, counter_wins, &mut seen);
        if let Some(result) = win_stage.refutation {
            return self.store(AND_NODE, attacker, depth, key, context, result);
        }
        searched_any |= win_stage.searched;
        solved &= win_stage.solved;

        let forced_stage = self.search_defense_stage(
            view,
            attacker,
            depth,
            attack.level,
            attack.defenses.iter().copied(),
            &mut seen,
        );
        if let Some(result) = forced_stage.refutation {
            return self.store(AND_NODE, attacker, depth, key, context, result);
        }
        searched_any |= forced_stage.searched;
        solved &= forced_stage.solved;

        let counter_forcing = self.collect_counter_forcing_defenses(view, attacker, &seen);
        let counter_stage = self.search_defense_stage(
            view,
            attacker,
            depth,
            attack.level,
            counter_forcing,
            &mut seen,
        );
        if let Some(result) = counter_stage.refutation {
            return self.store(AND_NODE, attacker, depth, key, context, result);
        }
        searched_any |= counter_stage.searched;
        solved &= counter_stage.solved;

        self.store(
            AND_NODE,
            attacker,
            depth,
            key,
            context,
            VCTResult {
                move_: None,
                found: true,
                solved: solved || !searched_any,
            },
        )
    }

    fn search_defense_stage<I>(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        depth: i32,
        attack_level: ThreatLevel,
        defenses: I,
        seen: &mut std::collections::HashSet<Move>,
    ) -> DefenseStageResult
    where
        I: IntoIterator<Item = Move>,
    {
        let mut solved = true;
        let mut searched = false;
        for d_move in defenses {
            if !seen.insert(d_move) || !view.is_rule_legal(d_move, -attacker) {
                continue;
            }
            searched = true;
            self.stats.defenses_generated += 1;
            self.stats.max_defense_count = self.stats.max_defense_count.max(seen.len());
            self.stats.current_depth_max_defense_count =
                self.stats.current_depth_max_defense_count.max(seen.len());

            view.play(d_move, -attacker);
            let (dx, dy) = crate::board::move_to_xy(d_move).expect("move stays valid");
            // A defender five refutes any threat. A defender open four only
            // refutes an open-three-level attack: against a four (B4) the
            // attacker completes five first, so the reply is searched like any
            // other defense and the child OR node claims the win.
            if view.board.winner() == -attacker
                || (attack_level < ThreatLevel::B4 && view.has_winning_a4(dx, dy))
            {
                view.undo();
                return DefenseStageResult {
                    refutation: Some(VCTResult {
                        move_: None,
                        found: false,
                        solved: true,
                    }),
                    solved: true,
                    searched,
                };
            }

            let or_result = self.or_node(view, attacker, depth - 1);
            view.undo();

            if !or_result.found {
                return DefenseStageResult {
                    refutation: Some(VCTResult {
                        move_: None,
                        found: false,
                        solved: or_result.solved,
                    }),
                    solved: or_result.solved,
                    searched,
                };
            }
            if !or_result.solved {
                solved = false;
            }
        }
        DefenseStageResult {
            refutation: None,
            solved,
            searched,
        }
    }

    fn collect_counter_wins(&mut self, view: &mut ThreatBoardView, attacker: Side) -> Vec<Move> {
        let defender = -attacker;
        let mut counter_wins = Vec::new();

        for m in view.threat_moves(defender) {
            if !view.is_rule_legal(m, defender) {
                continue;
            }
            view.play(m, defender);
            let (dx, dy) = crate::board::move_to_xy(m).expect("move stays valid");
            if view.board.winner() == defender || view.has_winning_a4(dx, dy) {
                counter_wins.push(m);
            }
            view.undo();
        }

        counter_wins
    }

    fn collect_counter_forcing_defenses(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        seen: &std::collections::HashSet<Move>,
    ) -> Vec<Move> {
        let defender = -attacker;
        let mut counter_b4 = Vec::new();
        let mut counter_a3 = Vec::new();

        for m in view.threat_moves(defender) {
            if seen.contains(&m) || !view.is_rule_legal(m, defender) {
                continue;
            }
            view.play(m, defender);
            let (dx, dy) = crate::board::move_to_xy(m).expect("move stays valid");
            if view.board.winner() != defender && !view.has_winning_a4(dx, dy) {
                let b4 = view.b4_count(dx, dy);
                if b4 >= 1 {
                    counter_b4.push(m);
                } else if view.a3r_count(dx, dy) >= 1 {
                    counter_a3.push(m);
                }
            }
            view.undo();
        }

        counter_b4.into_iter().chain(counter_a3).collect()
    }
}

fn attack_signature(attack: &AttackMove) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    mix_signature(&mut hash, u64::from(attack.move_));
    mix_signature(&mut hash, u64::from(attack.level as u8));
    mix_signature(&mut hash, attack.defenses.len() as u64);
    for &defense in &attack.defenses {
        mix_signature(&mut hash, u64::from(defense));
    }
    hash
}

fn mix_signature(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(0x1000_0000_01b3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::constants::{BLACK, WHITE};

    // Black has an open three (6..8,7): extending it makes an open four (A4).
    // But white already has a live four on the diagonal — after any black
    // non-winning move white completes five first, so black has no VCT. The
    // OR node must not treat the A4 attack as an unconditional instant win.
    #[test]
    fn vct_open_four_is_not_instant_win_when_defender_answers_with_five() {
        let mut board = Board::new();
        for (x, y, side) in [
            (6, 7, BLACK),
            (7, 7, BLACK),
            (8, 7, BLACK),
            (1, 1, WHITE),
            (2, 2, WHITE),
            (3, 3, WHITE),
            (4, 4, WHITE),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }

        let result = VCTSearcher::default().search(&board, BLACK, 4);
        assert!(
            !result.found,
            "black's open four loses to white's immediate five"
        );
    }

    // Black wins by force: (8,7) makes a four (5..8,7) and at the same time a
    // column three (8,5)(8,6)(8,7). White's only block (9,7) lets black play
    // (8,8) for an open four. White's open three at (1,1)..(3,3) can answer
    // with an open four, but never a five in time — it must not be accepted
    // as a refutation of the four attack.
    #[test]
    fn vct_four_attack_is_not_refuted_by_defender_counter_open_four() {
        let mut board = Board::new();
        for (x, y, side) in [
            (5, 7, BLACK),
            (6, 7, BLACK),
            (7, 7, BLACK),
            (8, 5, BLACK),
            (8, 6, BLACK),
            (4, 7, WHITE),
            (1, 1, WHITE),
            (2, 2, WHITE),
            (3, 3, WHITE),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }

        let result = VCTSearcher::default().search(&board, BLACK, 4);
        assert!(
            result.found,
            "the forcing four -> open four chain is a real VCT"
        );
        assert_eq!(result.move_, Some(xy_to_move(8, 7).unwrap()));
    }

    #[test]
    fn and_memo_context_diagnostic_detects_same_key_different_attack() {
        let mut searcher = VCTSearcher {
            memo_diagnostics_enabled: true,
            ..VCTSearcher::default()
        };
        let first = AttackMove {
            move_: 10,
            level: ThreatLevel::B4,
            defenses: vec![11],
        };
        let second = AttackMove {
            move_: 12,
            level: ThreatLevel::A3,
            defenses: vec![13, 14],
        };

        searcher.observe_and_memo_context(1, 3, 1234, &first);
        searcher.observe_and_memo_context(1, 3, 1234, &second);

        assert_eq!(searcher.stats.and_memo_context_observations, 2);
        assert_eq!(searcher.stats.and_memo_context_collisions, 1);
        assert_eq!(searcher.stats.and_memo_context_collision_keys, 1);
        assert_eq!(searcher.stats.and_memo_context_collision_samples.len(), 1);
    }

    #[test]
    fn and_memo_context_diagnostic_detects_shallow_key_reuse_risk() {
        let mut searcher = VCTSearcher {
            memo_diagnostics_enabled: true,
            ..VCTSearcher::default()
        };
        let shallow = AttackMove {
            move_: 10,
            level: ThreatLevel::B4,
            defenses: vec![11],
        };
        let deeper = AttackMove {
            move_: 10,
            level: ThreatLevel::B4,
            defenses: vec![11, 12],
        };

        searcher.observe_and_memo_context(1, 2, 5678, &shallow);
        searcher.observe_and_memo_context(1, 4, 5678, &deeper);

        assert_eq!(searcher.stats.and_memo_context_observations, 2);
        assert_eq!(searcher.stats.and_memo_context_collisions, 1);
        assert_eq!(searcher.stats.and_memo_context_collision_keys, 1);
        assert_eq!(
            searcher.stats.and_memo_context_collision_samples[0].observed_depth,
            2
        );
        assert_eq!(
            searcher.stats.and_memo_context_collision_samples[0].current_depth,
            4
        );
    }
}
