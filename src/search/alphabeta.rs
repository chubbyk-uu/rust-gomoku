//! Alpha-beta search implementation for the classic mainline.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use crate::board::{move_to_xy, Board};
use crate::config::EngineConfig;
use crate::constants::{BOARD_AREA, HASHF_ALPHA, HASHF_BETA, HASHF_EXACT, INF, WIN};
use crate::eval::{evaluate_board, value_wide_compute_for_rule, EvalCaches, EvalSnapshot};
use crate::nnue::{load_mix9svq_evaluator_for_rule, Mix9svqEvaluator};
use crate::search::abdada::{AbdadaContext, ABDADA_MIN_DEPTH};
use crate::search::movegen::{
    generate_candidates_with_coverage_scratch, Candidate, CoverageTracker, MoveAnalysis,
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
}

fn load_nnue_for_config(config: &EngineConfig) -> Option<Mix9svqEvaluator> {
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
        let mut tt = TranspositionTable::default();
        tt.set_subply(config.runtime.threads > 1);
        let nnue = load_nnue_for_config(&config);
        Self {
            config,
            tt,
            vcf: VCFSearcher::default(),
            tt_generation: 0,
            movegen_scratch: MoveAnalysis::new(),
            nnue,
        }
    }

    pub fn with_tt(config: EngineConfig, mut tt: TranspositionTable) -> Self {
        tt.set_subply(config.runtime.threads > 1);
        let nnue = load_nnue_for_config(&config);
        Self {
            config,
            tt,
            vcf: VCFSearcher::default(),
            tt_generation: 0,
            movegen_scratch: MoveAnalysis::new(),
            nnue,
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
        self.tt.set_subply(self.config.runtime.threads > 1);
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
        self.tt.set_subply(self.config.runtime.threads > 1);
        let tt = self.tt.clone();
        let tt_view = tt.view();
        let tt_view4 = if self.nnue.is_some() {
            tt.view4()
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
        depth: f64,
        mut alpha: i32,
        mut beta: i32,
        wide: usize,
        stats: &mut SearchStats,
        options: SearchOptions<'_>,
        coverage: &mut CoverageTracker,
        pending_cache: Option<((usize, usize), &mut Option<EvalSnapshot>)>,
        abdada: Option<&AbdadaContext>,
        tt_view: TTTableView<'_>,
        tt_view4: Option<TTTableView4<'_>>,
    ) -> (i32, Option<Move>) {
        let root_depth = options.root_depth.unwrap_or(depth);
        let priority_base = options.priority_base.unwrap_or(board.move_count() as i32);
        let subply_tt = tt_view.is_subply();
        let max_encodable_depth = if subply_tt {
            SUBPLY_TT_DEPTH_CAP
        } else {
            INTEGER_TT_DEPTH_CAP
        };
        let raw_hash_depth = if depth <= 0.0 {
            0
        } else if subply_tt {
            ((depth * SUBPLY_TT_SCALE_F64).floor() as i64).clamp(0, i64::from(i32::MAX)) as i32
        } else {
            (depth.floor() as i64).clamp(0, i64::from(i32::MAX)) as i32
        };
        let hash_depth = raw_hash_depth.min(max_encodable_depth);
        let raw_probe_depth = if depth <= 0.0 {
            0
        } else if subply_tt {
            ((depth * SUBPLY_TT_SCALE_F64).ceil() as i64).clamp(1, i64::from(i32::MAX)) as i32
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
                p4.best_move,
                p4.best_move_hint,
                p4.has_window,
                p4.window_alpha,
                p4.window_beta,
                p4.static_eval,
                p4.tt_pv,
            )
        } else {
            let p2 = tt_view.probe(
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
                p2.best_move,
                p2.best_move_hint,
                p2.has_window,
                p2.window_alpha,
                p2.window_beta,
                None,
                false,
            )
        };
        if let Some(best_move_hint) = probe_best_move_hint {
            if best_move_hint.generation == self.tt_generation {
                stats.tt_bestmove_current_generation += 1;
            } else {
                stats.tt_bestmove_old_generation += 1;
            }
        }
        if !root_restricted && probe_hit && probe_value.is_some() {
            stats.tt_hits += 1;
            return (
                score_from_tt(probe_value.expect("checked above"), options.ply),
                probe_best_move,
            );
        }
        if probe_has_window && !options.root {
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
            } else {
                tt_view.store(TTEntry {
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

        let generated = generate_candidates_with_coverage_scratch(
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

        // Tactically-guarded NNUE Reverse Futility Pruning (RFP):
        // At shallow non-PV quiet nodes with no immediate win/forcing/open-3
        // threat, if the NNUE static evaluation (cached in 4-way TT) exceeds
        // beta by a generous depth-scaled margin, cut off without expanding
        // wide candidates.
        if let (Some(tt4), Some(nnue)) = (tt_view4, self.nnue.as_mut()) {
            if !options.root
                && !is_pv
                && !win_priority
                && !single_forcing
                && !hostile_threat
                && depth > 0.0
                && depth <= 3.0
                && beta > -WIN
                && beta < WIN
            {
                let raw_eval = match tt_static_eval {
                    Some(ev) => ev,
                    None => {
                        let ev = nnue.evaluate_value(side);
                        tt_static_eval = Some(ev);
                        tt4.store(TTEntry4 {
                            key: board.zobrist_key(),
                            value: TT_VALUE_NONE,
                            static_eval: ev,
                            flag: crate::constants::HASHF_EMPTY,
                            depth: 0,
                            best_move: None,
                            tt_pv: tt_was_pv,
                            generation: self.tt_generation,
                        });
                        ev
                    }
                };
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
                let rfp_margin = (135.0 * depth).round() as i32;
                if eval < WIN && eval - rfp_margin >= beta {
                    return (eval, None);
                }
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
            if let Some(pos) = tt_best_move.and_then(|tt_mv| {
                candidates
                    .iter()
                    .position(|candidate| candidate.move_ == tt_mv)
            }) {
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
            if !win_priority && !single_forcing && candidates.len() > 1 && tt_best_move.is_some() {
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
        if !lazy_tie_ordering && !use_nnue_policy && !win_priority && !single_forcing {
            ordered.truncate(wide);
        }
        let search_limit = if !win_priority && !single_forcing {
            ordered.len().min(wide)
        } else {
            ordered.len()
        };
        if search_limit == 0 {
            return (-INF - 1, None);
        }

        let mut current = i32::MIN;
        let mut has_score = false;
        let mut best_move = None;
        let original_alpha = alpha;
        let mut hash_flag = HASHF_ALPHA;
        let mut found_pv = false;
        let child_wide = (((wide * self.config.root_search.ratio_num as usize)
            / self.config.root_search.ratio_den as usize)
            + 1)
        .min(wide);
        let case_count = search_limit;
        let (pre_corner, pre_half) = if options.root {
            compute_corner_state(board)
        } else {
            (false, 0)
        };
        let mut running_downf = options.downf;
        let raw_depthdown = 0.0_f64.max(
            1.0 - self.config.search.extend_ratio
                + self.config.search.extend_ratio * (case_count.max(1) as f64).ln()
                    / (wide.max(2) as f64).ln(),
        );
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
            && depth <= 4.0
        {
            if depth <= 1.0 {
                6
            } else if depth <= 2.0 {
                10
            } else if depth <= 3.0 {
                15
            } else {
                22
            }
        } else {
            usize::MAX
        };
        let use_abdada = abdada.is_some() && depth >= ABDADA_MIN_DEPTH && search_limit > 1;

        #[derive(Clone, Copy)]
        struct DeferredCandidate {
            index: usize,
            candidate: Candidate,
            local_downf: i32,
            depthdown: f64,
            net: i32,
        }
        let mut deferred = [DeferredCandidate {
            index: 0,
            candidate: ordered[0],
            local_downf: 0,
            depthdown: 0.0,
            net: 0,
        }; 64];
        let mut deferred_len = 0_usize;
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
                let (index, candidate, local_downf, depthdown, mut net) = if pass == 0 {
                    let index = step;
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
                    if lazy_tie_ordering && index >= tie_sorted_until {
                        tie_sorted_until = resolve_candidate_tie_group(
                            board,
                            &mut ordered,
                            index,
                            side,
                            tt_best_move,
                        );
                    }
                    let candidate = ordered[index];
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
                    running_downf += index as i32;
                    let mut local_downf = running_downf;
                    let mut depthdown = base_depthdown;
                    let mut net = 0_i32;
                    if local_downf >= 15 {
                        net = local_downf / 15;
                        depthdown += f64::from(net);
                        local_downf %= 15;
                    }
                    running_downf = local_downf;
                    // Shift-invariant Policy-guided LMR for low-probability
                    // quiet tail moves (re-searched at full depth on fail-high).
                    if use_nnue_policy
                        && index >= 2
                        && !hostile_threat
                        && candidate.self_attack < 4
                        && candidate.opp_attack < 4
                        && policy_top_logit.is_finite()
                    {
                        let gap = (policy_top_logit - candidate.order_score).max(0.0);
                        if gap > 1.5 {
                            let scale = if is_pv { 0.18 } else { 0.35 };
                            let extra = (scale * (gap - 1.5)).min(2.5);
                            let extra_plies = extra.floor() as i32;
                            if extra_plies > 0 {
                                depthdown += f64::from(extra_plies);
                                net += extra_plies;
                            }
                        }
                    }
                    (index, candidate, local_downf, depthdown, net)
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
                        item.net,
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
                                net,
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
                } else {
                    tt_view.prefetch(board.zobrist_key());
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
                    let child_options = SearchOptions {
                        opo: 1 - options.opo,
                        ply: options.ply + 1,
                        root: false,
                        root_allowed_moves: None,
                        root_preferred_move: None,
                        downf: local_downf,
                        root_depth: Some(root_depth),
                        priority_base: Some(priority_base),
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
                            child_options,
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
                                child_options,
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
                            child_options,
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
                    if score > alpha && score > current && net > 0 {
                        attempt_depth += f64::from(net);
                        net = 0;
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
                    hash_flag = HASHF_BETA;
                    stats.cutoffs += 1;
                    early_exit = true;
                    break;
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

        if !root_restricted {
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
            } else {
                tt_view.store(TTEntry {
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
}
