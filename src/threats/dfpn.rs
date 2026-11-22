//! Depth-First Proof-Number (df-pn) VCT solver backed by `Pattern4State` and a
//! fixed-size Zobrist transposition table.
//!
//! Unlike depth-limited IDDFS VCT, df-pn concentrates search effort on narrow
//! forcing chains (e.g. 1–2 defense B4/A3 sequences up to 20+ plies deep) while
//! automatically steering away from wide multi-defense open-three trees, all
//! under a strict node/time budget and reusing `ThreatBoardView`'s verified
//! Renju-safe attack/defense semantics.

use std::time::Instant;

use crate::board::Board;
use crate::constants::BLACK;
use crate::rules::RuleSet;
use crate::search::budget::MoveBudget;
use crate::threats::pattern4::{col_major_idx, P1};
use crate::threats::threat_board::{LegalityCache, ThreatBoardView};
use crate::threats::types::{AttackMove, ThreatLevel};
use crate::threats::vct::{attack_signature, MoveSet225, VCTResult, VCTSearcher};
use crate::types::{Move, Side};

pub const DFPN_INF: u32 = 100_000_000;
const DFPN_DEPTH_CAP_PN: u32 = 50_000;
const DFPN_NO_MOVE: Move = u16::MAX;

const DFPN_TT_BITS: usize = 16;
const DFPN_TT_BUCKETS: usize = 1 << DFPN_TT_BITS;
const DFPN_TT_MASK: usize = DFPN_TT_BUCKETS - 1;

const OR_SALT: u64 = 0x243F_6A88_85A3_08D3;
const AND_SALT: u64 = 0x1319_8A2E_0370_7344;
const BLACK_SALT: u64 = 0xA409_3822_299F_31D0;
const WHITE_SALT: u64 = 0x082E_FA98_EC4E_6C89;

pub const DEFAULT_DFPN_MAX_PLY: i32 = 22;
pub const DEFAULT_DFPN_NODE_LIMIT: usize = 120_000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DfpnStats {
    pub or_nodes: usize,
    pub and_nodes: usize,
    pub tt_hits: usize,
    pub elapsed_ms: f64,
    pub aborted: bool,
}

pub type DfpnResult = VCTResult;

#[derive(Clone, Copy, Debug, Default)]
struct DfpnTtEntry {
    key: u64,
    generation: u32,
    pn: u32,
    dn: u32,
    best_move: Move,
    _pad: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct DfpnTtBucket {
    entries: [DfpnTtEntry; 2],
}

#[derive(Clone, Debug)]
pub struct DfpnSearcher {
    pub(crate) budget: MoveBudget,
    pub max_ply: i32,
    pub node_limit: usize,
    pub stats: DfpnStats,
    tt: Option<Box<[DfpnTtBucket]>>,
    tt_generation: u32,
    legality_cache: Option<Box<LegalityCache>>,
    aborted: bool,
}

impl Default for DfpnSearcher {
    fn default() -> Self {
        Self {
            budget: MoveBudget::default(),
            max_ply: DEFAULT_DFPN_MAX_PLY,
            node_limit: DEFAULT_DFPN_NODE_LIMIT,
            stats: DfpnStats::default(),
            tt: None,
            tt_generation: 0,
            legality_cache: None,
            aborted: false,
        }
    }
}

#[inline(always)]
fn side_salt(attacker: Side) -> u64 {
    if attacker == BLACK {
        BLACK_SALT
    } else {
        WHITE_SALT
    }
}

#[inline(always)]
fn or_key(board: &Board, attacker: Side) -> u64 {
    board.zobrist_key() ^ OR_SALT ^ side_salt(attacker)
}

#[inline(always)]
fn and_key(board: &Board, attacker: Side, attack: &AttackMove) -> u64 {
    board.zobrist_key() ^ AND_SALT ^ side_salt(attacker) ^ attack_signature(attack)
}

#[inline(always)]
fn add_pn(a: u32, b: u32) -> u32 {
    a.saturating_add(b).min(DFPN_INF)
}

#[inline(always)]
fn epsilon_threshold(second_best: u32, parent_limit: u32) -> u32 {
    if second_best >= DFPN_INF {
        return parent_limit;
    }
    let eps = second_best
        .saturating_add(1)
        .saturating_add(second_best >> 3);
    parent_limit.min(eps)
}

impl DfpnSearcher {
    fn ensure_tt(&mut self) {
        if self.tt.is_none() {
            self.tt = Some(vec![DfpnTtBucket::default(); DFPN_TT_BUCKETS].into_boxed_slice());
            self.tt_generation = 1;
        }
    }

