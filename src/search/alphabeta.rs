//! Alpha-beta search implementation for the classic mainline.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, LazyLock,
};
use std::time::Instant;

use crate::board::{move_to_xy, Board};
use crate::config::EngineConfig;
use crate::constants::{
    BLACK, BOARD_AREA, BOARD_SIZE, EMPTY, HASHF_ALPHA, HASHF_BETA, HASHF_EMPTY, HASHF_EXACT, INF,
    WIN,
};
use crate::eval::{
    attack_level, evaluate_board, value_wide_compute_for_rule, EvalCaches, EvalSnapshot,
};
use crate::nnue::{load_mix9svq_evaluator_for_rule, Mix9svqEvaluator};
use crate::rules::RuleSet;
use crate::search::abdada::{AbdadaContext, ABDADA_MIN_DEPTH};
use crate::search::movegen::{
    generate_candidates_with_coverage_scratch, is_rule_legal_for_movegen, Candidate,
    CoverageTracker, MoveAnalysis,
};
use crate::search::ordering::{
    order_candidates_owned, order_candidates_primary_owned, order_candidates_primary_slice,
    resolve_candidate_tie_group,
};
use crate::search::tt::{TTEntry4, TTTableView, TTTableView4, TT_VALUE_NONE};
use crate::search::{order_candidates_root_classic, TTEntry, TranspositionTable};
use crate::threats::VCFSearcher;
use crate::types::{Move, Side};

const SUBPLY_TT_SCALE_F64: f64 = 64.0;
const SUBPLY_TT_DEPTH_CAP: i32 = 3200;
const SUBPLY_TT_WIN_BONUS: i32 = 640;
const INTEGER_TT_DEPTH_CAP: i32 = crate::search::tt::DEPTH_MAX - 10;
const QVCF_MAX_STEPS: i32 = 8;
const QVCF_MAX_PLY: i32 = 96;
/// TT4 depth bands in NNUE subply mode:
/// - `0`: static-eval-only (`HASHF_EMPTY`)
/// - `1`: QVCF combo-only (`qdepth <= -2`)
/// - `2`: QVCF full local B4 (`qdepth >= -1`)
/// - `>= 3`: main alpha-beta search (`depth > 0.0`, shifted by `NNUE_MAIN_TT_DEPTH_OFFSET`)
const QVCF_TT_DEPTH_COMB: i32 = 1;
const QVCF_TT_DEPTH_FULL: i32 = 2;
const NNUE_MAIN_TT_DEPTH_OFFSET: i32 = 3;
/// Minimum `value_cache` bucket for a combo B4 (`B4 + L2` or stronger: `L4S` top1=10, top2>=6 -> bucket >= 62).
const QVCF_COMB_MIN_BUCKET: i32 = 62;

/// Precomputed Rapfi-calibrated LMR log table:
/// index 0 = Freestyle (`Factor = 0.95`), index 1 = Renju (`Factor = 0.83`).
static LMR_LUT: LazyLock<[[f64; 64]; 2]> = LazyLock::new(|| {
    let factors = [0.95_f64, 0.83_f64];
    let mut table = [[0.0_f64; 64]; 2];
    for (r, &factor) in factors.iter().enumerate() {
        for i in 1..64 {
            table[r][i] = factor * (i as f64).ln();
        }
    }
    table
});

/// Deterministic per-square TT key salt used when `nonlocal_b4_dropped` is true
/// in QVCF so history-dependent fail-low/exact bounds stay cached for repeated
/// visits with the same `self_last` without ever polluting pure `zobrist_key`.
#[inline(always)]
fn qvcf_self_last_salt(last_move: usize) -> u64 {
    let mut z = (last_move as u64)
        .wrapping_add(1)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ 0xBF58_476D_1CE4_E5B9;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline(always)]
fn update_quiet_history(entry: &mut i32, bonus: i32) {
    let clamped = bonus.clamp(-400, 400);
    *entry += clamped - (*entry * clamped.abs()) / 4096;
}

