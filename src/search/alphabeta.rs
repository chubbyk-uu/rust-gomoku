//! Alpha-beta search implementation for the classic mainline.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use crate::board::{move_to_xy, Board};
use crate::config::EngineConfig;
use crate::constants::{HASHF_ALPHA, HASHF_BETA, HASHF_EXACT, INF, WIN};
use crate::eval::{evaluate_board, value_wide_compute, EvalCaches};
use crate::search::movegen::{generate_candidates_with_coverage, CoverageTracker};
use crate::search::{order_candidates, order_candidates_root_classic, TTEntry, TranspositionTable};
use crate::threats::VCFSearcher;
use crate::types::{Move, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchStats {
    pub nodes: usize,
    pub leaf_nodes: usize,
    pub tt_hits: usize,
    pub cutoffs: usize,
    pub stop: bool,
    pub node_limit: Option<usize>,
    pub deadline: Option<Instant>,
    pub time_check_mask: usize,
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SearchOptions<'a> {
    pub opo: usize,
    pub ply: i32,
    pub root: bool,
    pub root_allowed_moves: Option<&'a HashSet<Move>>,
    pub downf: i32,
    pub root_depth: Option<f64>,
    pub priority_base: Option<i32>,
}

pub fn terminal_score(board: &Board, side: Side, ply: i32) -> Option<i32> {
    if board.winner() == 0 {
        return None;
    }
    if board.winner() == side {
        return Some(INF - ply);
    }
    Some(-INF + ply)
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
    pub stop_signal: Option<Arc<AtomicBool>>,
}

impl AlphaBetaSearcher {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            tt: TranspositionTable::default(),
            vcf: VCFSearcher::default(),
            stop_signal: None,
        }
    }

    pub fn with_tt(config: EngineConfig, tt: TranspositionTable) -> Self {
        Self {
            config,
            tt,
            vcf: VCFSearcher::default(),
            stop_signal: None,
        }
    }

    pub fn with_stop_signal(mut self, stop_signal: Arc<AtomicBool>) -> Self {
        self.stop_signal = Some(stop_signal);
        self
    }

    pub fn nonroot_vcf_depth(depth: f64, root_depth: f64) -> i32 {
        (depth + 6.0 - 2.0 * root_depth) as i32
    }

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
        )
    }

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
    ) -> (i32, Option<Move>) {
        let root_depth = options.root_depth.unwrap_or(depth);
        let priority_base = options.priority_base.unwrap_or(board.move_count() as i32);
        let hash_depth = depth as i32;
        let original_beta = beta;

        let next_node = stats.nodes + 1;
        if self
            .stop_signal
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Relaxed))
        {
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
        stats.nodes = next_node;

        if let Some(score) = terminal_score(board, side, options.ply) {
            return (score, None);
        }

        let probe = self.tt.probe(board.zobrist_key(), hash_depth, alpha, beta);
        if probe.hit && probe.value.is_some() {
            stats.tt_hits += 1;
            return (probe.value.expect("checked above"), probe.best_move);
        }
        if probe.has_window && !options.root {
            alpha = alpha.max(probe.window_alpha);
            beta = beta.min(probe.window_beta);
        }

        if depth <= 0.0 {
            stats.leaf_nodes += 1;
            let mut score =
                (-evaluate_board(board, caches, -side, options.opo, &self.config)) as i32;
            if score >= WIN {
                score = INF - options.ply;
            } else if score <= -WIN {
                score = -INF + options.ply;
            }
            self.tt.store(TTEntry {
                key: board.zobrist_key(),
                value: score,
                flag: HASHF_EXACT,
                depth: 0,
                priority: priority_base * 10,
                best_move: None,
            });
            return (score, None);
        }

        let generated = generate_candidates_with_coverage(
            board,
            caches,
            side,
            &self.config,
            Some(wide),
            if options.root {
                options.root_allowed_moves
            } else {
                None
            },
            probe.best_move,
            options.root,
            coverage,
        );
        let mut ordered = if options.root {
            order_candidates_root_classic(board, &generated.candidates, side)
        } else {
            order_candidates(board, &generated.candidates, side, probe.best_move)
        };
        if generated.win_priority && !ordered.is_empty() {
            return (INF, Some(ordered[0].move_));
        }

        if self.config.runtime.compute_vcf && self.config.runtime.nonroot_vcf && !options.root {
            let nonroot_vcf_depth = Self::nonroot_vcf_depth(depth, root_depth);
            if nonroot_vcf_depth > 0 && self.vcf.search(board, -side, nonroot_vcf_depth).found {
                let mut filtered = Vec::new();
                for candidate in ordered {
                    let mut trial = board.clone();
                    trial
                        .play(candidate.move_, Some(side))
                        .expect("ordered move stays legal on trial board");
                    if !self.vcf.search(&trial, -side, nonroot_vcf_depth).found {
                        filtered.push(candidate);
                    }
                }
                ordered = filtered;
            }
        }

        if !generated.win_priority && !generated.single_forcing {
            ordered.truncate(wide);
        }
        if ordered.is_empty() {
            return (-INF - 1, None);
        }

        let mut current = -INF - 1;
        let mut best_move = None;
        let original_alpha = alpha;
        let mut hash_flag = HASHF_ALPHA;
        let mut found_pv = false;
        let child_wide = (((wide * self.config.root_search.ratio_num as usize)
            / self.config.root_search.ratio_den as usize)
            + 1)
        .min(wide);
        let case_count = ordered.len();
        let (pre_corner, pre_half) = if options.root {
            compute_corner_state(board)
        } else {
            (false, 0)
        };
        let mut running_downf = options.downf;

        for (index, candidate) in ordered.iter().copied().enumerate() {
            let snapshot = caches.snapshot();
            let (mx, my) = move_to_xy(candidate.move_).expect("candidate move is valid");
            board
                .play(candidate.move_, Some(side))
                .expect("ordered candidate stays legal");
            coverage.add_move(candidate.move_);
            value_wide_compute(board, caches, (mx, my));

            running_downf += index as i32;
            let mut local_downf = running_downf;
            let mut depthdown = 0.0_f64.max(
                1.0 - self.config.search.extend_ratio
                    + self.config.search.extend_ratio * (case_count.max(1) as f64).ln()
                        / (wide.max(2) as f64).ln(),
            );
            let mut net = 0_i32;
            if local_downf >= 15 {
                net = local_downf / 15;
                depthdown += f64::from(net);
                local_downf %= 15;
            }
            running_downf = local_downf;

            let mut atdown = 0_i32;
            if candidate.self_attack == 4 {
                atdown = self.config.search.atdown4 as i32;
            } else if candidate.self_attack == 3 {
                atdown = self.config.search.atdown3 as i32;
            }
            if options.root {
                let (x, y) = move_to_xy(candidate.move_).expect("ordered candidate stays in range");
                let h = x.min(y).min(board.size() - 1 - x).min(board.size() - 1 - y);
                let is_corner = pre_corner || h <= 1 || (h == 2 && pre_half >= 1);
                atdown += rootbonus(board, x, y, is_corner);
            }

            let mut attempt_depth = depth - depthdown;
            let score = loop {
                let child_options = SearchOptions {
                    opo: 1 - options.opo,
                    ply: options.ply + 1,
                    root: false,
                    root_allowed_moves: None,
                    downf: local_downf,
                    root_depth: Some(root_depth),
                    priority_base: Some(priority_base),
                };
                let score = if found_pv {
                    let (narrow_score, _) = self.search_with_coverage(
                        board,
                        caches,
                        -side,
                        attempt_depth,
                        -(alpha + atdown) - 1,
                        -(alpha + atdown),
                        child_wide,
                        stats,
                        child_options,
                        coverage,
                    );
                    if stats.stop {
                        break narrow_score;
                    }
                    let narrowed = -atdown - narrow_score;
                    if alpha < narrowed && narrowed < beta {
                        let (full_score, _) = self.search_with_coverage(
                            board,
                            caches,
                            -side,
                            attempt_depth,
                            -(beta + atdown),
                            -(alpha + atdown),
                            child_wide,
                            stats,
                            child_options,
                            coverage,
                        );
                        if stats.stop {
                            break full_score;
                        }
                        -atdown - full_score
                    } else {
                        narrowed
                    }
                } else {
                    let (full_score, _) = self.search_with_coverage(
                        board,
                        caches,
                        -side,
                        attempt_depth,
                        -(beta + atdown),
                        -(alpha + atdown),
                        child_wide,
                        stats,
                        child_options,
                        coverage,
                    );
                    if stats.stop {
                        break full_score;
                    }
                    -atdown - full_score
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

            board.undo().expect("ordered candidate was just played");
            coverage.remove_move(candidate.move_);
            caches.restore_snapshot(&snapshot);
            if stats.stop {
                break;
            }

            if score > current {
                current = score;
            }
            if score > alpha {
                alpha = score;
                best_move = Some(candidate.move_);
                hash_flag = HASHF_EXACT;
                found_pv = true;
            }
            if options.root && score >= WIN {
                break;
            }
            if alpha >= beta {
                hash_flag = HASHF_BETA;
                stats.cutoffs += 1;
                break;
            }
        }

        if current <= original_alpha && hash_flag != HASHF_BETA {
            hash_flag = HASHF_ALPHA;
        }
        if stats.stop {
            return (current, best_move);
        }

        let mut store_depth = hash_depth;
        if (current >= WIN && current > original_alpha)
            || (current <= -WIN && current < original_beta)
        {
            hash_flag = HASHF_EXACT;
            store_depth += 10;
        }
        self.tt.store(TTEntry {
            key: board.zobrist_key(),
            value: current,
            flag: hash_flag,
            depth: store_depth,
            priority: priority_base * 10 + hash_depth,
            best_move,
        });
        (current, best_move)
    }
}