    fn next_generation(&mut self) {
        self.ensure_tt();
        self.tt_generation = self.tt_generation.wrapping_add(1);
        if self.tt_generation == 0 {
            if let Some(tt) = self.tt.as_deref_mut() {
                tt.fill(DfpnTtBucket::default());
            }
            self.tt_generation = 1;
        }
    }

    #[inline(always)]
    fn tt_lookup(
        &mut self,
        key: u64,
        default_pn: u32,
        default_dn: u32,
    ) -> (u32, u32, Option<Move>) {
        if let Some(tt) = self.tt.as_deref() {
            let bucket = &tt[(key as usize) & DFPN_TT_MASK];
            for entry in &bucket.entries {
                if entry.generation == self.tt_generation && entry.key == key {
                    self.stats.tt_hits += 1;
                    let mv = (entry.best_move != DFPN_NO_MOVE).then_some(entry.best_move);
                    return (entry.pn, entry.dn, mv);
                }
            }
        }
        (default_pn, default_dn, None)
    }

    #[inline(always)]
    fn tt_store(&mut self, key: u64, pn: u32, dn: u32, best_move: Option<Move>) {
        let Some(tt) = self.tt.as_deref_mut() else {
            return;
        };
        let bucket = &mut tt[(key as usize) & DFPN_TT_MASK];
        let gen = self.tt_generation;
        // 1. Update matching key in place.
        for entry in &mut bucket.entries {
            if entry.generation == gen && entry.key == key {
                entry.pn = pn;
                entry.dn = dn;
                entry.best_move = best_move.unwrap_or(DFPN_NO_MOVE);
                return;
            }
        }
        // 2. Fill an empty / stale-generation slot if available.
        for entry in &mut bucket.entries {
            if entry.generation != gen {
                entry.key = key;
                entry.generation = gen;
                entry.pn = pn;
                entry.dn = dn;
                entry.best_move = best_move.unwrap_or(DFPN_NO_MOVE);
                return;
            }
        }
        // 3. Always-replace one slot so df-pn progress is never dropped; keep
        //    a solved entry in slot 0 when possible by replacing slot 1.
        let victim = if bucket.entries[0].pn == 0 || bucket.entries[0].dn == 0 {
            1
        } else {
            0
        };
        let slot = &mut bucket.entries[victim];
        slot.key = key;
        slot.generation = gen;
        slot.pn = pn;
        slot.dn = dn;
        slot.best_move = best_move.unwrap_or(DFPN_NO_MOVE);
    }

    #[inline(always)]
    fn enter_node(&mut self) -> bool {
        if self.aborted {
            return false;
        }
        let total = self.stats.or_nodes + self.stats.and_nodes;
        if self.node_limit > 0 && total >= self.node_limit {
            self.aborted = true;
            return false;
        }
        if !self.budget.enter() {
            self.aborted = true;
            return false;
        }
        true
    }

    pub fn search_for_rule(
        &mut self,
        board: &Board,
        side: Side,
        max_ply: i32,
        rule: RuleSet,
    ) -> DfpnResult {
        self.stats = DfpnStats::default();
        self.aborted = false;
        if max_ply <= 0 {
            return VCTResult::default();
        }
        if board.winner() != 0 {
            return VCTResult {
                move_: None,
                found: false,
                solved: true,
            };
        }
        self.max_ply = max_ply;
        self.next_generation();

        let cache = (rule == RuleSet::Renju)
            .then(|| self.legality_cache.take())
            .flatten();
        let mut view = ThreatBoardView::from_board_with_rule_and_cache(board.clone(), rule, cache);

        let start = Instant::now();
        let (pn, dn, best_move) = self.mid_or(&mut view, side, 0, DFPN_INF, DFPN_INF);
        self.stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.stats.aborted = self.aborted;

        if let Some(cache) = view.take_legality_cache() {
            self.legality_cache = Some(cache);
        }

        if pn == 0 && !self.aborted {
            VCTResult {
                move_: best_move,
                found: true,
                solved: true,
            }
        } else {
            VCTResult {
                move_: None,
                found: false,
                solved: dn == 0 && !self.aborted,
            }
        }
    }