/// Precomputed `RANGE_SQUARE2_LINE4` neighborhood bitboards for each board square
/// (all cells within Chebyshev distance <= 2 plus distance 3..=4 along the 4 lines).
static QVCF_NEIGHBOR_MASKS: LazyLock<[[u64; 4]; BOARD_AREA]> = LazyLock::new(|| {
    let mut table = [[0_u64; 4]; BOARD_AREA];
    for idx in 0..BOARD_AREA {
        let cx = (idx % BOARD_SIZE) as isize;
        let cy = (idx / BOARD_SIZE) as isize;
        let mut mask = [0_u64; 4];
        for dy in -2_isize..=2 {
            for dx in -2_isize..=2 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = cx + dx;
                let ny = cy + dy;
                if (0..BOARD_SIZE as isize).contains(&nx) && (0..BOARD_SIZE as isize).contains(&ny)
                {
                    let nidx = ny as usize * BOARD_SIZE + nx as usize;
                    mask[nidx >> 6] |= 1_u64 << (nidx & 63);
                }
            }
        }
        for &(dx, dy) in &[
            (1_isize, 0_isize),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
        ] {
            for dist in 3_isize..=4 {
                let nx = cx + dx * dist;
                let ny = cy + dy * dist;
                if (0..BOARD_SIZE as isize).contains(&nx) && (0..BOARD_SIZE as isize).contains(&ny)
                {
                    let nidx = ny as usize * BOARD_SIZE + nx as usize;
                    mask[nidx >> 6] |= 1_u64 << (nidx & 63);
                }
            }
        }
        table[idx] = mask;
    }
    table
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootCandidateProfile {
    pub index: usize,
    pub move_: Move,
    pub order_score: i32,
    pub self_attack: i32,
    pub opp_attack: i32,
    pub depthdown_milli: i32,
    pub atdown: i32,
    pub attempt_depth_milli: i32,
    pub score: i32,
    pub nodes: usize,
    pub elapsed_us: u128,
    pub alpha_before: i32,
    pub alpha_after: i32,
    pub beta: i32,
    pub reason: &'static str,
    pub zero_window_nodes: usize,
    pub zero_window_elapsed_us: u128,
    pub full_window_nodes: usize,
    pub full_window_elapsed_us: u128,
    pub pvs_research: bool,
}

#[derive(Clone, Debug)]
pub struct SearchStats {
    pub nodes: usize,
    pub leaf_nodes: usize,
    pub tt_hits: usize,
    pub cutoffs: usize,
    pub stop: bool,
    pub node_limit: Option<usize>,
    pub deadline: Option<Instant>,
    pub time_check_mask: usize,
    pub root_profile: bool,
    pub root_candidates: Vec<RootCandidateProfile>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub tt_bestmove_current_generation: usize,
    pub tt_bestmove_old_generation: usize,
}

impl Default for SearchStats {
    fn default() -> Self {
        Self {
            nodes: 0,
            leaf_nodes: 0,
            tt_hits: 0,
            cutoffs: 0,
            stop: false,
            node_limit: None,
            deadline: None,
            time_check_mask: 0xFF,
            root_profile: false,
            root_candidates: Vec::new(),
            cancel: None,
            tt_bestmove_current_generation: 0,
            tt_bestmove_old_generation: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SearchOptions<'a> {
    pub opo: usize,
    pub ply: i32,
    pub root: bool,
    pub root_allowed_moves: Option<&'a HashSet<Move>>,
    pub root_preferred_move: Option<Move>,
    pub downf: i32,
    pub root_depth: Option<f64>,
    pub priority_base: Option<i32>,
    pub cut_node: bool,
    pub skip_move: Option<Move>,
}

pub fn terminal_score(board: &Board, side: Side, ply: i32) -> Option<i32> {
    if board.winner() == 0 {
        return board.is_draw().then_some(0);
    }
    if board.winner() == side {
        return Some(INF - ply);
    }
    Some(-INF + ply)
}

#[inline]
fn score_to_tt(score: i32, ply: i32) -> i32 {
    if score >= WIN {
        score + ply
    } else if score <= -WIN {
        score - ply
    } else {
        score
    }
}

#[inline]
fn score_from_tt(score: i32, ply: i32) -> i32 {
    if score >= WIN {
        score - ply
    } else if score <= -WIN {
        score + ply
    } else {
        score
    }
}

#[inline]
fn shift_bound_with_atdown(bound: i32, atdown: i32) -> i32 {
    if bound >= WIN || bound <= -WIN {
        bound
    } else {
        (bound + atdown).clamp(-WIN + 1, WIN - 1)
    }
}

#[inline]
fn score_from_child(child_score: i32, atdown: i32) -> i32 {
    if child_score >= WIN || child_score <= -WIN {
        -child_score
    } else {
        (-atdown - child_score).clamp(-WIN + 1, WIN - 1)
    }
}

pub fn compute_corner_state(board: &Board) -> (bool, i32) {
    let mut half = 0_i32;
    for played in board.move_history() {
        let (mx, my) = move_to_xy(played.move_).expect("played move is in range");
        let h = mx
            .min(my)
            .min(board.size() - 1 - mx)
            .min(board.size() - 1 - my);
        if h <= 1 {
            return (true, half);
        }
        if h == 2 {
            half += 1;
            if half >= 2 {
                return (true, half);
            }
        }
    }
    (false, half)
}

pub fn rootbonus(board: &Board, x: usize, y: usize, is_corner: bool) -> i32 {
    let height = x.min(y).min(board.size() - 1 - x).min(board.size() - 1 - y);
    if is_corner {
        let mut bonus = 0.0_f64;
        let height_score = [4.0, 3.0, 2.0, 1.0];
        if height <= 3 {
            bonus += height_score[height];
        }
        let countall_list = [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut countall = 0_usize;
        for xx in x.saturating_sub(1)..=(x + 1).min(board.size() - 1) {
            for yy in y.saturating_sub(1)..=(y + 1).min(board.size() - 1) {
                if board.at(xx, yy).expect("in-bounds point stays valid") != 0 {
                    countall += 1;
                }
            }
        }
        let x_edge = usize::from(x == 0) + usize::from(x + 1 >= board.size());
        let y_edge = usize::from(y == 0) + usize::from(y + 1 >= board.size());
        countall += 3 * x_edge + 3 * y_edge - x_edge * y_edge;
        bonus += countall_list[countall.min(countall_list.len() - 1)] * 0.7;
        return bonus.round() as i32;
    }

    if height <= 3 {
        return [8, 4, 2, 1][height];
    }
    0
}

#[derive(Clone, Debug)]
pub struct AlphaBetaSearcher {
    pub config: EngineConfig,
    pub tt: TranspositionTable,
    pub vcf: VCFSearcher,
    tt_generation: u16,
    movegen_scratch: MoveAnalysis,
    nnue: Option<Mix9svqEvaluator>,
    static_eval_stack: [i32; 128],
    quiet_history: [[i32; BOARD_AREA]; 2],
}

pub(crate) fn load_nnue_for_config(config: &EngineConfig) -> Option<Mix9svqEvaluator> {
    if !config.runtime.use_nnue {
        return None;
    }
    let Some(dir) = config.runtime.nnue_dir.as_deref() else {
        eprintln!(
            "warning: use_nnue is enabled without nnue_dir; falling back to classical evaluation"
        );
        return None;
    };
    match load_mix9svq_evaluator_for_rule(std::path::Path::new(dir), config.rule_set) {
        Ok(evaluator) => Some(evaluator),
        Err(err) => {
            eprintln!(
                "warning: failed to load Mix9svq NNUE from {:?} for {:?}: {err}; falling back to classical evaluation",
                dir, config.rule_set
            );
            None
        }
    }
}

impl AlphaBetaSearcher {
    pub fn new(config: EngineConfig) -> Self {
        let nnue = load_nnue_for_config(&config);
        let mut tt = TranspositionTable::try_new_with_layout(20, nnue.is_some())
            .expect("default TT allocation succeeds");
        tt.set_subply(config.runtime.threads > 1 || nnue.is_some());
        Self {
            config,
            tt,
            vcf: VCFSearcher::default(),
            tt_generation: 0,
            movegen_scratch: MoveAnalysis::new(),
            nnue,
            static_eval_stack: [TT_VALUE_NONE; 128],
            quiet_history: [[0; BOARD_AREA]; 2],
        }
    }

    pub fn with_tt(config: EngineConfig, tt: TranspositionTable) -> Self {
        let nnue = load_nnue_for_config(&config);
        Self::with_tt_and_nnue(config, tt, nnue)
    }

    pub(crate) fn with_tt_and_nnue(
        config: EngineConfig,
        mut tt: TranspositionTable,
        nnue: Option<Mix9svqEvaluator>,
    ) -> Self {
        tt.set_subply(config.runtime.threads > 1 || nnue.is_some());
        Self {
            config,
            tt,
            vcf: VCFSearcher::default(),
            tt_generation: 0,
            movegen_scratch: MoveAnalysis::new(),
            nnue,
            static_eval_stack: [TT_VALUE_NONE; 128],
            quiet_history: [[0; BOARD_AREA]; 2],
        }
    }

    pub fn advance_tt_generation(&mut self) {
        self.tt_generation = (self.tt_generation + 1) & crate::search::tt::GENERATION_MASK;
        if self.tt_generation == 0 {
            self.tt_generation = 1;
        }
    }

    pub(crate) fn tt_generation(&self) -> u16 {
        self.tt_generation
    }

    pub(crate) fn set_tt_generation(&mut self, generation: u16) {
        self.tt_generation = generation;
    }

    pub fn has_nnue(&self) -> bool {
        self.nnue.is_some()
    }

    /// Deterministic 64-bit TT key salt isolating rule set (Freestyle vs Renju)
    /// and active evaluator / NNUE weight identity on shared transposition tables.
    /// Classical Freestyle keeps `0` so legacy baseline keys remain bit-identical.
    fn tt_key_salt(&self) -> u64 {
        const RENJU_SALT: u64 = 0x9E37_79B9_7F4A_7C15;
        let rule_salt = match self.config.rule_set {
            crate::rules::RuleSet::Freestyle => 0,
            crate::rules::RuleSet::Renju => RENJU_SALT,
        };
        match &self.nnue {
            None => rule_salt,
            Some(nnue) => rule_salt ^ nnue.weight_identity_hash(),
        }
    }

    pub(crate) fn sync_config(&mut self, config: &EngineConfig) {
        let nnue_changed = self.config.runtime.use_nnue != config.runtime.use_nnue
            || self.config.runtime.nnue_dir != config.runtime.nnue_dir
            || self.config.rule_set != config.rule_set;
        if self.config != *config {
            self.config = config.clone();
        }
        if nnue_changed {
            self.nnue = load_nnue_for_config(&self.config);
        }
    }

    pub(crate) fn sync_worker_state(
        &mut self,
        config: &EngineConfig,
        tt: &TranspositionTable,
        tt_generation: u16,
    ) {
        self.sync_config(config);
        if !self.tt.is_shared_with(tt) || self.tt.is_subply() != tt.is_subply() {
            self.tt = tt.clone();
        }
        self.tt
            .set_subply(self.config.runtime.threads > 1 || self.nnue.is_some());
        if self.tt_generation != tt_generation {
            self.quiet_history = [[0; BOARD_AREA]; 2];
        }
        self.tt_generation = tt_generation;
    }

    pub fn nonroot_vcf_depth(depth: f64, root_depth: f64) -> i32 {
        (depth + 6.0 - 2.0 * root_depth) as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &mut self,
        board: &mut Board,
        caches: &mut EvalCaches,
        side: Side,
        depth: f64,
        alpha: i32,
        beta: i32,
        wide: usize,
        stats: &mut SearchStats,
        options: SearchOptions<'_>,
    ) -> (i32, Option<Move>) {
        self.search_with_abdada(
            board, caches, side, depth, alpha, beta, wide, stats, options, None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn search_with_abdada(
        &mut self,
        board: &mut Board,
        caches: &mut EvalCaches,
        side: Side,
        depth: f64,
        alpha: i32,
        beta: i32,
        wide: usize,
        stats: &mut SearchStats,
        options: SearchOptions<'_>,
        abdada: Option<&AbdadaContext>,
    ) -> (i32, Option<Move>) {
        self.tt
            .set_subply(self.config.runtime.threads > 1 || self.nnue.is_some());
        let tt = self.tt.clone();
        let key_salt = self.tt_key_salt();
        let tt_view4 = if self.nnue.is_some() {
            tt.view4_with_salt(key_salt)
        } else {
            None
        };
        let tt_view = if tt_view4.is_none() {
            Some(tt.view_with_salt(key_salt))
        } else {
            None
        };
        if let Some(nnue) = self.nnue.as_mut() {
            nnue.sync_with_board(board);
        }
        let mut coverage = CoverageTracker::from_board(board);
        self.search_with_coverage(
            board,
            caches,
            side,
            depth,
            alpha,
            beta,
            wide,
            stats,
            options,
            &mut coverage,
            None,
            abdada,
            tt_view,
            tt_view4,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn search_with_coverage(
        &mut self,
        board: &mut Board,
        caches: &mut EvalCaches,
        side: Side,
        mut depth: f64,
        mut alpha: i32,
        mut beta: i32,
        wide: usize,
        stats: &mut SearchStats,
        options: SearchOptions<'_>,
        coverage: &mut CoverageTracker,
        pending_cache: Option<((usize, usize), &mut Option<EvalSnapshot>)>,
        abdada: Option<&AbdadaContext>,
        tt_view: Option<TTTableView<'_>>,
        tt_view4: Option<TTTableView4<'_>>,
    ) -> (i32, Option<Move>) {
        if options.root {
            self.static_eval_stack[0] = TT_VALUE_NONE;
            self.static_eval_stack[1] = TT_VALUE_NONE;
            if depth <= 1.0 && self.nnue.is_some() {
                self.quiet_history = [[0; BOARD_AREA]; 2];
            }
        }
        let root_depth = options.root_depth.unwrap_or(depth);
        let priority_base = options.priority_base.unwrap_or(board.move_count() as i32);
        let subply_tt = tt_view4
            .map(|v| v.is_subply())
            .or_else(|| tt_view.map(|v| v.is_subply()))
            .unwrap_or(false);
        let max_encodable_depth = if subply_tt {
            SUBPLY_TT_DEPTH_CAP
        } else {
            INTEGER_TT_DEPTH_CAP
        };
        let nnue_depth_offset = if tt_view4.is_some() && subply_tt {
            NNUE_MAIN_TT_DEPTH_OFFSET
        } else {
            0
        };
        let raw_hash_depth = if depth <= 0.0 {
            0
        } else if subply_tt {
            ((depth * SUBPLY_TT_SCALE_F64).floor() as i64).clamp(0, i64::from(i32::MAX)) as i32
                + nnue_depth_offset
        } else {
            (depth.floor() as i64).clamp(0, i64::from(i32::MAX)) as i32
        };
        let mut hash_depth = raw_hash_depth.min(max_encodable_depth);
        let raw_probe_depth = if depth <= 0.0 {
            if nnue_depth_offset > 0 {
                QVCF_TT_DEPTH_FULL
            } else {
                0
            }
        } else if subply_tt {
            ((depth * SUBPLY_TT_SCALE_F64).ceil() as i64).clamp(1, i64::from(i32::MAX)) as i32
                + nnue_depth_offset
        } else {
            (depth.ceil() as i64).clamp(1, i64::from(i32::MAX)) as i32
        };
        // If the requested depth exceeds the encodable range, raise probe_depth
        // above DEPTH_MAX so saturated TT records can only provide move hints
        // and never cut off or narrow the search window.
        let probe_depth = if raw_probe_depth > max_encodable_depth {
            crate::search::tt::DEPTH_MAX + 1
        } else {
            raw_probe_depth
        };
        let original_beta = beta;

        let next_node = stats.nodes + 1;
        if stats
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            stats.stop = true;
            return (0, None);
        }
        if (next_node & 0x3F) == 0 && abdada.is_some_and(AbdadaContext::is_stopped) {
            stats.stop = true;
            return (0, None);
        }
        if stats.node_limit.is_some_and(|limit| stats.nodes >= limit) {
            stats.stop = true;
            return (0, None);
        }
        if stats.deadline.is_some_and(|deadline| {
            (next_node == 1 || (next_node & stats.time_check_mask) == 0)
                && Instant::now() >= deadline
        }) {
            stats.stop = true;
            return (0, None);
        }
        if !self.vcf.budget.enter() {
            stats.stop = true;
            return (0, None);
        }
        stats.nodes = next_node;

        if let Some(score) = terminal_score(board, side, options.ply) {
            return (score, None);
        }

        let root_allowed_moves = if options.root {
            options.root_allowed_moves
        } else {
            None
        };
        let root_restricted = root_allowed_moves.is_some();

        // Quiet last moves can have zero static value and be dropped by movegen.
        if board.move_count() == BOARD_AREA - 1 {
            let last = (0..BOARD_AREA as Move).find(|&m| {
                root_allowed_moves.is_none_or(|allowed| allowed.contains(&m))
                    && board.is_legal_move_for_rule(m, side, self.config.rule_set)
            });
            if let Some(m) = last {
                let (x, y) = move_to_xy(m).expect("board move");
                let score = if board.is_winning_move_for_rule(x, y, side, self.config.rule_set) {
                    INF - options.ply
                } else {
                    0
                };
                return (score, Some(m));
            }
            return (-INF + options.ply, None);
        }

        // Stored depths round down. Round the requirement up so a leaf never
        // cuts off a positive-depth search, even after fractional reductions.
        let is_pv = beta - alpha > 1;
        let (
            probe_hit,
            probe_value,
            probe_tt_value,
            probe_tt_flag,
            probe_tt_depth,
            probe_best_move,
            probe_best_move_hint,
            probe_has_window,
            probe_window_alpha,
            probe_window_beta,
            mut tt_static_eval,
            tt_was_pv,
        ) = if let Some(tt4) = tt_view4 {
            let p4 = tt4.probe(
                board.zobrist_key(),
                probe_depth,
                score_to_tt(alpha, options.ply),
                score_to_tt(beta, options.ply),
            );
            (
                p4.hit,
                p4.value,
                p4.tt_value,
                p4.tt_flag,
                p4.tt_depth,
                p4.best_move,
                p4.best_move_hint,
                p4.has_window,
                p4.window_alpha,
                p4.window_beta,
                p4.static_eval,
                p4.tt_pv,
            )
        } else if let Some(tt2) = tt_view {
            let p2 = tt2.probe(
                board.zobrist_key(),
                probe_depth,
                score_to_tt(alpha, options.ply),
                score_to_tt(beta, options.ply),
            );
            (
                p2.hit,
                p2.value,
                None,
                0,
                0,
                p2.best_move,
                p2.best_move_hint,
                p2.has_window,
                p2.window_alpha,
                p2.window_beta,
                None,
                false,
            )
        } else {
            (
                false, None, None, 0, 0, None, None, false, alpha, beta, None, false,
            )
        };
        if let Some(best_move_hint) = probe_best_move_hint {
            if best_move_hint.generation == self.tt_generation {
                stats.tt_bestmove_current_generation += 1;
            } else {
                stats.tt_bestmove_old_generation += 1;
            }
        }
        let in_singular = options.skip_move.is_some();
        if !root_restricted && !in_singular && probe_hit && probe_value.is_some() {
            stats.tt_hits += 1;
            return (
                score_from_tt(probe_value.expect("checked above"), options.ply),
                probe_best_move,
            );
        }
        if probe_has_window && !options.root && !in_singular {
            alpha = alpha.max(score_from_tt(probe_window_alpha, options.ply));
            beta = beta.min(score_from_tt(probe_window_beta, options.ply));
        }

        if let Some(((mx, my), snapshot_slot)) = pending_cache {
            if snapshot_slot.is_none() {
                *snapshot_slot = Some(caches.snapshot());
                value_wide_compute_for_rule(board, caches, (mx, my), self.config.rule_set);
            }
        }

        if depth <= 0.0 {
            stats.leaf_nodes += 1;
            if self.nnue.is_some() && caches.initialized && caches.rule_set == self.config.rule_set
            {
                let score = if caches.has_l5(side) {
                    INF - options.ply
                } else if caches.has_l5(-side) {
                    self.vcfdefend(
                        board,
                        caches,
                        side,
                        alpha,
                        beta,
                        0,
                        options.ply,
                        stats,
                        coverage,
                        abdada,
                        tt_view4,
                    )
                } else {
                    self.vcfsearch(
                        board,
                        caches,
                        side,
                        alpha,
                        beta,
                        0,
                        options.ply,
                        stats,
                        coverage,
                        abdada,
                        tt_view4,
                        Some((
                            probe_hit,
                            probe_tt_value,
                            probe_tt_flag,
                            probe_best_move,
                            tt_static_eval,
                        )),
                    )
                };
                return (score, None);
            }
            let mut score =
                (-evaluate_board(board, caches, -side, options.opo, &self.config)) as i32;
            let mut leaf_static_eval = TT_VALUE_NONE;
            if score >= WIN {
                score = INF - options.ply;
            } else if score <= -WIN {
                score = -INF + options.ply;
            } else if let Some(nnue) = self.nnue.as_mut() {
                score = tt_static_eval.unwrap_or_else(|| nnue.evaluate_value(side));
                leaf_static_eval = score;
            }
            if let Some(tt4) = tt_view4 {
                tt4.store(TTEntry4 {
                    key: board.zobrist_key(),
                    value: score_to_tt(score, options.ply),
                    static_eval: leaf_static_eval,
                    flag: HASHF_EXACT,
                    depth: 0,
                    best_move: None,
                    tt_pv: is_pv || tt_was_pv,
                    generation: self.tt_generation,
                });
            } else if let Some(tt2) = tt_view {
                tt2.store(TTEntry {
                    key: board.zobrist_key(),
                    value: score_to_tt(score, options.ply),
                    flag: HASHF_EXACT,
                    depth: 0,
                    priority: priority_base * 10,
                    best_move: None,
                    generation: self.tt_generation,
                });
            }
            return (score, None);
        }

        let tt_best_move = if let Some(allowed) = root_allowed_moves {
            options
                .root_preferred_move
                .filter(|move_| allowed.contains(move_))
                .or_else(|| probe_best_move.filter(|move_| allowed.contains(move_)))
        } else {
            probe_best_move
        };

        let original_alpha = alpha;
        let child_wide = if self.nnue.is_some() {
            wide
        } else {
            (((wide * self.config.root_search.ratio_num as usize)
                / self.config.root_search.ratio_den as usize)
                + 1)
            .min(wide)
        };
        let no_l4_l5 = caches.initialized
            && caches.rule_set == self.config.rule_set
            && !caches.has_any_l4_or_l5();

        let ply_idx = (options.ply as usize).min(127);
        let mut improving = false;
        let mut node_eval: Option<i32> = None;

        // Tactically-guarded NNUE Static Eval + Razoring (with QVCF) +
        // Deeper Rapfi-calibrated Reverse Futility Pruning (RFP) BEFORE movegen:
        // When O(1) bucket masks confirm neither side has an immediate L4/L5
        // threat, evaluate NNUE static value (cached in 4-way TT), update
        // `static_eval_stack` / `improving`, and prune before movegen.
        if let Some(tt4) = tt_view4 {
            if self.nnue.is_some() && !options.root {
                if no_l4_l5 {
                    let raw_eval = match tt_static_eval {
                        Some(ev) => ev,
                        None => {
                            let ev = self
                                .nnue
                                .as_mut()
                                .expect("checked above")
                                .evaluate_value(side);
                            tt_static_eval = Some(ev);
                            tt4.store(TTEntry4 {
                                key: board.zobrist_key(),
                                value: TT_VALUE_NONE,
                                static_eval: ev,
                                flag: HASHF_EMPTY,
                                depth: 0,
                                best_move: None,
                                tt_pv: tt_was_pv,
                                generation: self.tt_generation,
                            });
                            ev
                        }
                    };
                    self.static_eval_stack[ply_idx] = raw_eval;
                    improving = ply_idx >= 2
                        && self.static_eval_stack[ply_idx - 2] != TT_VALUE_NONE
                        && raw_eval > self.static_eval_stack[ply_idx - 2];

                    if !is_pv && depth > 0.0 {
                        let mut eval = raw_eval;
                        if let Some(tt_val) = probe_tt_value {
                            let adj = score_from_tt(tt_val, options.ply);
                            if adj.abs() < WIN
                                && (probe_tt_flag == HASHF_EXACT
                                    || (probe_tt_flag == HASHF_BETA && adj > eval)
                                    || (probe_tt_flag == HASHF_ALPHA && adj < eval))
                            {
                                eval = adj;
                            }
                        }
                        node_eval = Some(eval);
                        // Step 7 (Rapfi): Razoring with leaf QVCF when static eval is
                        // clearly below alpha at shallow depth (< 2.6 ply).
                        if depth < 2.6 && (alpha < WIN || !probe_hit) {
                            let razor_margin = match self.config.rule_set {
                                RuleSet::Renju => (0.17 * depth * depth + 8.0 * depth) as i32 + 9,
                                RuleSet::Freestyle => {
                                    (0.17 * depth * depth + 15.0 * depth) as i32 + 11
                                }
                            }
                            .max(0);
                            if eval + razor_margin < alpha {
                                let score = self.vcfsearch(
                                    board,
                                    caches,
                                    side,
                                    alpha,
                                    alpha + 1,
                                    0,
                                    options.ply,
                                    stats,
                                    coverage,
                                    abdada,
                                    tt_view4,
                                    Some((
                                        probe_hit,
                                        probe_tt_value,
                                        probe_tt_flag,
                                        probe_best_move,
                                        Some(raw_eval),
                                    )),
                                );
                                return (score, None);
                            }
                        }
                        // Step 8 (Rapfi): Futility / Reverse Futility Pruning across
                        // quiet non-PV nodes calibrated with `cut_node` & `improving`.
                        if depth <= 9.0 && beta > -WIN && beta < WIN && eval < WIN {
                            let (scale, no_tt_penalty) = match self.config.rule_set {
                                RuleSet::Renju => (63, 13),
                                RuleSet::Freestyle => (68, 16),
                            };
                            let eff_scale = scale
                                - if options.cut_node && !probe_hit {
                                    no_tt_penalty
                                } else {
                                    0
                                };
                            let eff_depth =
                                (depth - 1.0 - if improving { 1.0 } else { 0.0 }).max(0.0);
                            let rfp_margin = (f64::from(eff_scale) * eff_depth).trunc() as i32;
                            if eval - rfp_margin >= beta {
                                return (eval, None);
                            }
                        }
                    }
                } else {
                    self.static_eval_stack[ply_idx] =
                        if ply_idx > 0 && self.static_eval_stack[ply_idx - 1] != TT_VALUE_NONE {
                            -self.static_eval_stack[ply_idx - 1]
                        } else {
                            TT_VALUE_NONE
                        };
                }
            }
        }

        // Step 10 (Rapfi): Internal Iterative Reduction (IIR) on non-root PV
        // nodes missing a TT move or with a stale PV TT depth.
        if self.nnue.is_some() && !options.root && no_l4_l5 && is_pv && depth > 1.0 {
            let mut iir_red = 0.0;
            if tt_best_move.is_none() && depth >= 2.0 {
                iir_red = match self.config.rule_set {
                    RuleSet::Renju => 24.0 / SUBPLY_TT_SCALE_F64,
                    RuleSet::Freestyle => 29.0 / SUBPLY_TT_SCALE_F64,
                };
            } else if tt_best_move.is_some() {
                let tt_depth_plies =
                    f64::from(probe_tt_depth.saturating_sub(NNUE_MAIN_TT_DEPTH_OFFSET))
                        / SUBPLY_TT_SCALE_F64;
                let (iir_scale, iir_max) = match self.config.rule_set {
                    RuleSet::Renju => (0.17, 3.3),
                    RuleSet::Freestyle => (0.19, 3.6),
                };
                let raw_red = ((depth - tt_depth_plies) * iir_scale).clamp(0.0, iir_max);
                iir_red = (raw_red * SUBPLY_TT_SCALE_F64).floor() / SUBPLY_TT_SCALE_F64;
            }
            if iir_red > 0.0 {
                depth = (depth - iir_red).max(0.5);
                if subply_tt {
                    hash_depth = (((depth * SUBPLY_TT_SCALE_F64).floor() as i64)
                        .clamp(0, i64::from(i32::MAX)) as i32
                        + nnue_depth_offset)
                        .min(max_encodable_depth);
                }
            }
        }
        let _ = improving;
        // Fail-High Reduction (Rapfi ~50 Elo): on non-PV cutNodes where static
        // eval already exceeds beta by a depth-scaled margin, reduce non-B4
        // moves by 1 ply.
        let likely_fail_high = self.nnue.is_some()
            && !options.root
            && !is_pv
            && options.cut_node
            && depth >= 2.0
            && node_eval.is_some_and(|ev| {
                let margin = match self.config.rule_set {
                    RuleSet::Renju => -4 + 16 * (depth as i32),
                    RuleSet::Freestyle => -3 + 20 * (depth as i32),
                };
                ev >= beta + margin
            });

        let mut current = i32::MIN;
        let mut has_score = false;
        let mut best_move = None;
        let mut hash_flag = HASHF_ALPHA;
        let mut found_pv = false;
        let mut pre_searched_tt_move: Option<Move> = None;

        // Pre-movegen NNUE TT-move probe: on quiet non-root nodes (no L4/L5),
        // search a legal covered TT move directly before candidate generation.
        // Includes Rapfi-calibrated Singular Extension / Multi-Cut / Negative
        // Extension for deep TT moves (`depth >= 6.85`).
        if self.nnue.is_some()
            && !options.root
            && !in_singular
            && !self.config.runtime.nonroot_vcf
            && no_l4_l5
        {
            if let Some(tt_mv) = tt_best_move {
                if coverage.is_covered(tt_mv)
                    && is_rule_legal_for_movegen(board, caches, tt_mv, side, self.config.rule_set)
                {
                    let (mx, my) = move_to_xy(tt_mv).expect("TT move is valid");
                    let tt_self_attack = attack_level(caches, mx, my, side);
                    let mut tt_extension = 0.0_f64;

                    // Singular Extension (Rapfi ~52 Elo): verify whether `tt_mv`
                    // is uniquely superior to all other moves at reduced depth.
                    if depth >= 6.85
                        && (probe_tt_flag == HASHF_BETA || probe_tt_flag == HASHF_EXACT)
                    {
                        if let Some(tt_val_raw) = probe_tt_value {
                            let tt_val_adj = score_from_tt(tt_val_raw, options.ply);
                            let tt_depth_plies =
                                f64::from(probe_tt_depth.saturating_sub(NNUE_MAIN_TT_DEPTH_OFFSET))
                                    / SUBPLY_TT_SCALE_F64;
                            let se_tte_depth = match self.config.rule_set {
                                RuleSet::Renju => 1.95,
                                RuleSet::Freestyle => 2.09,
                            };
                            if tt_val_adj.abs() < WIN && tt_depth_plies >= depth - se_tte_depth {
                                let former_pv = !is_pv && tt_was_pv;
                                let se_scale = match self.config.rule_set {
                                    RuleSet::Renju => 0.90,
                                    RuleSet::Freestyle => 0.85,
                                };
                                let singular_margin =
                                    ((se_scale + if former_pv { 1.0 } else { 0.0 }) * depth) as i32;
                                let singular_beta = (tt_val_adj - singular_margin).max(-WIN + 1);
                                let raw_se_depth =
                                    (depth * 0.55 - if former_pv { 1.0 } else { 0.0 }).max(1.0);
                                let se_depth = (raw_se_depth * SUBPLY_TT_SCALE_F64).floor()
                                    / SUBPLY_TT_SCALE_F64;

                                let se_key =
                                    board.zobrist_key() ^ 0x517C_C1B7_2722_0A95 ^ u64::from(tt_mv);
                                let skip_dup_se = abdada.is_some_and(|ctx| {
                                    ctx.worker_id > 0 && ctx.table.is_busy(se_key, ctx.salt)
                                });
                                if !skip_dup_se {
                                    if let Some(ctx) = abdada {
                                        ctx.table.mark_busy(se_key, ctx.salt);
                                    }
                                    let (se_score, _) = self.search_with_coverage(
                                        board,
                                        caches,
                                        side,
                                        se_depth,
                                        singular_beta - 1,
                                        singular_beta,
                                        wide,
                                        stats,
                                        SearchOptions {
                                            skip_move: Some(tt_mv),
                                            ..options
                                        },
                                        coverage,
                                        None,
                                        abdada,
                                        tt_view,
                                        tt_view4,
                                    );
                                    if let Some(ctx) = abdada {
                                        ctx.table.clear_busy(se_key, ctx.salt);
                                    }
                                    if stats.stop {
                                        return (current, best_move);
                                    }
                                    if se_score < singular_beta {
                                        tt_extension = 1.0;
                                    } else if singular_beta >= beta {
                                        return (singular_beta, None);
                                    } else if tt_val_adj >= beta {
                                        tt_extension = -1.5;
                                    }
                                }
                            }
                        }
                    }

                    let mut tt_depthdown = 1.0 - tt_extension;
                    if likely_fail_high && tt_self_attack < 4 {
                        tt_depthdown += 1.0;
                    }
                    let mut candidate_snapshot: Option<EvalSnapshot> = None;
                    board
                        .play_assuming_rule_legal(tt_mv, Some(side), self.config.rule_set)
                        .expect("legal TT move stays legal");
                    if let Some(tt4) = tt_view4 {
                        tt4.prefetch(board.zobrist_key());
                    }
                    coverage.add_move(tt_mv);
                    if let Some(nnue) = self.nnue.as_mut() {
                        nnue.before_move(side, mx, my);
                    }
                    let child_options = SearchOptions {
                        opo: 1 - options.opo,
                        ply: options.ply + 1,
                        root: false,
                        root_allowed_moves: None,
                        root_preferred_move: None,
                        downf: options.downf,
                        root_depth: Some(root_depth),
                        priority_base: Some(priority_base),
                        cut_node: if is_pv { false } else { !options.cut_node },
                        skip_move: None,
                    };
                    let (child_score, _) = self.search_with_coverage(
                        board,
                        caches,
                        -side,
                        depth - tt_depthdown,
                        -beta,
                        -alpha,
                        child_wide,
                        stats,
                        child_options,
                        coverage,
                        Some(((mx, my), &mut candidate_snapshot)),
                        abdada,
                        tt_view,
                        tt_view4,
                    );
                    board.undo().expect("TT move undo matches play");
                    if let Some(nnue) = self.nnue.as_mut() {
                        nnue.after_undo(side, mx, my);
                    }
                    coverage.remove_move(tt_mv);
                    if let Some(snapshot) = candidate_snapshot.as_ref() {
                        caches.restore_snapshot(snapshot);
                    }
                    if stats.stop {
                        return (current, best_move);
                    }
                    let score = -child_score;
                    current = score;
                    has_score = true;
                    best_move = Some(tt_mv);
                    pre_searched_tt_move = Some(tt_mv);
                    if score > alpha {
                        alpha = score;
                        hash_flag = HASHF_EXACT;
                        found_pv = true;
                    }
                    if alpha >= beta {
                        hash_flag = HASHF_BETA;
                        stats.cutoffs += 1;
                        let mut store_depth = hash_depth;
                        if (current >= WIN && current > original_alpha)
                            || (current <= -WIN && current < original_beta)
                        {
                            hash_flag = HASHF_EXACT;
                            store_depth += if subply_tt { SUBPLY_TT_WIN_BONUS } else { 10 };
                        }
                        if let Some(tt4) = tt_view4 {
                            tt4.store(TTEntry4 {
                                key: board.zobrist_key(),
                                value: score_to_tt(current, options.ply),
                                static_eval: tt_static_eval.unwrap_or(TT_VALUE_NONE),
                                flag: hash_flag,
                                depth: store_depth,
                                best_move,
                                tt_pv: is_pv || tt_was_pv,
                                generation: self.tt_generation,
                            });
                        }
                        return (current, best_move);
                    }
                }
            }
        }

        let mut generated = generate_candidates_with_coverage_scratch(
            board,
            caches,
            side,
            &self.config,
            Some(wide),
            root_allowed_moves,
            tt_best_move,
            options.root,
            coverage,
            &mut self.movegen_scratch,
        );
        let win_priority = generated.win_priority;
        let single_forcing = generated.single_forcing;
        let hostile_threat = generated.hostile_threat;
        if no_l4_l5 {
            debug_assert!(!win_priority && !single_forcing && !hostile_threat);
        }
        if let Some(tt_mv) = pre_searched_tt_move {
            if let Some(pos) = generated
                .candidates
                .iter()
                .position(|candidate| candidate.move_ == tt_mv)
            {
                generated.candidates.remove(pos);
            }
        }

        let lazy_tie_ordering = !options.root && !self.config.runtime.nonroot_vcf;
        let mut rest_primary_sorted = true;
        let mut policy_rest_sorted = true;
        let use_nnue_policy = self.nnue.is_some()
            && !win_priority
            && !single_forcing
            && generated.candidates.len() > 1
            && (options.root || depth >= 2.0);
        let mut ordered = if use_nnue_policy {
            let mut candidates = generated.candidates;
            if let Some(pos) = (pre_searched_tt_move.is_none())
                .then_some(tt_best_move)
                .flatten()
                .and_then(|tt_mv| {
                    candidates
                        .iter()
                        .position(|candidate| candidate.move_ == tt_mv)
                })
            {
                // Keep exact relative scan order for `candidates[1..]` while
                // deferring NNUE Policy evaluation until `index == 1` (only
                // when `tt_best_move` fails to cause an immediate cutoff).
                candidates[..=pos].rotate_right(1);
                policy_rest_sorted = false;
                candidates
            } else {
                let n = candidates.len();
                let mut move_buf = [0_u16; BOARD_AREA];
                let mut score_buf = [0.0_f32; BOARD_AREA];
                for (i, c) in candidates.iter().enumerate() {
                    move_buf[i] = c.move_;
                }
                if let Some(nnue) = self.nnue.as_mut() {
                    nnue.evaluate_policy_moves(side, &move_buf[..n], &mut score_buf[..n]);
                }
                for (i, c) in candidates.iter_mut().enumerate() {
                    c.order_score = f64::from(score_buf[i]);
                }
                candidates.sort_by(|a, b| b.order_score.total_cmp(&a.order_score));
                candidates
            }
        } else if options.root {
            order_candidates_root_classic(board, &generated.candidates, side)
        } else if lazy_tie_ordering {
            let mut candidates = generated.candidates;
            if pre_searched_tt_move.is_none()
                && !win_priority
                && !single_forcing
                && candidates.len() > 1
                && tt_best_move.is_some()
            {
                if let Some(pos) = candidates
                    .iter()
                    .position(|candidate| Some(candidate.move_) == tt_best_move)
                {
                    candidates.swap(0, pos);
                    rest_primary_sorted = false;
                    candidates
                } else {
                    order_candidates_primary_owned(candidates, tt_best_move)
                }
            } else {
                order_candidates_primary_owned(candidates, tt_best_move)
            }
        } else {
            order_candidates_owned(board, generated.candidates, side, tt_best_move)
        };
        let mut tie_sorted_until = if use_nnue_policy || !lazy_tie_ordering {
            ordered.len()
        } else if !rest_primary_sorted {
            1
        } else {
            0
        };
        if win_priority && !ordered.is_empty() {
            if lazy_tie_ordering && tie_sorted_until == 0 {
                let _ = resolve_candidate_tie_group(board, &mut ordered, 0, side, tt_best_move);
            }
            return (INF - options.ply, Some(ordered[0].move_));
        }

        if self.config.runtime.compute_vcf && self.config.runtime.nonroot_vcf && !options.root {
            let nonroot_vcf_depth = Self::nonroot_vcf_depth(depth, root_depth);
            if nonroot_vcf_depth > 0
                && self
                    .vcf
                    .search_with_multi_reply_for_rule(
                        board,
                        -side,
                        nonroot_vcf_depth,
                        self.config.runtime.vcf_multi_reply,
                        self.config.rule_set,
                    )
                    .found
            {
                let mut filtered = Vec::new();
                for candidate in ordered {
                    if self.vcf.budget.stopped() {
                        stats.stop = true;
                        return (0, None);
                    }
                    let mut trial = board.clone();
                    trial
                        .play_assuming_rule_legal(candidate.move_, Some(side), self.config.rule_set)
                        .expect("ordered move stays legal on trial board");
                    if !self
                        .vcf
                        .search_with_multi_reply_for_rule(
                            &trial,
                            -side,
                            nonroot_vcf_depth,
                            self.config.runtime.vcf_multi_reply,
                            self.config.rule_set,
                        )
                        .found
                    {
                        filtered.push(candidate);
                    }
                }
                ordered = filtered;
            }
        }

        if self.config.runtime.nonroot_vcf && self.vcf.budget.stopped() {
            stats.stop = true;
            return (0, None);
        }
        let tt_offset = usize::from(pre_searched_tt_move.is_some());
        if !lazy_tie_ordering && !use_nnue_policy && !win_priority && !single_forcing {
            ordered.truncate(wide.saturating_sub(tt_offset));
        }
        let search_limit = if !win_priority && !single_forcing {
            (ordered.len() + tt_offset)
                .min(wide)
                .saturating_sub(tt_offset)
        } else {
            ordered.len()
        };
        if search_limit == 0 && !has_score {
            return (-INF - 1, None);
        }

        let case_count = search_limit + tt_offset;
        let (pre_corner, pre_half) = if options.root {
            compute_corner_state(board)
        } else {
            (false, 0)
        };
        let mut running_downf = options.downf;
        let raw_depthdown = if self.nnue.is_some() {
            if single_forcing {
                (1.0 - self.config.search.extend_ratio).max(0.0)
            } else {
                1.0
            }
        } else {
            0.0_f64.max(
                1.0 - self.config.search.extend_ratio
                    + self.config.search.extend_ratio * (case_count.max(1) as f64).ln()
                        / (wide.max(2) as f64).ln(),
            )
        };
        // Floor-quantize fractional depth reductions to exact 1/64-ply dyadic
        // steps in multi-threaded mode so internal nodes hit exact same-depth
        // TT entries (floor == ceil) without ever rounding near-full-width
        // reductions up to 1.0 (preserving fractional extension plies) while
        // store=floor / probe=ceil remains strictly sound for all f64 depths.
        let base_depthdown = if subply_tt {
            (raw_depthdown * SUBPLY_TT_SCALE_F64).floor() / SUBPLY_TT_SCALE_F64
        } else {
            raw_depthdown
        };
        let mut policy_top_logit = f64::NEG_INFINITY;
        if use_nnue_policy && policy_rest_sorted && !ordered.is_empty() {
            policy_top_logit = ordered[0].order_score;
        }
        let nnue_lmp_limit = if self.nnue.is_some()
            && !options.root
            && !is_pv
            && !win_priority
            && !single_forcing
            && !hostile_threat
            && depth <= 10.0
        {
            let fmc = 3 + (depth.max(0.0).powf(1.30) as usize);
            (fmc / (2 - usize::from(improving))).max(2)
        } else {
            usize::MAX
        };
        let use_abdada = abdada.is_some() && depth >= ABDADA_MIN_DEPTH && case_count > 1;

        #[derive(Clone, Copy)]
        struct DeferredCandidate {
            index: usize,
            candidate: Candidate,
            local_downf: i32,
            depthdown: f64,
            lmr_reduction: f64,
        }
        let default_candidate = ordered.first().copied().unwrap_or(Candidate {
            move_: 0,
            order_score: 0.0,
            self_attack: 0,
            opp_attack: 0,
        });
        let mut deferred = [DeferredCandidate {
            index: 0,
            candidate: default_candidate,
            local_downf: 0,
            depthdown: 0.0,
            lmr_reduction: 0.0,
        }; 64];
        let mut deferred_len = 0_usize;
        let mut searched_quiets = [0_u16; 64];
        let mut searched_quiets_len = 0_usize;
        let mut early_exit = false;

        for pass in 0..2 {
            let pass_len = if pass == 0 {
                search_limit
            } else {
                if early_exit || deferred_len == 0 {
                    break;
                }
                deferred_len
            };

            for step in 0..pass_len {
                let (index, candidate, local_downf, depthdown, mut lmr_reduction) = if pass == 0 {
                    let index = step + tt_offset;
                    if !policy_rest_sorted && index == 1 {
                        let rest = &mut ordered[1..];
                        let n = rest.len();
                        let mut move_buf = [0_u16; BOARD_AREA];
                        let mut score_buf = [0.0_f32; BOARD_AREA];
                        for (i, c) in rest.iter().enumerate() {
                            move_buf[i] = c.move_;
                        }
                        if let Some(nnue) = self.nnue.as_mut() {
                            nnue.evaluate_policy_moves(side, &move_buf[..n], &mut score_buf[..n]);
                        }
                        for (i, c) in rest.iter_mut().enumerate() {
                            c.order_score = f64::from(score_buf[i]);
                        }
                        rest.sort_by(|a, b| b.order_score.total_cmp(&a.order_score));
                        if let Some(first) = rest.first() {
                            policy_top_logit = first.order_score;
                        }
                        policy_rest_sorted = true;
                    }
                    if !rest_primary_sorted && index == 1 {
                        order_candidates_primary_slice(&mut ordered[1..]);
                        rest_primary_sorted = true;
                    }
                    if lazy_tie_ordering && step >= tie_sorted_until {
                        tie_sorted_until = resolve_candidate_tie_group(
                            board,
                            &mut ordered,
                            step,
                            side,
                            tt_best_move,
                        );
                    }
                    let candidate = ordered[step];
                    if options.skip_move == Some(candidate.move_) {
                        continue;
                    }
                    // Safe Move Count Pruning (LMP) for quiet non-forcing NNUE
                    // tail moves once at least one non-losing move is found.
                    if index >= nnue_lmp_limit
                        && has_score
                        && current > -WIN
                        && candidate.self_attack < 4
                        && candidate.opp_attack < 4
                    {
                        continue;
                    }
                    let mut depthdown = base_depthdown;
                    if likely_fail_high && candidate.self_attack < 4 {
                        depthdown += 1.0;
                    }
                    let (local_downf, lmr_reduction) = if self.nnue.is_some() {
                        let mut red = 0.0_f64;
                        if depth >= 2.0
                            && index >= 1 + usize::from(options.root)
                            && !win_priority
                            && !single_forcing
                            && candidate.self_attack < 4
                        {
                            let rule_idx = usize::from(self.config.rule_set == RuleSet::Renju);
                            let d_idx = (depth as usize).clamp(1, 63);
                            let mc_idx = (index + 1).clamp(1, 63);
                            let mut r = LMR_LUT[rule_idx][d_idx] * LMR_LUT[rule_idx][mc_idx];
                            if let Some(ctx) = abdada {
                                if ctx.worker_id > 0 && (ctx.worker_id & 1) == 1 {
                                    r += 0.25;
                                }
                            }
                            if !is_pv && !improving && r > 1.0 {
                                r += 1.0;
                            }
                            if use_nnue_policy && policy_top_logit.is_finite() {
                                let gap = (policy_top_logit - candidate.order_score).max(0.0);
                                if gap > 1.0 {
                                    let scale = if is_pv { 0.25 } else { 0.45 };
                                    r += (scale * (gap - 1.0)).min(3.0);
                                }
                            }
                            if is_pv || tt_was_pv {
                                r -= 0.71;
                            }
                            if options.cut_node {
                                r += 1.25;
                            }
                            let player = if side == BLACK { 0 } else { 1 };
                            let h = self.quiet_history[player][candidate.move_ as usize];
                            r -= (f64::from(h) / 2048.0).clamp(-1.5, 1.5);
                            if candidate.opp_attack >= 4 || hostile_threat {
                                r *= 0.5;
                            }
                            let full_child_depth = depth - depthdown;
                            if r > 0.0 && full_child_depth > 1.0 {
                                let r_quant =
                                    (r * SUBPLY_TT_SCALE_F64).floor() / SUBPLY_TT_SCALE_F64;
                                let reduced_depth = (full_child_depth - r_quant).max(1.0);
                                let actual_red = (full_child_depth - reduced_depth).max(0.0);
                                if actual_red > 0.0 {
                                    depthdown += actual_red;
                                    red = actual_red;
                                }
                            }
                        }
                        (0, red)
                    } else {
                        running_downf += index as i32;
                        let mut local_downf = running_downf;
                        let mut net = 0_i32;
                        if local_downf >= 15 {
                            net = local_downf / 15;
                            depthdown += f64::from(net);
                            local_downf %= 15;
                        }
                        running_downf = local_downf;
                        (local_downf, f64::from(net))
                    };
                    (index, candidate, local_downf, depthdown, lmr_reduction)
                } else {
                    let wid = abdada.map_or(0, |ctx| ctx.worker_id);
                    let d_idx = if wid == 0 || deferred_len <= 1 {
                        step
                    } else if wid % 2 == 1 {
                        (deferred_len - 1 - step + wid) % deferred_len
                    } else {
                        (step + wid) % deferred_len
                    };
                    let item = deferred[d_idx];
                    (
                        item.index,
                        item.candidate,
                        item.local_downf,
                        item.depthdown,
                        item.lmr_reduction,
                    )
                };

                if options.root {
                    if let Some(ctx) = abdada {
                        if let Some((shared_alpha, shared_move)) =
                            ctx.root_best.load_if_better(alpha)
                        {
                            alpha = shared_alpha;
                            if shared_alpha > current {
                                current = shared_alpha;
                                has_score = true;
                            }
                            best_move = Some(shared_move);
                            hash_flag = HASHF_EXACT;
                            found_pv = true;
                        }
                    }
                }

                let mut marked_busy_key: Option<u64> = None;
                if use_abdada && pass == 0 && index > 0 {
                    if let Some(ctx) = abdada {
                        let depth_tag =
                            ((depth - depthdown).max(0.0) * SUBPLY_TT_SCALE_F64).floor() as u64;
                        let child_key = board.zobrist_key_after(candidate.move_, side)
                            ^ depth_tag.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                        if found_pv
                            && deferred_len < deferred.len()
                            && ctx.table.is_busy(child_key, ctx.salt)
                        {
                            deferred[deferred_len] = DeferredCandidate {
                                index,
                                candidate,
                                local_downf,
                                depthdown,
                                lmr_reduction,
                            };
                            deferred_len += 1;
                            continue;
                        }
                        ctx.table.mark_busy(child_key, ctx.salt);
                        marked_busy_key = Some(child_key);
                    }
                }

                let mut candidate_snapshot: Option<EvalSnapshot> = None;
                let (mx, my) = move_to_xy(candidate.move_).expect("candidate move is valid");
                let profile_root_candidate = options.root && stats.root_profile;
                let profile_start = profile_root_candidate.then(Instant::now);
                let profile_nodes_before = stats.nodes;
                let profile_alpha_before = alpha;
                let mut profile_zero_window_nodes = 0_usize;
                let mut profile_zero_window_elapsed_us = 0_u128;
                let mut profile_full_window_nodes = 0_usize;
                let mut profile_full_window_elapsed_us = 0_u128;
                let mut profile_pvs_research = false;
                board
                    .play_assuming_rule_legal(candidate.move_, Some(side), self.config.rule_set)
                    .expect("ordered candidate stays legal");
                if let Some(tt4) = tt_view4 {
                    tt4.prefetch(board.zobrist_key());
                } else if let Some(tt2) = tt_view {
                    tt2.prefetch(board.zobrist_key());
                }
                coverage.add_move(candidate.move_);
                if let Some(nnue) = self.nnue.as_mut() {
                    nnue.before_move(side, mx, my);
                }

                let mut atdown = 0_i32;
                if self.nnue.is_none() {
                    if candidate.self_attack == 4 {
                        atdown = self.config.search.atdown4 as i32;
                    } else if candidate.self_attack == 3 {
                        atdown = self.config.search.atdown3 as i32;
                    }
                    if options.root {
                        let (x, y) =
                            move_to_xy(candidate.move_).expect("ordered candidate stays in range");
                        let h = x.min(y).min(board.size() - 1 - x).min(board.size() - 1 - y);
                        let is_corner = pre_corner || h <= 1 || (h == 2 && pre_half >= 1);
                        atdown += rootbonus(board, x, y, is_corner);
                    }
                }

                let mut attempt_depth = depth - depthdown;
                let score = loop {
                    let base_child_options = SearchOptions {
                        opo: 1 - options.opo,
                        ply: options.ply + 1,
                        root: false,
                        root_allowed_moves: None,
                        root_preferred_move: None,
                        downf: local_downf,
                        root_depth: Some(root_depth),
                        priority_base: Some(priority_base),
                        cut_node: false,
                        skip_move: None,
                    };
                    let score = if found_pv {
                        let narrow_start = profile_root_candidate.then(Instant::now);
                        let narrow_nodes_before = stats.nodes;
                        let shifted_alpha = shift_bound_with_atdown(alpha, atdown);
                        let (narrow_score, _) = self.search_with_coverage(
                            board,
                            caches,
                            -side,
                            attempt_depth,
                            -shifted_alpha - 1,
                            -shifted_alpha,
                            child_wide,
                            stats,
                            SearchOptions {
                                cut_node: true,
                                ..base_child_options
                            },
                            coverage,
                            Some(((mx, my), &mut candidate_snapshot)),
                            abdada,
                            tt_view,
                            tt_view4,
                        );
                        if let Some(start) = narrow_start {
                            profile_zero_window_nodes +=
                                stats.nodes.saturating_sub(narrow_nodes_before);
                            profile_zero_window_elapsed_us += start.elapsed().as_micros();
                        }
                        if stats.stop {
                            break narrow_score;
                        }
                        let narrowed = score_from_child(narrow_score, atdown);
                        if alpha < narrowed && narrowed < beta {
                            profile_pvs_research = true;
                            let full_start = profile_root_candidate.then(Instant::now);
                            let full_nodes_before = stats.nodes;
                            let (full_score, _) = self.search_with_coverage(
                                board,
                                caches,
                                -side,
                                attempt_depth,
                                -shift_bound_with_atdown(beta, atdown),
                                -shift_bound_with_atdown(alpha, atdown),
                                child_wide,
                                stats,
                                SearchOptions {
                                    cut_node: false,
                                    ..base_child_options
                                },
                                coverage,
                                Some(((mx, my), &mut candidate_snapshot)),
                                abdada,
                                tt_view,
                                tt_view4,
                            );
                            if let Some(start) = full_start {
                                profile_full_window_nodes +=
                                    stats.nodes.saturating_sub(full_nodes_before);
                                profile_full_window_elapsed_us += start.elapsed().as_micros();
                            }
                            if stats.stop {
                                break full_score;
                            }
                            score_from_child(full_score, atdown)
                        } else {
                            narrowed
                        }
                    } else {
                        let full_start = profile_root_candidate.then(Instant::now);
                        let full_nodes_before = stats.nodes;
                        let (full_score, _) = self.search_with_coverage(
                            board,
                            caches,
                            -side,
                            attempt_depth,
                            -shift_bound_with_atdown(beta, atdown),
                            -shift_bound_with_atdown(alpha, atdown),
                            child_wide,
                            stats,
                            SearchOptions {
                                cut_node: if is_pv { false } else { !options.cut_node },
                                ..base_child_options
                            },
                            coverage,
                            Some(((mx, my), &mut candidate_snapshot)),
                            abdada,
                            tt_view,
                            tt_view4,
                        );
                        if let Some(start) = full_start {
                            profile_full_window_nodes +=
                                stats.nodes.saturating_sub(full_nodes_before);
                            profile_full_window_elapsed_us += start.elapsed().as_micros();
                        }
                        if stats.stop {
                            break full_score;
                        }
                        score_from_child(full_score, atdown)
                    };
                    if score >= WIN {
                        break score;
                    }
                    if score > alpha && score > current && lmr_reduction > 0.0 {
                        attempt_depth += lmr_reduction;
                        lmr_reduction = 0.0;
                        continue;
                    }
                    break score;
                };

                if let (Some(child_key), Some(ctx)) = (marked_busy_key, abdada) {
                    ctx.table.clear_busy(child_key, ctx.salt);
                }

                board.undo().expect("ordered candidate was just played");
                if let Some(nnue) = self.nnue.as_mut() {
                    nnue.after_undo(side, mx, my);
                }
                coverage.remove_move(candidate.move_);
                if let Some(snapshot) = candidate_snapshot {
                    caches.restore_snapshot(&snapshot);
                }
                if stats.stop {
                    if profile_root_candidate {
                        stats.root_candidates.push(RootCandidateProfile {
                            index,
                            move_: candidate.move_,
                            order_score: candidate.order_score as i32,
                            self_attack: candidate.self_attack,
                            opp_attack: candidate.opp_attack,
                            depthdown_milli: (depthdown * 1000.0).round() as i32,
                            atdown,
                            attempt_depth_milli: (attempt_depth * 1000.0).round() as i32,
                            score,
                            nodes: stats.nodes.saturating_sub(profile_nodes_before),
                            elapsed_us: profile_start
                                .map(|start| start.elapsed().as_micros())
                                .unwrap_or_default(),
                            alpha_before: profile_alpha_before,
                            alpha_after: alpha,
                            beta,
                            reason: "stop",
                            zero_window_nodes: profile_zero_window_nodes,
                            zero_window_elapsed_us: profile_zero_window_elapsed_us,
                            full_window_nodes: profile_full_window_nodes,
                            full_window_elapsed_us: profile_full_window_elapsed_us,
                            pvs_research: profile_pvs_research,
                        });
                    }
                    early_exit = true;
                    break;
                }

                if score > current {
                    current = score;
                    has_score = true;
                    if options.root && !found_pv {
                        best_move = Some(candidate.move_);
                    }
                }
                let improved = score > alpha;
                if score > alpha {
                    alpha = score;
                    best_move = Some(candidate.move_);
                    hash_flag = HASHF_EXACT;
                    found_pv = true;
                    if options.root {
                        if let Some(ctx) = abdada {
                            ctx.root_best.publish(score, candidate.move_);
                        }
                    }
                }
                let root_win = options.root && score >= WIN;
                let beta_cutoff = alpha >= beta;
                let profile_elapsed_us = profile_start
                    .map(|start| start.elapsed().as_micros())
                    .unwrap_or_default();
                if profile_root_candidate {
                    let reason = if root_win {
                        "root_win"
                    } else if beta_cutoff {
                        "beta_cutoff"
                    } else if improved {
                        "improved"
                    } else {
                        "fail_low"
                    };
                    stats.root_candidates.push(RootCandidateProfile {
                        index,
                        move_: candidate.move_,
                        order_score: candidate.order_score as i32,
                        self_attack: candidate.self_attack,
                        opp_attack: candidate.opp_attack,
                        depthdown_milli: (depthdown * 1000.0).round() as i32,
                        atdown,
                        attempt_depth_milli: (attempt_depth * 1000.0).round() as i32,
                        score,
                        nodes: stats.nodes.saturating_sub(profile_nodes_before),
                        elapsed_us: profile_elapsed_us,
                        alpha_before: profile_alpha_before,
                        alpha_after: alpha,
                        beta,
                        reason,
                        zero_window_nodes: profile_zero_window_nodes,
                        zero_window_elapsed_us: profile_zero_window_elapsed_us,
                        full_window_nodes: profile_full_window_nodes,
                        full_window_elapsed_us: profile_full_window_elapsed_us,
                        pvs_research: profile_pvs_research,
                    });
                }
                if root_win {
                    early_exit = true;
                    break;
                }
                if beta_cutoff {
                    if self.nnue.is_some() && candidate.self_attack < 4 {
                        let player = if side == BLACK { 0 } else { 1 };
                        let bonus = ((depth * depth * 4.0) as i32).clamp(1, 400);
                        update_quiet_history(
                            &mut self.quiet_history[player][candidate.move_ as usize],
                            bonus,
                        );
                        for &prev_mv in &searched_quiets[..searched_quiets_len] {
                            update_quiet_history(
                                &mut self.quiet_history[player][prev_mv as usize],
                                -bonus,
                            );
                        }
                    }
                    hash_flag = HASHF_BETA;
                    stats.cutoffs += 1;
                    early_exit = true;
                    break;
                }
                if self.nnue.is_some()
                    && candidate.self_attack < 4
                    && searched_quiets_len < searched_quiets.len()
                {
                    searched_quiets[searched_quiets_len] = candidate.move_;
                    searched_quiets_len += 1;
                }
            }
        }

        if !has_score {
            current = -INF - 1;
        }
        if current <= original_alpha && hash_flag != HASHF_BETA {
            hash_flag = HASHF_ALPHA;
        }
        if stats.stop {
            return (current, best_move);
        }

        if !root_restricted && !in_singular {
            let mut store_depth = hash_depth;
            if (current >= WIN && current > original_alpha)
                || (current <= -WIN && current < original_beta)
            {
                hash_flag = HASHF_EXACT;
                store_depth += if subply_tt { SUBPLY_TT_WIN_BONUS } else { 10 };
            }
            if let Some(tt4) = tt_view4 {
                tt4.store(TTEntry4 {
                    key: board.zobrist_key(),
                    value: score_to_tt(current, options.ply),
                    static_eval: tt_static_eval.unwrap_or(TT_VALUE_NONE),
                    flag: hash_flag,
                    depth: store_depth,
                    best_move,
                    tt_pv: is_pv || tt_was_pv,
                    generation: self.tt_generation,
                });
            } else if let Some(tt2) = tt_view {
                tt2.store(TTEntry {
                    key: board.zobrist_key(),
                    value: score_to_tt(current, options.ply),
                    flag: hash_flag,
                    depth: store_depth,
                    priority: if subply_tt {
                        priority_base * 256 + hash_depth
                    } else {
                        priority_base * 10 + hash_depth
                    },
                    best_move,
                    generation: self.tt_generation,
                });
            }
        }
        (current, best_move)
    }

    #[inline(always)]
    fn qvcf_check_stop(&mut self, stats: &mut SearchStats, abdada: Option<&AbdadaContext>) -> bool {
        let next_node = stats.nodes + 1;
        if stats
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            stats.stop = true;
            return true;
        }
        if (next_node & 0x3F) == 0 && abdada.is_some_and(AbdadaContext::is_stopped) {
            stats.stop = true;
            return true;
        }
        if stats.node_limit.is_some_and(|limit| stats.nodes >= limit) {
            stats.stop = true;
            return true;
        }
        if stats.deadline.is_some_and(|deadline| {
            (next_node == 1 || (next_node & stats.time_check_mask) == 0)
                && Instant::now() >= deadline
        }) {
            stats.stop = true;
            return true;
        }
        if !self.vcf.budget.enter() {
            stats.stop = true;
            return true;
        }
        stats.nodes = next_node;
        false
    }

    #[inline(always)]
    fn qvcf_delta_margin(&self, qdepth: i32) -> i32 {
        match self.config.rule_set {
            RuleSet::Renju => (2154 + 53 * qdepth).max(768),
            RuleSet::Freestyle => (2084 + 62 * qdepth).max(768),
        }
    }

    /// Attack-side leaf quiescence VCF search (`vcfsearch`) for NNUE mode.
    /// Evaluates stand-pat NNUE value and extends forcing B4 sequences so leaf
    /// horizons do not miss tactical wins or counter-VCFs.
    #[allow(clippy::too_many_arguments)]
    fn vcfsearch(
        &mut self,
        board: &mut Board,
        caches: &mut EvalCaches,
        side: Side,
        mut alpha: i32,
        mut beta: i32,
        qdepth: i32,
        ply: i32,
        stats: &mut SearchStats,
        coverage: &mut CoverageTracker,
        abdada: Option<&AbdadaContext>,
        tt_view4: Option<TTTableView4<'_>>,
        pre_probe: Option<(bool, Option<i32>, u8, Option<Move>, Option<i32>)>,
    ) -> i32 {
        if pre_probe.is_none() {
            if self.qvcf_check_stop(stats, abdada) {
                return 0;
            }
            if let Some(score) = terminal_score(board, side, ply) {
                return score;
            }
            if board.move_count() >= BOARD_AREA {
                return 0;
            }
        }
        if ply >= QVCF_MAX_PLY {
            return self
                .nnue
                .as_mut()
                .expect("vcfsearch requires NNUE")
                .evaluate_value(side);
        }

        // O(1) immediate win / counter-four defense check.
        if caches.has_l5(side) {
            return INF - ply;
        }
        if caches.has_l5(-side) {
            return self.vcfdefend(
                board,
                caches,
                side,
                alpha,
                beta,
                qdepth - 1,
                ply,
                stats,
                coverage,
                abdada,
                tt_view4,
            );
        }
        if caches.has_l4(side) && board.move_count() + 2 < BOARD_AREA {
            return INF - (ply + 2);
        }

        // Mate distance pruning.
        alpha = alpha.max(-INF + ply);
        beta = beta.min(INF - ply - 1);
        if alpha >= beta {
            return alpha;
        }

        let is_pv = beta - alpha > 1;
        let old_alpha = alpha;
        let qvcf_tt_depth = if qdepth >= -1 {
            QVCF_TT_DEPTH_FULL
        } else {
            QVCF_TT_DEPTH_COMB
        };

        let (tt_hit, probe_tt_value, probe_tt_flag, tt_move, tt_static_eval) =
            if let Some(pre) = pre_probe {
                pre
            } else if let Some(tt4) = tt_view4 {
                let p4 = tt4.probe(
                    board.zobrist_key(),
                    qvcf_tt_depth,
                    score_to_tt(alpha, ply),
                    score_to_tt(beta, ply),
                );
                if p4.hit && p4.value.is_some() {
                    stats.tt_hits += 1;
                    return score_from_tt(p4.value.expect("checked above"), ply);
                }
                (
                    p4.hit,
                    p4.tt_value,
                    p4.tt_flag,
                    p4.best_move,
                    p4.static_eval,
                )
            } else {
                (false, None, HASHF_EMPTY, None, None)
            };

        // Stand-pat NNUE evaluation, refined with cached TT bound when sharper.
        let raw_eval = match tt_static_eval {
            Some(ev) => ev,
            None => self
                .nnue
                .as_mut()
                .expect("vcfsearch requires NNUE")
                .evaluate_value(side),
        };
        let mut best_value = raw_eval;
        if let Some(tt_val) = probe_tt_value {
            let adj = score_from_tt(tt_val, ply);
            if adj.abs() < WIN
                && (probe_tt_flag == HASHF_EXACT
                    || (probe_tt_flag == HASHF_BETA && adj > best_value)
                    || (probe_tt_flag == HASHF_ALPHA && adj < best_value))
            {
                best_value = adj;
            }
        }

        // Stand-pat cutoff.
        if best_value >= beta {
            if !tt_hit && tt_static_eval.is_none() {
                if let Some(tt4) = tt_view4 {
                    tt4.store(TTEntry4 {
                        key: board.zobrist_key(),
                        value: TT_VALUE_NONE,
                        static_eval: raw_eval,
                        flag: HASHF_EMPTY,
                        depth: 0,
                        best_move: None,
                        tt_pv: false,
                        generation: self.tt_generation,
                    });
                }
            }
            return best_value;
        }
        if is_pv && best_value > alpha {
            alpha = best_value;
        }

        // Non-PV delta pruning.
        if !is_pv && best_value + self.qvcf_delta_margin(qdepth) < alpha {
            if !tt_hit && tt_static_eval.is_none() {
                if let Some(tt4) = tt_view4 {
                    tt4.store(TTEntry4 {
                        key: board.zobrist_key(),
                        value: TT_VALUE_NONE,
                        static_eval: raw_eval,
                        flag: HASHF_EMPTY,
                        depth: 0,
                        best_move: None,
                        tt_pv: false,
                        generation: self.tt_generation,
                    });
                }
            }
            return alpha;
        }

        // O(1) depth cap or no attacker B4 available.
        let has_any_b4 = caches.has_attack4_plus(side);
        let self_last = if has_any_b4 {
            board
                .move_history()
                .iter()
                .rev()
                .find(|played| played.side == side)
                .map(|played| played.move_ as usize)
        } else {
            None
        };
        if qdepth <= -QVCF_MAX_STEPS || !has_any_b4 || self_last.is_none() {
            if let Some(tt4) = tt_view4 {
                if !has_any_b4 || qdepth <= -QVCF_MAX_STEPS {
                    let flag = if is_pv && best_value > old_alpha {
                        HASHF_EXACT
                    } else {
                        HASHF_ALPHA
                    };
                    tt4.store(TTEntry4 {
                        key: board.zobrist_key(),
                        value: score_to_tt(best_value, ply),
                        static_eval: raw_eval,
                        flag,
                        depth: qvcf_tt_depth,
                        best_move: None,
                        tt_pv: is_pv,
                        generation: self.tt_generation,
                    });
                } else if !tt_hit && tt_static_eval.is_none() {
                    tt4.store(TTEntry4 {
                        key: board.zobrist_key(),
                        value: TT_VALUE_NONE,
                        static_eval: raw_eval,
                        flag: HASHF_EMPTY,
                        depth: 0,
                        best_move: None,
                        tt_pv: false,
                        generation: self.tt_generation,
                    });
                }
            }
            return best_value;
        }

        // Collect legal local B4 forcing moves in `RANGE_SQUARE2_LINE4` around
        // `self_last` directly from the O(1) bitboard (plus valid TT move).
        // For `qdepth <= -2`, restrict to combo B4s (`bucket >= QVCF_COMB_MIN_BUCKET`)
        // to match Rapfi's `DEPTH_QVCF_FULL = -1` / `VCF | COMB` discipline.
        let player = if side == BLACK { 0 } else { 1 };
        let allow_plain_b4 = qdepth >= -1;
        let last_move = self_last.expect("checked above");
        let neighbor_mask = &QVCF_NEIGHBOR_MASKS[last_move];
        let mut mask = [
            caches.attack4_mask[player][0] & neighbor_mask[0],
            caches.attack4_mask[player][1] & neighbor_mask[1],
            caches.attack4_mask[player][2] & neighbor_mask[2],
            caches.attack4_mask[player][3] & neighbor_mask[3],
        ];
        // Track whether the `self_last` neighborhood filter dropped any legal
        // B4 outside the local mask. Dropping attacker moves can only
        // underestimate the attacker's score, so when `nonlocal_b4_dropped` is
        // true, lower bounds (`HASHF_BETA` / proven wins) remain sound on the
        // pure board `zobrist_key`, while upper/exact fail-low bounds are
        // path-dependent and are stored/probed under a `self_last`-salted key.
        let nonlocal_b4_dropped = ((caches.attack4_mask[player][0] & !mask[0])
            | (caches.attack4_mask[player][1] & !mask[1])
            | (caches.attack4_mask[player][2] & !mask[2])
            | (caches.attack4_mask[player][3] & !mask[3]))
            != 0;
        let mut local_tt_move = tt_move;
        let salted_key = if nonlocal_b4_dropped {
            let key = board.zobrist_key() ^ qvcf_self_last_salt(last_move);
            if let Some(tt4) = tt_view4 {
                let ps = tt4.probe(
                    key,
                    qvcf_tt_depth,
                    score_to_tt(alpha, ply),
                    score_to_tt(beta, ply),
                );
                if ps.hit && ps.value.is_some() {
                    stats.tt_hits += 1;
                    return score_from_tt(ps.value.expect("checked above"), ply);
                }
                if local_tt_move.is_none() {
                    local_tt_move = ps.best_move;
                }
            }
            Some(key)
        } else {
            None
        };
        if let Some(tt_mv) = local_tt_move {
            let idx = tt_mv as usize;
            if idx < BOARD_AREA
                && (caches.attack4_mask[player][idx >> 6] & (1_u64 << (idx & 63))) != 0
            {
                mask[idx >> 6] |= 1_u64 << (idx & 63);
            }
        }
        let mut b4_moves = [(0_u16, 0_i32); 32];
        let mut b4_len = 0_usize;
        for (word_idx, mut word) in mask.into_iter().enumerate() {
            let base = word_idx << 6;
            while word != 0 {
                let idx = base + word.trailing_zeros() as usize;
                word &= word - 1;
                if idx >= BOARD_AREA {
                    break;
                }
                let mv = idx as Move;
                let mx = idx % BOARD_SIZE;
                let my = idx / BOARD_SIZE;
                let bucket = caches.value_cache[player][mx][my];
                if !allow_plain_b4 && Some(mv) != local_tt_move && bucket < QVCF_COMB_MIN_BUCKET {
                    continue;
                }
                debug_assert_eq!(board.grid_rows()[my][mx], EMPTY);
                debug_assert!(is_rule_legal_for_movegen(
                    board,
                    caches,
                    mv,
                    side,
                    self.config.rule_set
                ));
                let score = if Some(mv) == local_tt_move {
                    i32::MAX
                } else {
                    bucket
                };
                if b4_len < b4_moves.len() {
                    b4_moves[b4_len] = (mv, score);
                    b4_len += 1;
                }
            }
        }
        // Small insertion sort descending by score (typically 1..5 moves).
        for i in 1..b4_len {
            let cur = b4_moves[i];
            let mut j = i;
            while j > 0 && b4_moves[j - 1].1 < cur.1 {
                b4_moves[j] = b4_moves[j - 1];
                j -= 1;
            }
            b4_moves[j] = cur;
        }

        let mut best_move: Option<Move> = None;
        for &(mv, _) in &b4_moves[..b4_len] {
            let (mx, my) = move_to_xy(mv).expect("B4 move valid");
            let snapshot = caches.snapshot();
            board
                .play_assuming_rule_legal(mv, Some(side), self.config.rule_set)
                .expect("B4 move stays legal");
            if let Some(tt4) = tt_view4 {
                tt4.prefetch(board.zobrist_key());
            }
            coverage.add_move(mv);
            if let Some(nnue) = self.nnue.as_mut() {
                nnue.before_move(side, mx, my);
            }
            value_wide_compute_for_rule(board, caches, (mx, my), self.config.rule_set);

            let child_score = -self.vcfdefend(
                board,
                caches,
                -side,
                -beta,
                -alpha,
                qdepth - 1,
                ply + 1,
                stats,
                coverage,
                abdada,
                tt_view4,
            );

            board.undo().expect("B4 move undo matches play");
            if let Some(nnue) = self.nnue.as_mut() {
                nnue.after_undo(side, mx, my);
            }
            coverage.remove_move(mv);
            caches.restore_snapshot(&snapshot);

            if stats.stop {
                return 0;
            }
            if child_score > best_value {
                best_value = child_score;
                if child_score > alpha {
                    best_move = Some(mv);
                    if is_pv && child_score < beta {
                        alpha = child_score;
                    } else {
                        break;
                    }
                }
            }
        }

        if let Some(tt4) = tt_view4 {
            let mut flag = if best_value >= beta {
                HASHF_BETA
            } else if is_pv && best_value > old_alpha {
                HASHF_EXACT
            } else {
                HASHF_ALPHA
            };
            if (best_value >= WIN && best_value > old_alpha)
                || (best_value <= -WIN && best_value < beta)
            {
                flag = HASHF_EXACT;
            }
            let safe_for_board_key = !nonlocal_b4_dropped
                || flag == HASHF_BETA
                || (best_value >= WIN && best_value > old_alpha);
            if safe_for_board_key {
                tt4.store(TTEntry4 {
                    key: board.zobrist_key(),
                    value: score_to_tt(best_value, ply),
                    static_eval: raw_eval,
                    flag,
                    depth: qvcf_tt_depth,
                    best_move,
                    tt_pv: is_pv,
                    generation: self.tt_generation,
                });
            } else {
                if let Some(key) = salted_key {
                    tt4.store(TTEntry4 {
                        key,
                        value: score_to_tt(best_value, ply),
                        static_eval: raw_eval,
                        flag,
                        depth: qvcf_tt_depth,
                        best_move,
                        tt_pv: is_pv,
                        generation: self.tt_generation,
                    });
                }
                if !tt_hit && tt_static_eval.is_none() {
                    tt4.store(TTEntry4 {
                        key: board.zobrist_key(),
                        value: TT_VALUE_NONE,
                        static_eval: raw_eval,
                        flag: HASHF_EMPTY,
                        depth: 0,
                        best_move: None,
                        tt_pv: false,
                        generation: self.tt_generation,
                    });
                }
            }
        }

        best_value
    }

    /// Defense-side leaf quiescence VCF search (`vcfdefend`) for NNUE mode.
    #[allow(clippy::too_many_arguments)]
    fn vcfdefend(
        &mut self,
        board: &mut Board,
        caches: &mut EvalCaches,
        side: Side,
        alpha: i32,
        beta: i32,
        qdepth: i32,
        ply: i32,
        stats: &mut SearchStats,
        coverage: &mut CoverageTracker,
        abdada: Option<&AbdadaContext>,
        tt_view4: Option<TTTableView4<'_>>,
    ) -> i32 {
        if self.qvcf_check_stop(stats, abdada) {
            return 0;
        }
        if let Some(score) = terminal_score(board, side, ply) {
            return score;
        }
        if board.move_count() >= BOARD_AREA {
            return 0;
        }
        if ply >= QVCF_MAX_PLY {
            return self
                .nnue
                .as_mut()
                .expect("vcfdefend requires NNUE")
                .evaluate_value(side);
        }

        // Immediate counter-win beats opponent's pending four.
        if caches.has_l5(side) {
            return INF - ply;
        }

        let opp_l5_count = caches.l5_count(-side);
        if opp_l5_count == 0 {
            return self
                .nnue
                .as_mut()
                .expect("vcfdefend requires NNUE")
                .evaluate_value(side);
        }
        if opp_l5_count >= 2 {
            return -INF + ply + 2;
        }

        let mv = caches.first_l5_move(-side).expect("opp_l5_count == 1");
        if self.config.rule_set == RuleSet::Renju
            && side == BLACK
            && !is_rule_legal_for_movegen(board, caches, mv, side, self.config.rule_set)
        {
            return -INF + ply + 2;
        }

        let (mx, my) = move_to_xy(mv).expect("forced block valid");
        let snapshot = caches.snapshot();
        board
            .play_assuming_rule_legal(mv, Some(side), self.config.rule_set)
            .expect("forced block stays legal");
        if let Some(tt4) = tt_view4 {
            tt4.prefetch(board.zobrist_key());
        }
        coverage.add_move(mv);
        if let Some(nnue) = self.nnue.as_mut() {
            nnue.before_move(side, mx, my);
        }
        value_wide_compute_for_rule(board, caches, (mx, my), self.config.rule_set);

        let value = -self.vcfsearch(
            board,
            caches,
            -side,
            -beta,
            -alpha,
            qdepth,
            ply + 1,
            stats,
            coverage,
            abdada,
            tt_view4,
            None,
        );

        board.undo().expect("forced block undo matches play");
        if let Some(nnue) = self.nnue.as_mut() {
            nnue.after_undo(side, mx, my);
        }
        coverage.remove_move(mv);
        caches.restore_snapshot(&snapshot);

        if stats.stop {
            return 0;
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::config::load_default_config;
    use crate::eval::recompute_all_for_rule;
    use crate::rules::RuleSet;

    #[test]
    fn subply_tt_shallower_depth_never_cuts_off_deeper_search() {
        let mut board = Board::new();
        for (x, y) in [(7, 7), (7, 6), (8, 8), (6, 6), (6, 8), (5, 8)] {
            let m = xy_to_move(x, y).unwrap();
            board.play_for_rule(m, None, RuleSet::Freestyle).unwrap();
        }
        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.threads = 2;
        cfg.runtime.compute_vcf = false;
        cfg.runtime.compute_vct = false;

        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut caches, RuleSet::Freestyle);
        let side = board.side_to_move();

        // Cold search at depth 1.10.
        let mut cold_searcher = AlphaBetaSearcher::new(cfg.clone());
        let mut cold_stats = SearchStats::default();
        let (cold_score, cold_move) = cold_searcher.search(
            &mut board.clone(),
            &mut caches.clone(),
            side,
            1.10,
            -INF,
            INF,
            12,
            &mut cold_stats,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert_eq!(move_to_xy(cold_move.unwrap()).unwrap(), (8, 6));
        assert_eq!(cold_score, 9);

        // Warm search: first depth 1.00, then depth 1.10 reusing the same TT.
        let mut warm_searcher = AlphaBetaSearcher::new(cfg);
        let mut stats_100 = SearchStats::default();
        let (score_100, move_100) = warm_searcher.search(
            &mut board.clone(),
            &mut caches.clone(),
            side,
            1.00,
            -INF,
            INF,
            12,
            &mut stats_100,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert_eq!(move_to_xy(move_100.unwrap()).unwrap(), (8, 7));
        assert_eq!(score_100, 16);

        let mut stats_110 = SearchStats::default();
        let (warm_score, warm_move) = warm_searcher.search(
            &mut board.clone(),
            &mut caches.clone(),
            side,
            1.10,
            -INF,
            INF,
            12,
            &mut stats_110,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert!(
            stats_110.nodes > 1,
            "shallower depth 1.00 must not cut off depth 1.10"
        );
        assert_eq!(warm_move, cold_move);
        assert_eq!(warm_score, cold_score);

        // Exact same depth (1.00 -> 1.00 on a fresh searcher) still hits TT.
        let mut stats_same = SearchStats::default();
        let (same_score, same_move) = warm_searcher.search(
            &mut board.clone(),
            &mut caches.clone(),
            side,
            1.00,
            -INF,
            INF,
            12,
            &mut stats_same,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert_eq!(stats_same.nodes, 1);
        assert_eq!(same_move, cold_move);
        assert_eq!(same_score, cold_score);

        // Same 1/16 bucket (1.563125 -> 1.625) and same 1/64 bucket (1.6095 -> 1.625)
        // must also never let a shallower search cut off a deeper search.
        let mut board2 = board.clone();
        board2
            .play_for_rule(xy_to_move(9, 9).unwrap(), None, RuleSet::Freestyle)
            .unwrap();
        let mut caches2 = EvalCaches::new();
        recompute_all_for_rule(&mut board2, &mut caches2, RuleSet::Freestyle);
        let side2 = board2.side_to_move();

        let mut cold_1625 = AlphaBetaSearcher::new(warm_searcher.config.clone());
        let mut cold_1625_stats = SearchStats::default();
        let (cold_1625_score, cold_1625_move) = cold_1625.search(
            &mut board2.clone(),
            &mut caches2.clone(),
            side2,
            1.625,
            -INF,
            INF,
            12,
            &mut cold_1625_stats,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );

        for shallow_d in [1.563125, 1.6095] {
            let mut warm_1625 = AlphaBetaSearcher::new(warm_searcher.config.clone());
            let mut shallow_stats = SearchStats::default();
            let (shallow_score, _) = warm_1625.search(
                &mut board2.clone(),
                &mut caches2.clone(),
                side2,
                shallow_d,
                -INF,
                INF,
                12,
                &mut shallow_stats,
                SearchOptions {
                    opo: 1,
                    root: true,
                    ..SearchOptions::default()
                },
            );
            if shallow_d == 1.563125 {
                assert_ne!(
                    shallow_score, cold_1625_score,
                    "shallow {shallow_d} should differ from 1.625 on this fixture"
                );
            }

            let mut warm_1625_stats = SearchStats::default();
            let (warm_1625_score, warm_1625_move) = warm_1625.search(
                &mut board2.clone(),
                &mut caches2.clone(),
                side2,
                1.625,
                -INF,
                INF,
                12,
                &mut warm_1625_stats,
                SearchOptions {
                    opo: 1,
                    root: true,
                    ..SearchOptions::default()
                },
            );
            assert!(
                warm_1625_stats.nodes > 1,
                "shallower depth {shallow_d} must not cut off depth 1.625"
            );
            assert_eq!(warm_1625_move, cold_1625_move);
            assert_eq!(warm_1625_score, cold_1625_score);
        }
    }

    #[test]
    fn multithread_deep_search_above_d12_does_not_saturate_tt_depth_cap() {
        let mut board = Board::new();
        board
            .play_for_rule(xy_to_move(7, 7).unwrap(), None, RuleSet::Freestyle)
            .unwrap();
        let limits = crate::search::SearchLimits {
            max_depth: 14,
            root_width: 2,
            node_limit: None,
            time_limit_ms: None,
        };

        let mut cfg_1t = load_default_config();
        cfg_1t.rule_set = RuleSet::Freestyle;
        cfg_1t.runtime.threads = 1;
        cfg_1t.runtime.compute_vcf = false;
        cfg_1t.runtime.compute_vct = false;
        let mut s1 = crate::search::RootSearcher::new(cfg_1t.clone());
        let res_1t = s1.search(&mut board.clone(), Some(limits));

        let mut cfg_2t = cfg_1t;
        cfg_2t.runtime.threads = 2;
        cfg_2t.runtime.root_profile = true;
        let mut s2 = crate::search::RootSearcher::new(cfg_2t);
        let res_2t = s2.search(&mut board, Some(limits));
        assert_eq!(res_2t.depth, 14);
        assert_eq!(res_2t.score, res_1t.score);
        let trace = s2.last_trace.as_ref().expect("trace recorded");
        assert_eq!(trace.root_profiles.len(), 14);
        for d in [13, 14] {
            let prof = &trace.root_profiles[(d - 1) as usize];
            assert_eq!(prof.depth, d);
            assert!(
                prof.nodes > 10,
                "depth {d} should genuinely expand tree, got only {} nodes",
                prof.nodes
            );
        }
    }

    #[test]
    fn depth_above_50_uses_tt_move_hint_only_without_saturated_cutoff() {
        let mut board = Board::new();
        for (x, y) in [(7, 7), (7, 6), (8, 8), (6, 6), (6, 8), (5, 8)] {
            let m = xy_to_move(x, y).unwrap();
            board.play_for_rule(m, None, RuleSet::Freestyle).unwrap();
        }
        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.threads = 2;
        cfg.runtime.compute_vcf = false;
        cfg.runtime.compute_vct = false;

        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut caches, RuleSet::Freestyle);
        let side = board.side_to_move();

        let mut searcher = AlphaBetaSearcher::new(cfg);
        searcher.tt.set_subply(true);
        // Store a normal d50 record (encoded at the saturation cap 3200).
        searcher.tt.store(TTEntry {
            key: board.zobrist_key(),
            value: 95,
            flag: HASHF_EXACT,
            depth: SUBPLY_TT_DEPTH_CAP,
            priority: 1000,
            best_move: Some(xy_to_move(8, 7).unwrap()),
            generation: 1,
        });

        // Request d51 (with node_limit so it stops quickly after expanding root):
        // it must NOT cut off at 1 node from the d50 record.
        let mut stats = SearchStats {
            node_limit: Some(32),
            ..SearchStats::default()
        };
        let _ = searcher.search(
            &mut board,
            &mut caches,
            side,
            51.0,
            -INF,
            INF,
            2,
            &mut stats,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert!(
            stats.nodes > 1,
            "d51 search must not cut off at 1 node from a saturated d50 TT entry"
        );
        assert_eq!(stats.tt_hits, 0);
    }

    #[test]
    fn concurrent_and_interleaved_1t_and_mt_searches_on_shared_tt_stay_isolated() {
        let mut board = Board::new();
        for (x, y) in [(7, 7), (7, 6), (8, 8), (6, 6), (6, 8), (5, 8)] {
            let m = xy_to_move(x, y).unwrap();
            board.play_for_rule(m, None, RuleSet::Freestyle).unwrap();
        }
        let limits_d7 = crate::search::SearchLimits {
            max_depth: 7,
            root_width: 10,
            node_limit: None,
            time_limit_ms: None,
        };
        let limits_d5 = crate::search::SearchLimits {
            max_depth: 5,
            root_width: 10,
            node_limit: None,
            time_limit_ms: None,
        };

        let mut cfg_1t = load_default_config();
        cfg_1t.rule_set = RuleSet::Freestyle;
        cfg_1t.runtime.threads = 1;
        cfg_1t.runtime.compute_vcf = false;
        cfg_1t.runtime.compute_vct = false;

        // Cold 1T d7 baseline with the same 16-bit TT capacity.
        let mut cold_1t =
            crate::search::RootSearcher::with_tt(cfg_1t.clone(), TranspositionTable::new(16));
        let cold_res = cold_1t.search(&mut board.clone(), Some(limits_d7));
        assert!(cold_res.nodes > 100);

        // Shared TT between a 4T searcher and a 1T searcher running concurrently.
        let shared_tt = TranspositionTable::new(16);
        let mut cfg_4t = cfg_1t.clone();
        cfg_4t.runtime.threads = 4;

        let (res_1t, _) = std::thread::scope(|s| {
            let b1 = board.clone();
            let b4 = board.clone();
            let tt1 = shared_tt.clone();
            let tt4 = shared_tt.clone();
            let c1 = cfg_1t.clone();
            let c4 = cfg_4t.clone();

            let h4 = s.spawn(move || {
                let mut s4 = crate::search::RootSearcher::with_tt(c4, tt4);
                for _ in 0..3 {
                    let _ = s4.search(&mut b4.clone(), Some(limits_d5));
                }
            });
            let h1 = s.spawn(move || {
                let mut s1 = crate::search::RootSearcher::with_tt(c1, tt1);
                s1.search(&mut b1.clone(), Some(limits_d7))
            });
            let r1 = h1.join().unwrap();
            h4.join().unwrap();
            (r1, ())
        });

        assert_eq!(res_1t.move_, cold_res.move_);
        assert_eq!(res_1t.score, cold_res.score);
        assert_eq!(res_1t.nodes, cold_res.nodes);
    }

    #[test]
    fn single_thread_sub_one_depth_stores_zero_and_never_cuts_off_depth_one() {
        let mut board = Board::new();
        for (x, y) in [(7, 7), (7, 6), (8, 8), (6, 6), (6, 8), (5, 8)] {
            let m = xy_to_move(x, y).unwrap();
            board.play_for_rule(m, None, RuleSet::Freestyle).unwrap();
        }
        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.threads = 1;
        cfg.runtime.compute_vcf = false;
        cfg.runtime.compute_vct = false;

        let mut caches = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut caches, RuleSet::Freestyle);
        let side = board.side_to_move();

        let mut cold_10 = AlphaBetaSearcher::new(cfg.clone());
        let mut cold_stats = SearchStats::default();
        let (cold_score, cold_move) = cold_10.search(
            &mut board.clone(),
            &mut caches.clone(),
            side,
            1.0,
            -INF,
            INF,
            12,
            &mut cold_stats,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert!(cold_stats.nodes > 1);

        let mut warm = AlphaBetaSearcher::new(cfg);
        let mut stats_025 = SearchStats::default();
        let _ = warm.search(
            &mut board.clone(),
            &mut caches.clone(),
            side,
            0.25,
            -INF,
            INF,
            12,
            &mut stats_025,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );

        let mut warm_stats = SearchStats::default();
        let (warm_score, warm_move) = warm.search(
            &mut board,
            &mut caches,
            side,
            1.0,
            -INF,
            INF,
            12,
            &mut warm_stats,
            SearchOptions {
                opo: 1,
                root: true,
                ..SearchOptions::default()
            },
        );
        assert!(
            warm_stats.nodes > 1,
            "1T 0.25-ply search must store depth 0 and never cut off depth 1.0"
        );
        assert_eq!(warm_move, cold_move);
        assert_eq!(warm_score, cold_score);
    }

    #[test]
    fn multithread_overlap_vct_alphabeta_does_not_allocate_secondary_tt_plane() {
        let mut board = Board::new();
        // Create a board where Black has a double-three VCT trigger at (7,7)
        // (`(5,7)-(6,7)` and `(7,5)-(7,6)`), while White's stones are scattered
        // so VCF cannot solve the position and VCT at depth 1 also misses (forcing
        // overlap search to run both VCT and the overlapped Alpha-Beta worker).
        for (x, y) in [
            (5, 7),
            (0, 0),
            (6, 7),
            (0, 1),
            (7, 5),
            (14, 14),
            (7, 6),
            (14, 13),
        ] {
            let m = xy_to_move(x, y).unwrap();
            board.play_for_rule(m, None, RuleSet::Freestyle).unwrap();
        }
        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.threads = 4;
        cfg.runtime.overlap_vct_alphabeta = true;
        cfg.runtime.root_vct_depth = 1;

        let tt = TranspositionTable::new(16);
        let initial_tt = tt.clone();
        let mut searcher = crate::search::RootSearcher::with_tt(cfg, tt);
        assert!(searcher.tt.is_subply());
        assert!(!initial_tt.has_secondary_buckets());
        assert!(!searcher.tt.has_secondary_buckets());

        let _ = searcher.search(
            &mut board,
            Some(crate::search::SearchLimits {
                max_depth: 5,
                root_width: 6,
                node_limit: None,
                time_limit_ms: None,
            }),
        );
        let trace = searcher.last_trace.as_ref().expect("trace recorded");
        assert!(
            trace.overlap_used,
            "expected search_vct_and_alphabeta_overlap to run, trace={trace:?}"
        );
        assert!(searcher.tt.is_subply());
        assert!(
            !initial_tt.has_secondary_buckets(),
            "original TT must not allocate secondary TT plane during overlap snapshot"
        );
        assert!(
            !searcher.tt.has_secondary_buckets(),
            "adopted worker TT must not allocate secondary TT plane during overlap search"
        );
    }

    #[test]
    fn same_mode_concurrent_first_use_shares_primary_plane_without_allocating_secondary() {
        for subply in [false, true] {
            let mut tt = TranspositionTable::new(12);
            tt.set_subply(subply);
            let barrier = std::sync::Barrier::new(2);
            let k1 = 0x1111_2222_3333_0001_u64;
            let k2 = 0x4444_5555_6666_0002_u64;

            std::thread::scope(|s| {
                let tt1 = tt.clone();
                let tt2 = tt.clone();
                let b1 = &barrier;
                let b2 = &barrier;
                s.spawn(move || {
                    b1.wait();
                    tt1.store(TTEntry {
                        key: k1,
                        value: 17,
                        flag: HASHF_EXACT,
                        depth: 64,
                        priority: 100,
                        best_move: Some(10),
                        generation: 1,
                    });
                });
                s.spawn(move || {
                    b2.wait();
                    tt2.store(TTEntry {
                        key: k2,
                        value: 29,
                        flag: HASHF_EXACT,
                        depth: 64,
                        priority: 100,
                        best_move: Some(20),
                        generation: 1,
                    });
                });
            });

            assert!(
                !tt.has_secondary_buckets(),
                "concurrent first use in the same mode (subply={subply}) must not allocate secondary plane"
            );
            let p1 = tt.probe(k1, 64, -INF, INF);
            let p2 = tt.probe(k2, 64, -INF, INF);
            assert!(p1.hit && p1.value == Some(17));
            assert!(p2.hit && p2.value == Some(29));
        }
    }

    #[test]
    fn qvcf_self_last_salts_are_unique_and_nonzero() {
        let mut seen = std::collections::HashSet::with_capacity(BOARD_AREA);
        for sq in 0..BOARD_AREA {
            let salt = qvcf_self_last_salt(sq);
            assert_ne!(salt, 0);
            assert!(seen.insert(salt), "duplicate salt at square {sq}");
        }
    }

    #[test]
    fn qvcf_leaf_is_move_order_invariant_and_does_not_poison_tt() {
        // Real game `renjunet_center_g151985_ply6` after ply 28:
        // Black wins by forced B4 `(4,10) -> (5,9) -> (5,10)` (open four).
        // Swapping black moves #25 `(2,5)` and #27 `(6,10)` produces the exact
        // same board and side to move, and must yield the same winning mate
        // both on a cold TT and on a warm shared TT.
        let nnue_dir = std::path::Path::new("opponent/rapfi-networks/mix9svq");
        if !nnue_dir.exists() {
            return;
        }
        let orig_moves: [(usize, usize); 28] = [
            (7, 7),
            (7, 6),
            (6, 7),
            (8, 7),
            (6, 5),
            (6, 6),
            (5, 6),
            (4, 7),
            (4, 5),
            (3, 4),
            (6, 8),
            (5, 8),
            (8, 6),
            (9, 5),
            (7, 5),
            (5, 5),
            (7, 9),
            (6, 9),
            (7, 8),
            (8, 9),
            (7, 10),
            (7, 11),
            (9, 7),
            (3, 6),
            (2, 5),
            (10, 8),
            (6, 10),
            (8, 8),
        ];
        let mut swapped_moves = orig_moves;
        swapped_moves.swap(24, 26);

        let build_board = |moves: &[(usize, usize)]| {
            let mut b = Board::new();
            for &(x, y) in moves {
                let mv = xy_to_move(x, y).unwrap();
                b.play_for_rule(mv, None, RuleSet::Renju).unwrap();
            }
            b
        };
        let mut board_orig = build_board(&orig_moves);
        let mut board_swapped = build_board(&swapped_moves);
        assert_eq!(board_orig.zobrist_key(), board_swapped.zobrist_key());
        assert_eq!(board_orig.side_to_move(), BLACK);

        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Renju;
        cfg.runtime.use_nnue = true;
        cfg.runtime.nnue_dir = Some(nnue_dir.to_string_lossy().into_owned());
        cfg.runtime.compute_vcf = false;
        cfg.runtime.compute_vct = false;

        // Search the swapped order FIRST on a shared TT (where `self_last = (2,5)`
        // is outside the local neighborhood of `(4,10)`), then search the original
        // order (`self_last = (6,10)`) on the SAME shared TT. The swapped path
        // must NOT poison the shared TT for the original path!
        let mut caches_swapped = EvalCaches::new();
        recompute_all_for_rule(&mut board_swapped, &mut caches_swapped, RuleSet::Renju);
        let mut shared_searcher = AlphaBetaSearcher::new(cfg.clone());
        let mut stats_swapped = SearchStats::default();
        let (_score_swapped, _) = shared_searcher.search(
            &mut board_swapped,
            &mut caches_swapped,
            BLACK,
            0.0,
            -INF,
            INF,
            40,
            &mut stats_swapped,
            SearchOptions {
                opo: 1,
                root: false,
                ..SearchOptions::default()
            },
        );

        let mut caches_orig = EvalCaches::new();
        recompute_all_for_rule(&mut board_orig, &mut caches_orig, RuleSet::Renju);
        let mut stats_orig = SearchStats::default();
        let (score_orig, _) = shared_searcher.search(
            &mut board_orig,
            &mut caches_orig,
            BLACK,
            0.0,
            -INF,
            INF,
            40,
            &mut stats_orig,
            SearchOptions {
                opo: 1,
                root: false,
                ..SearchOptions::default()
            },
        );
        assert!(
            score_orig >= WIN,
            "swapped move order poisoned shared TT and hid forced win on original order: score_orig={score_orig}"
        );
    }
}