    fn mid_or(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        ply: i32,
        th_pn: u32,
        th_dn: u32,
    ) -> (u32, u32, Option<Move>) {
        if !self.enter_node() {
            return (1, 1, None);
        }
        self.stats.or_nodes += 1;

        if view.board.winner() == attacker {
            return (0, DFPN_INF, None);
        }
        if view.board.winner() == -attacker {
            return (DFPN_INF, 0, None);
        }

        let key = or_key(&view.board, attacker);
        let (cached_pn, cached_dn, cached_move) = self.tt_lookup(key, 1, 1);
        if cached_pn == 0 || cached_dn == 0 || cached_pn >= th_pn || cached_dn >= th_dn {
            return (cached_pn, cached_dn, cached_move);
        }

        // Ply cap: check O(1) instant win (WIN5 or unblocked A4); otherwise
        // store a large finite proof number (not dn=0!) so depth truncation is
        // never mis-cached as a mathematical disproof, yet stops re-expansion.
        if ply >= self.max_ply {
            for idx in view.pat4.win5_bits(attacker).iter_indices() {
                let mv = crate::threats::pattern4::idx_to_move(idx);
                if view.fast_rule_legal_empty(mv, idx, attacker) {
                    self.tt_store(key, 0, DFPN_INF, Some(mv));
                    return (0, DFPN_INF, Some(mv));
                }
            }
            if view.pat4.win5_bits(-attacker).is_empty() {
                for idx in view.pat4.a4_bits(attacker).iter_indices() {
                    let mv = crate::threats::pattern4::idx_to_move(idx);
                    if view.fast_rule_legal_empty(mv, idx, attacker) {
                        self.tt_store(key, 0, DFPN_INF, Some(mv));
                        return (0, DFPN_INF, Some(mv));
                    }
                }
            }
            self.tt_store(key, DFPN_DEPTH_CAP_PN, 1, None);
            return (DFPN_DEPTH_CAP_PN, 1, None);
        }

        let attacks = view.collect_attack_moves(attacker);
        if attacks.is_empty() {
            self.tt_store(key, DFPN_INF, 0, None);
            return (DFPN_INF, 0, None);
        }

        for attack in &attacks {
            if attack.level >= ThreatLevel::A4
                && VCTSearcher::a4_attack_wins_immediately_on_view(view, attacker, attack)
            {
                self.tt_store(key, 0, DFPN_INF, Some(attack.move_));
                return (0, DFPN_INF, Some(attack.move_));
            }
        }

        // Precompute child AND keys once per OR expansion.
        let mut child_keys = Vec::with_capacity(attacks.len());
        let mut child_init_pn = Vec::with_capacity(attacks.len());
        let mut child_init_dn = Vec::with_capacity(attacks.len());
        for attack in &attacks {
            let ckey = view.board.zobrist_key_after(attack.move_, attacker)
                ^ AND_SALT
                ^ side_salt(attacker)
                ^ attack_signature(attack);
            child_keys.push(ckey);
            // Bias initial proof numbers toward narrow B4 attacks and lethal
            // Double-A3 forks (pn=1), while scaling single-A3 attacks with ply
            // (4 + ply) so Double-A3's 4 defenses are verified in a single pass
            // and shallow wins are proved before diving into deep A3 trees.
            if attack.level >= ThreatLevel::B4 {
                child_init_pn.push(1);
                child_init_dn.push(2);
            } else {
                let (ax, ay) = crate::board::move_to_xy(attack.move_).expect("attack stays valid");
                let aidx = col_major_idx(ax, ay);
                let mut a3_dirs = 0_u8;
                for dir in 0..4 {
                    if view.pat4.p1_code(attacker, aidx, dir) == P1::A3 {
                        a3_dirs += 1;
                    }
                }
                if a3_dirs >= 2 {
                    child_init_pn.push(1);
                    child_init_dn.push(4);
                } else {
                    let a3_pn = 9 + (ply as u32);
                    child_init_pn.push(a3_pn);
                    child_init_dn.push(1);
                }
            }
        }

        loop {
            let mut min_pn = DFPN_INF;
            let mut second_pn = DFPN_INF;
            let mut sum_dn = 0_u32;
            let mut best_idx = 0_usize;
            let mut best_dn = 0_u32;

            for i in 0..attacks.len() {
                let (c_pn, c_dn, _) =
                    self.tt_lookup(child_keys[i], child_init_pn[i], child_init_dn[i]);
                sum_dn = add_pn(sum_dn, c_dn);
                if c_pn < min_pn {
                    second_pn = min_pn;
                    min_pn = c_pn;
                    best_idx = i;
                    best_dn = c_dn;
                } else if c_pn < second_pn {
                    second_pn = c_pn;
                }
            }

            if min_pn == 0 {
                let win_mv = attacks[best_idx].move_;
                self.tt_store(key, 0, DFPN_INF, Some(win_mv));
                return (0, DFPN_INF, Some(win_mv));
            }
            if min_pn >= th_pn || sum_dn >= th_dn || self.aborted {
                self.tt_store(key, min_pn, sum_dn, None);
                return (min_pn, sum_dn, None);
            }

            let child_th_pn = epsilon_threshold(second_pn, th_pn);
            let child_th_dn = add_pn(th_dn.saturating_sub(sum_dn), best_dn);

            let attack = &attacks[best_idx];
            view.play(attack.move_, attacker);
            let _ = self.mid_and(view, attacker, ply + 1, attack, child_th_pn, child_th_dn);
            view.undo();

            if self.aborted {
                return (min_pn, sum_dn, None);
            }
        }
    }

    fn mid_and(
        &mut self,
        view: &mut ThreatBoardView,
        attacker: Side,
        ply: i32,
        attack: &AttackMove,
        th_pn: u32,
        th_dn: u32,
    ) -> (u32, u32) {
        if !self.enter_node() {
            return (1, 1);
        }
        self.stats.and_nodes += 1;

        let key = and_key(&view.board, attacker, attack);
        let (cached_pn, cached_dn, _) = self.tt_lookup(key, 1, 1);
        if cached_pn == 0 || cached_dn == 0 || cached_pn >= th_pn || cached_dn >= th_dn {
            return (cached_pn, cached_dn);
        }

        let defender = -attacker;
        let mut seen = MoveSet225::default();
        let mut defenses: Vec<Move> = Vec::with_capacity(16);

        let check_and_push = |d_move: Move,
                              seen: &mut MoveSet225,
                              defenses: &mut Vec<Move>,
                              view: &ThreatBoardView|
         -> bool {
            if !seen.insert(d_move) {
                return false;
            }
            let (dx, dy) = crate::board::move_to_xy(d_move).expect("defense stays valid");
            let didx = col_major_idx(dx, dy);
            let dpat = view.pat4.cell_pattern(defender, didx);
            if dpat.is_win5() || (attack.level < ThreatLevel::B4 && dpat.has_straight_a4()) {
                return true;
            }
            defenses.push(d_move);
            false
        };

        if let Some(d_move) = view.first_counter_win(defender, attack.level < ThreatLevel::B4) {
            if check_and_push(d_move, &mut seen, &mut defenses, view) {
                self.tt_store(key, DFPN_INF, 0, None);
                return (DFPN_INF, 0);
            }
        }
        for &d_move in &attack.defenses {
            if check_and_push(d_move, &mut seen, &mut defenses, view) {
                self.tt_store(key, DFPN_INF, 0, None);
                return (DFPN_INF, 0);
            }
        }
        if attack.level < ThreatLevel::B4 {
            let counter_forcing =
                VCTSearcher::collect_counter_forcing_defenses_on_view(view, attacker, &seen);
            for d_move in counter_forcing {
                if check_and_push(d_move, &mut seen, &mut defenses, view) {
                    self.tt_store(key, DFPN_INF, 0, None);
                    return (DFPN_INF, 0);
                }
            }
        }

        if defenses.is_empty() {
            // No legal defense survives (e.g., all direct blocks are Renju
            // forbidden and defender has no counter-win/counter-threat).
            self.tt_store(key, 0, DFPN_INF, None);
            return (0, DFPN_INF);
        }

        let mut child_keys = Vec::with_capacity(defenses.len());
        for &d_move in &defenses {
            let ckey =
                view.board.zobrist_key_after(d_move, defender) ^ OR_SALT ^ side_salt(attacker);
            child_keys.push(ckey);
        }

        loop {
            let mut sum_pn = 0_u32;
            let mut min_dn = DFPN_INF;
            let mut second_dn = DFPN_INF;
            let mut best_idx = 0_usize;
            let mut best_pn = 0_u32;

            for i in 0..defenses.len() {
                let (c_pn, c_dn, _) = self.tt_lookup(child_keys[i], 1, 1);
                sum_pn = add_pn(sum_pn, c_pn);
                if c_dn < min_dn {
                    second_dn = min_dn;
                    min_dn = c_dn;
                    best_idx = i;
                    best_pn = c_pn;
                } else if c_dn < second_dn {
                    second_dn = c_dn;
                }
            }

            if min_dn == 0 {
                self.tt_store(key, DFPN_INF, 0, None);
                return (DFPN_INF, 0);
            }
            if sum_pn == 0 {
                self.tt_store(key, 0, DFPN_INF, None);
                return (0, DFPN_INF);
            }
            if sum_pn >= th_pn || min_dn >= th_dn || self.aborted {
                self.tt_store(key, sum_pn, min_dn, None);
                return (sum_pn, min_dn);
            }

            let child_th_pn = add_pn(th_pn.saturating_sub(sum_pn), best_pn);
            let child_th_dn = epsilon_threshold(second_dn, th_dn);

            let d_move = defenses[best_idx];
            view.play(d_move, defender);
            let _ = self.mid_or(view, attacker, ply + 1, child_th_pn, child_th_dn);
            view.undo();

            if self.aborted {
                return (sum_pn, min_dn);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::constants::{BLACK, WHITE};

    #[test]
    fn dfpn_rejects_open_four_when_defender_answers_with_five() {
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
        let result = DfpnSearcher::default().search_for_rule(&board, BLACK, 12, RuleSet::Freestyle);
        assert!(!result.found);
    }

    #[test]
    fn dfpn_rejects_renju_case17_quiet_endpoint_false_positive() {
        let raw = include_str!("../../cases/renju/vct6_false_positive_case17_after_ply40.json");
        #[derive(serde::Deserialize)]
        struct Case {
            moves: Vec<[usize; 2]>,
        }
        let case: Case = serde_json::from_str(raw).unwrap();
        let mut board = Board::new();
        for [x, y] in case.moves {
            board
                .play_for_rule(xy_to_move(x, y).unwrap(), None, RuleSet::Renju)
                .unwrap();
        }
        let side = board.side_to_move();
        let mut dfpn = DfpnSearcher::default();
        let res = dfpn.search_for_rule(&board, side, 16, RuleSet::Renju);
        assert_ne!(
            res.move_,
            Some(xy_to_move(8, 8).unwrap()),
            "df-pn must not accept the refuted (8,8) A3 false positive"
        );
    }

    #[test]
    fn dfpn_solves_dual_a3_diamond_at_center() {
        let mut board = Board::new();
        for (x, y, side) in [
            (6, 7, 1),
            (0, 0, -1),
            (8, 7, 1),
            (1, 3, -1),
            (7, 6, 1),
            (3, 1, -1),
            (7, 8, 1),
            (14, 14, -1),
        ] {
            board.play(xy_to_move(x, y).unwrap(), Some(side)).unwrap();
        }
        let mut dfpn = DfpnSearcher::default();
        let result = dfpn.search_for_rule(&board, 1, 18, RuleSet::Freestyle);
        eprintln!(
            "dual_a3_diamond: result={:?}, move={:?}, stats={:?}",
            result,
            result.move_.and_then(|m| crate::board::move_to_xy(m).ok()),
            dfpn.stats
        );
        assert!(result.found);
        assert_eq!(
            result.move_.and_then(|m| crate::board::move_to_xy(m).ok()),
            Some((7, 7))
        );
    }
}
