//! Root iterative-deepening search for the classic mainline.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::EngineConfig;
use crate::constants::{INF, WIN};
use crate::eval::{recompute_all, EvalCaches};
use crate::rules::RuleSet;
use crate::search::{
    AlphaBetaSearcher, RootCandidateProfile, SearchOptions, SearchStats, TranspositionTable,
};
use crate::threats::{forcing_threat_moves, has_vct_trigger, VCFSearcher, VCTSearcher, VCTStats};
use crate::types::{Move, Side};

const CLASSIC_RAND_SEED: i32 = 1_232_356;
const CLASSIC_FALLBACK_STATE: [u32; 31] = [
    3344391599, 3760159923, 229790648, 3328593876, 529145457, 4021946065, 1735816513, 469166854,
    1730624144, 2908500504, 649120694, 3012569930, 473519764, 1775465023, 936985512, 994684877,
    4231614135, 825016603, 3651181685, 2927649197, 4259523512, 1063826198, 2094918629, 2306226027,
    4013509952, 563982589, 367722354, 742065300, 1591101748, 477268195, 3720283884,
];
const CLASSIC_FALLBACK_FPTR: usize = 25;
const CLASSIC_FALLBACK_RPTR: usize = 22;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SearchLimits {
    pub max_depth: i32,
    pub root_width: usize,
    pub node_limit: Option<usize>,
    pub time_limit_ms: Option<f64>,
}

impl SearchLimits {
    pub fn fixed_from_config(config: &EngineConfig) -> Self {
        Self {
            max_depth: config.root_search.depth,
            root_width: config.root_search.wide as usize,
            node_limit: None,
            time_limit_ms: None,
        }
    }

    pub fn timed_from_config(config: &EngineConfig) -> Self {
        Self {
            max_depth: config.root_search.timed_max_depth,
            root_width: config.root_search.timed_max_wide as usize,
            node_limit: None,
            time_limit_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub move_: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootDepthProfile {
    pub depth: i32,
    pub score: i32,
    pub best_move: Option<Move>,
    pub nodes: usize,
    pub elapsed_us: u128,
    pub stopped: bool,
    pub candidates: Vec<RootCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RootTrace {
    pub used_vcf: bool,
    pub vcf_found: bool,
    pub used_vct: bool,
    pub vct_triggered: bool,
    pub vct_found: bool,
    pub vct_move: Option<Move>,
    pub vct_accepted: bool,
    pub vct_reject_reason: Option<&'static str>,
    pub vct_ms: Option<f64>,
    pub vct_stats: Option<VCTStats>,
    pub alphabeta_ms: Option<f64>,
    pub overlap_used: bool,
    pub overlap_ab_ms: Option<f64>,
    pub overlap_ab_cancelled: bool,
    pub overlap_wait_ms: Option<f64>,
    pub tt_snapshot_ms: Option<f64>,
    pub tactical_path: &'static str,
    pub root_profiles: Vec<RootDepthProfile>,
    pub fast_history_ordering: bool,
    pub killer_hits: usize,
    pub history_hits: usize,
    pub killer_updates: usize,
    pub history_updates: usize,
    pub tt_bestmove_current_generation: usize,
    pub tt_bestmove_old_generation: usize,
}

impl Default for RootTrace {
    fn default() -> Self {
        Self {
            used_vcf: false,
            vcf_found: false,
            used_vct: false,
            vct_triggered: false,
            vct_found: false,
            vct_move: None,
            vct_accepted: false,
            vct_reject_reason: None,
            vct_ms: None,
            vct_stats: None,
            alphabeta_ms: None,
            overlap_used: false,
            overlap_ab_ms: None,
            overlap_ab_cancelled: false,
            overlap_wait_ms: None,
            tt_snapshot_ms: None,
            tactical_path: "alphabeta",
            root_profiles: Vec::new(),
            fast_history_ordering: false,
            killer_hits: 0,
            history_hits: 0,
            killer_updates: 0,
            history_updates: 0,
            tt_bestmove_current_generation: 0,
            tt_bestmove_old_generation: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClassicFallbackRng {
    state: [u32; 31],
    fptr: usize,
    rptr: usize,
}

impl ClassicFallbackRng {
    pub fn new(seed: i32) -> Result<Self, &'static str> {
        if seed != CLASSIC_RAND_SEED {
            return Err("only the classic fallback seed is supported");
        }
        Ok(Self {
            state: CLASSIC_FALLBACK_STATE,
            fptr: CLASSIC_FALLBACK_FPTR,
            rptr: CLASSIC_FALLBACK_RPTR,
        })
    }

    pub fn randrange(&mut self, upper: usize) -> Result<usize, &'static str> {
        if upper == 0 {
            return Err("upper must be positive");
        }
        let value = self.state[self.fptr].wrapping_add(self.state[self.rptr]);
        self.state[self.fptr] = value;
        self.fptr = (self.fptr + 1) % self.state.len();
        self.rptr = (self.rptr + 1) % self.state.len();
        Ok((((value >> 1) & 0x7FFF_FFFF) as usize) % upper)
    }
}

pub fn new_classic_fallback_rng() -> ClassicFallbackRng {
    ClassicFallbackRng::new(CLASSIC_RAND_SEED).expect("classic fallback seed stays supported")
}

fn elapsed_ms(start: Instant) -> f64 {
    (start.elapsed().as_secs_f64() * 1000.0 * 1000.0).round() / 1000.0
}

fn shape_label(shape: i32) -> i32 {
    (shape >> 16) & 0xF
}

fn shape_aux(shape: i32) -> i32 {
    shape & 0xF
}

pub fn fallback_ai_move(
    board: &Board,
    caches: &EvalCaches,
    side: Side,
    rule: RuleSet,
    rng: &mut ClassicFallbackRng,
) -> Result<Move, &'static str> {
    let player = if side == 1 { 0 } else { 1 };
    let opponent = 1 - player;
    let mut best_value = i64::MIN;
    let mut best_moves = Vec::new();

    for move_index in 0..(board.size() * board.size()) {
        let move_ = move_index as Move;
        let legal = if rule == RuleSet::Freestyle {
            board.is_legal_move(move_)
        } else {
            board.is_legal_move_for_rule(move_, side, rule)
        };
        if !legal {
            continue;
        }
        let (x, y) = move_to_xy(move_).expect("iterated move index stays in range");

        let mut offensive = 0_i64;
        let (mut a1l, mut b2l, mut a2l, mut b3l, mut a4l, mut a3l, mut b4l, mut a5l, mut a6l) = (
            0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
        );
        for direction in 0..4 {
            let shape = caches.shape_cache[player][x][y][direction];
            let label = shape_label(shape);
            if label == 2 {
                a1l += 1;
            } else if label == 3 {
                b2l += 1;
            } else if label == 9 || label == 8 {
                a3l += 1;
            } else if label == 10 {
                b4l += i64::from(shape_aux(shape));
            } else if label == 12 {
                a5l += 1;
            } else if label == 7 {
                b3l += 1;
            } else if (4..=6).contains(&label) {
                a2l += 1;
            } else if label == 11 {
                a4l += 1;
                b4l += 1;
            } else if label == 13 {
                a6l += 1;
            }
        }
        offensive += a1l;
        offensive += b2l;
        offensive += a2l * 5;
        offensive += b3l * 10;
        offensive += a3l * 12;
        offensive += b4l * 16;
        offensive += i64::from(a3l >= 2) * 100;
        offensive += i64::from(b4l > 0 && a3l > 0) * 3000;
        offensive += i64::from(b4l >= 2) * 4000;
        offensive += a4l * 6000;
        offensive += a5l * 1_000_000;
        let _ = a6l;

        let mut defensive = 0_i64;
        let (mut a2l, mut b3l, mut a3l, mut b4l, mut a4l, mut a5l, mut a6l) =
            (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
        for direction in 0..4 {
            let shape = caches.shape_cache[opponent][x][y][direction];
            let label = shape_label(shape);
            if label == 9 || label == 8 {
                a3l += 1;
            } else if label == 10 {
                b4l += i64::from(shape_aux(shape));
            } else if label == 12 {
                a5l += 1;
            } else if label == 7 {
                b3l += 1;
            } else if (4..=6).contains(&label) {
                a2l += 1;
            } else if label == 11 {
                a4l += 1;
                b4l += 1;
            } else if label == 13 {
                a6l += 1;
            }
        }
        defensive += a2l;
        defensive += b3l;
        defensive += a3l * 6;
        defensive += b4l * 11;
        defensive += i64::from(a3l >= 2) * 15;
        defensive += i64::from(b4l > 0 && a3l > 0) * 1500;
        defensive += i64::from(b4l >= 2) * 2000;
        defensive += a4l * 3000;
        defensive += a5l * 50_000;
        let _ = a6l;

        let total = 5 * offensive + 5 * defensive;
        if total > best_value {
            best_value = total;
            best_moves.clear();
            best_moves.push(move_);
        } else if total == best_value {
            best_moves.push(move_);
        }
    }

    if best_moves.is_empty() {
        return Err("fallback AIs found no legal move on non-terminal board");
    }
    Ok(best_moves[rng.randrange(best_moves.len())?])
}

#[derive(Clone, Debug)]
pub struct RootSearcher {
    pub config: EngineConfig,
    pub tt: TranspositionTable,
    pub alphabeta: AlphaBetaSearcher,
    pub vcf: VCFSearcher,
    pub vct: VCTSearcher,
    pub fallback_rng: ClassicFallbackRng,
    pub last_trace: Option<RootTrace>,
}

struct AlphaBetaThreadResult {
    result: SearchResult,
    trace: RootTrace,
    searcher: RootSearcher,
    elapsed_ms: f64,
    cancelled: bool,
}

impl RootSearcher {
    fn fallback_search_result(
        &mut self,
        board: &Board,
        caches: &EvalCaches,
        side: Side,
        score: i32,
        depth: i32,
        nodes: usize,
    ) -> SearchResult {
        let move_ = fallback_ai_move(
            board,
            caches,
            side,
            self.config.rule_set,
            &mut self.fallback_rng,
        )
        .unwrap_or(0);
        let score = if board.is_legal_move_for_rule(move_, side, self.config.rule_set) {
            score
        } else {
            -INF
        };
        SearchResult {
            move_,
            score,
            depth,
            nodes,
        }
    }

    pub fn new(config: EngineConfig) -> Self {
        let tt = TranspositionTable::default();
        let alphabeta = AlphaBetaSearcher::with_tt(config.clone(), tt.clone());
        Self {
            config,
            tt,
            alphabeta,
            vcf: VCFSearcher::default(),
            vct: VCTSearcher::default(),
            fallback_rng: new_classic_fallback_rng(),
            last_trace: None,
        }
    }

    pub fn with_tt(config: EngineConfig, tt: TranspositionTable) -> Self {
        let alphabeta = AlphaBetaSearcher::with_tt(config.clone(), tt.clone());
        Self {
            config,
            tt,
            alphabeta,
            vcf: VCFSearcher::default(),
            vct: VCTSearcher::default(),
            fallback_rng: new_classic_fallback_rng(),
            last_trace: None,
        }
    }

    pub fn verify_root_vct_move(
        &mut self,
        board: &Board,
        side: Side,
        move_: Move,
    ) -> (bool, Option<&'static str>) {
        if !board.is_legal_move_for_rule(move_, side, self.config.rule_set) {
            return (false, Some("illegal"));
        }
        let mut trial = board.clone();
        if trial.force_side_to_move(side).is_err()
            || trial
                .play_assuming_rule_legal(move_, Some(side), self.config.rule_set)
                .is_err()
        {
            return (false, Some("illegal"));
        }
        if trial.winner() == side {
            return (true, None);
        }
        if !forcing_threat_moves(&trial, -side).is_empty() {
            return (false, Some("opponent_forcing"));
        }
        if self.config.runtime.compute_vcf
            && self
                .vcf
                .search_with_multi_reply(
                    &trial,
                    -side,
                    self.config.runtime.vct_verify_opponent_vcf_depth,
                    self.config.runtime.vcf_multi_reply,
                )
                .found
        {
            return (false, Some("opponent_vcf"));
        }
        (true, None)
    }

    pub fn root_allowed_moves(&self, board: &Board) -> Option<HashSet<Move>> {
        if self.config.runtime.static_board || board.move_count() == 0 {
            return None;
        }
        let moves = board.occupied_moves();
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for move_ in moves {
            let (x, y) = move_to_xy(move_).expect("occupied move stays in range");
            xs.push(x as i32);
            ys.push(y as i32);
        }
        let margin = self.config.runtime.dynamic_board_margin;
        let mut xmin = 0.max(xs.iter().copied().min()? - margin);
        let mut xmax = (board.size() as i32 - 1).min(xs.iter().copied().max()? + margin);
        let mut ymin = 0.max(ys.iter().copied().min()? - margin);
        let mut ymax = (board.size() as i32 - 1).min(ys.iter().copied().max()? + margin);

        let mut toggle = 0;
        while (xmax - xmin) != (ymax - ymin) {
            toggle += 1;
            if (xmax - xmin) > (ymax - ymin) {
                if toggle % 2 == 1 {
                    if ymin > 0 {
                        ymin -= 1;
                    } else {
                        ymax = (board.size() as i32 - 1).min(ymax + 1);
                    }
                } else if ymax < board.size() as i32 - 1 {
                    ymax += 1;
                } else {
                    ymin = 0.max(ymin - 1);
                }
            } else if toggle % 2 == 1 {
                if xmin > 0 {
                    xmin -= 1;
                } else {
                    xmax = (board.size() as i32 - 1).min(xmax + 1);
                }
            } else if xmax < board.size() as i32 - 1 {
                xmax += 1;
            } else {
                xmin = 0.max(xmin - 1);
            }
        }

        let mut allowed = HashSet::new();
        for y in ymin..=ymax {
            for x in xmin..=xmax {
                if board
                    .at(x as usize, y as usize)
                    .expect("window stays in bounds")
                    == 0
                {
                    allowed.insert(xy_to_move(x as usize, y as usize).expect("window stays valid"));
                }
            }
        }
        Some(allowed)
    }

    pub fn apply_opponent_vcf_filter(
        &mut self,
        board: &Board,
        side: Side,
        allowed_moves: Option<HashSet<Move>>,
    ) -> Option<HashSet<Move>> {
        if self.config.rule_set != RuleSet::Freestyle {
            return allowed_moves;
        }
        if !self.config.runtime.compute_vcf {
            return allowed_moves;
        }
        let opponent_vcf = self.vcf.search_with_multi_reply(
            board,
            -side,
            self.config.runtime.opponent_vcf_depth,
            self.config.runtime.vcf_multi_reply,
        );
        if !opponent_vcf.found {
            return allowed_moves;
        }

        let candidates: Vec<Move> = if let Some(allowed) = allowed_moves {
            let mut moves: Vec<_> = allowed
                .into_iter()
                .filter(|&move_| board.is_legal_move_for_rule(move_, side, self.config.rule_set))
                .collect();
            moves.sort_unstable();
            moves
        } else {
            (0..(board.size() * board.size()))
                .map(|index| index as Move)
                .filter(|&move_| board.is_legal_move_for_rule(move_, side, self.config.rule_set))
                .collect()
        };

        let mut filtered = HashSet::new();
        for move_ in candidates {
            let mut trial = board.clone();
            trial
                .play_assuming_rule_legal(move_, Some(side), self.config.rule_set)
                .expect("candidate move stays legal on trial board");
            if !self
                .vcf
                .search_with_multi_reply(
                    &trial,
                    -side,
                    self.config.runtime.opponent_vcf_depth,
                    self.config.runtime.vcf_multi_reply,
                )
                .found
            {
                filtered.insert(move_);
            }
        }
        Some(filtered)
    }

    fn can_overlap_vct_alphabeta(&self, limits: &SearchLimits) -> bool {
        self.config.runtime.overlap_vct_alphabeta
            && limits.time_limit_ms.is_none()
            && limits.node_limit.is_none()
    }

    fn search_alphabeta_only(
        &mut self,
        board: &mut Board,
        side: Side,
        limits: SearchLimits,
        trace: &mut RootTrace,
        cancel: Option<Arc<AtomicBool>>,
    ) -> SearchResult {
        let mut caches = EvalCaches::new();
        recompute_all(board, &mut caches);

        self.alphabeta.config = self.config.clone();
        self.alphabeta.advance_tt_generation();
        let mut best_move = None;
        let mut best_score = -INF;
        let mut total_nodes = 0_usize;
        let deadline = limits
            .time_limit_ms
            .map(|ms| Instant::now() + Duration::from_secs_f64(ms / 1000.0));
        if cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return self.fallback_search_result(board, &caches, side, best_score, 0, 0);
        }
        let root_allowed_moves =
            self.apply_opponent_vcf_filter(board, side, self.root_allowed_moves(board));
        if cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return self.fallback_search_result(board, &caches, side, best_score, 0, 0);
        }
        if let Some(allowed) = &root_allowed_moves {
            let mut root_legal_moves: Vec<_> = allowed
                .iter()
                .copied()
                .filter(|&move_| board.is_legal_move_for_rule(move_, side, self.config.rule_set))
                .collect();
            root_legal_moves.sort_unstable();
            if root_legal_moves.is_empty() {
                return self.fallback_search_result(board, &caches, side, -INF, 0, 0);
            }
            if root_legal_moves.len() == 1 {
                return SearchResult {
                    move_: root_legal_moves[0],
                    score: 0,
                    depth: 0,
                    nodes: 0,
                };
            }
        }

        let mut completed_depth = 0_i32;
        for depth in 1..=limits.max_depth {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            if cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
            {
                break;
            }
            let depth_start = Instant::now();
            let mut stats = SearchStats {
                node_limit: limits.node_limit,
                deadline,
                root_profile: self.config.runtime.root_profile,
                cancel: cancel.clone(),
                ..SearchStats::default()
            };
            let (score, move_) = self.alphabeta.search(
                board,
                &mut caches,
                side,
                f64::from(depth),
                -INF,
                INF,
                limits.root_width,
                &mut stats,
                SearchOptions {
                    opo: 1,
                    root: true,
                    root_allowed_moves: root_allowed_moves.as_ref(),
                    ..SearchOptions::default()
                },
            );
            let depth_elapsed_us = depth_start.elapsed().as_micros();
            total_nodes += stats.nodes;
            trace.fast_history_ordering |= stats.fast_history_ordering;
            trace.killer_hits += stats.killer_hits;
            trace.history_hits += stats.history_hits;
            trace.killer_updates += stats.killer_updates;
            trace.history_updates += stats.history_updates;
            trace.tt_bestmove_current_generation += stats.tt_bestmove_current_generation;
            trace.tt_bestmove_old_generation += stats.tt_bestmove_old_generation;
            if self.config.runtime.root_profile {
                trace.root_profiles.push(RootDepthProfile {
                    depth,
                    score,
                    best_move: move_,
                    nodes: stats.nodes,
                    elapsed_us: depth_elapsed_us,
                    stopped: stats.stop,
                    candidates: std::mem::take(&mut stats.root_candidates),
                });
                self.last_trace = Some(trace.clone());
            }
            if stats.stop {
                break;
            }
            completed_depth = depth;
            if let Some(move_) = move_ {
                best_move = Some(move_);
                best_score = score;
            } else if score <= -WIN {
                if let Ok(move_) = fallback_ai_move(
                    board,
                    &caches,
                    side,
                    self.config.rule_set,
                    &mut self.fallback_rng,
                ) {
                    best_move = Some(move_);
                }
                best_score = score;
            }
            if score >= WIN || score <= -WIN {
                break;
            }
        }

        if let Some(move_) = best_move {
            SearchResult {
                move_,
                score: best_score,
                depth: completed_depth,
                nodes: total_nodes,
            }
        } else {
            self.fallback_search_result(
                board,
                &caches,
                side,
                best_score,
                completed_depth,
                total_nodes,
            )
        }
    }

    fn search_vct_and_alphabeta_overlap(
        &mut self,
        board: &Board,
        side: Side,
        limits: SearchLimits,
        mut trace: RootTrace,
    ) -> SearchResult {
        trace.overlap_used = true;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel_flag = cancel.clone();
        let worker_cancel = Some(worker_cancel_flag.clone());
        let mut worker_board = board.clone();
        let worker_config = self.config.clone();
        let tt_snapshot_start = Instant::now();
        let worker_tt = self.tt.fork_snapshot();
        trace.tt_snapshot_ms = Some(elapsed_ms(tt_snapshot_start));
        let worker_fallback_rng = self.fallback_rng.clone();

        let ab_handle = thread::spawn(move || {
            let mut worker = RootSearcher::with_tt(worker_config, worker_tt);
            worker.fallback_rng = worker_fallback_rng;
            let mut worker_trace = RootTrace::default();
            let ab_start = Instant::now();
            let result = worker.search_alphabeta_only(
                &mut worker_board,
                side,
                limits,
                &mut worker_trace,
                worker_cancel,
            );
            let elapsed_ms = elapsed_ms(ab_start);
            let cancelled = worker_cancel_flag.load(Ordering::Relaxed);
            AlphaBetaThreadResult {
                result,
                trace: worker_trace,
                searcher: worker,
                elapsed_ms,
                cancelled,
            }
        });

        let vct_start = Instant::now();
        self.vct.memo_diagnostics_enabled = self.config.runtime.root_profile;
        self.vct.strict_and_memo_key = self.config.runtime.vct_strict_and_memo_key;
        let vct_result = self
            .vct
            .search(board, side, self.config.runtime.root_vct_depth);
        trace.vct_ms = Some(elapsed_ms(vct_start));
        trace.vct_stats = Some(self.vct.stats.clone());
        trace.vct_found = vct_result.found;
        trace.vct_move = vct_result.move_;
        if let Some(move_) = vct_result.move_.filter(|_| vct_result.found) {
            let (accepted, reason) = self.verify_root_vct_move(board, side, move_);
            trace.vct_accepted = accepted;
            trace.vct_reject_reason = reason;
            if accepted {
                cancel.store(true, Ordering::Relaxed);
                let wait_start = Instant::now();
                let ab_result = ab_handle
                    .join()
                    .expect("overlapped alphabeta worker stays healthy");
                trace.overlap_wait_ms = Some(elapsed_ms(wait_start));
                trace.overlap_ab_ms = Some(ab_result.elapsed_ms);
                trace.overlap_ab_cancelled = true;
                trace.tactical_path = "vct";
                self.last_trace = Some(trace);
                return SearchResult {
                    move_,
                    score: INF,
                    depth: 0,
                    nodes: 0,
                };
            }
        }

        let wait_start = Instant::now();
        let ab_result = ab_handle
            .join()
            .expect("overlapped alphabeta worker stays healthy");
        trace.overlap_wait_ms = Some(elapsed_ms(wait_start));
        trace.overlap_ab_ms = Some(ab_result.elapsed_ms);
        trace.overlap_ab_cancelled = ab_result.cancelled;
        self.tt = ab_result.searcher.tt;
        self.alphabeta = ab_result.searcher.alphabeta;
        self.fallback_rng = ab_result.searcher.fallback_rng;
        trace.alphabeta_ms = Some(ab_result.elapsed_ms);
        trace.root_profiles = ab_result.trace.root_profiles;
        trace.fast_history_ordering = ab_result.trace.fast_history_ordering;
        trace.killer_hits = ab_result.trace.killer_hits;
        trace.history_hits = ab_result.trace.history_hits;
        trace.killer_updates = ab_result.trace.killer_updates;
        trace.history_updates = ab_result.trace.history_updates;
        trace.tt_bestmove_current_generation = ab_result.trace.tt_bestmove_current_generation;
        trace.tt_bestmove_old_generation = ab_result.trace.tt_bestmove_old_generation;
        self.last_trace = Some(trace);
        ab_result.result
    }

    pub fn search(&mut self, board: &mut Board, limits: Option<SearchLimits>) -> SearchResult {
        let limits = limits.unwrap_or_else(|| SearchLimits::fixed_from_config(&self.config));

        if board.move_count() == 0 {
            self.last_trace = Some(RootTrace::default());
            let center =
                xy_to_move(board.size() / 2, board.size() / 2).expect("center stays valid");
            return SearchResult {
                move_: center,
                score: 0,
                depth: 0,
                nodes: 0,
            };
        }

        let side = board.side_to_move();
        let mut trace = RootTrace::default();
        self.last_trace = Some(trace.clone());
        let tactical_enabled = self.config.rule_set == RuleSet::Freestyle;

        if tactical_enabled && self.config.runtime.compute_vcf {
            trace.used_vcf = true;
            let vcf_result = self.vcf.search_with_multi_reply(
                board,
                side,
                self.config.runtime.root_vcf_depth,
                self.config.runtime.vcf_multi_reply,
            );
            if vcf_result.found {
                trace.vcf_found = true;
                trace.tactical_path = "vcf";
                self.last_trace = Some(trace);
                return SearchResult {
                    move_: vcf_result
                        .move_
                        .expect("successful VCF search returns a move"),
                    score: INF,
                    depth: 0,
                    nodes: 0,
                };
            }
        }

        if tactical_enabled
            && self.config.runtime.compute_vct
            && self.config.runtime.root_vct_depth > 0
        {
            trace.used_vct = true;
            if has_vct_trigger(board, side) {
                trace.vct_triggered = true;
                if self.can_overlap_vct_alphabeta(&limits) {
                    return self.search_vct_and_alphabeta_overlap(board, side, limits, trace);
                }

                let vct_start = Instant::now();
                self.vct.memo_diagnostics_enabled = self.config.runtime.root_profile;
                self.vct.strict_and_memo_key = self.config.runtime.vct_strict_and_memo_key;
                let vct_result = self
                    .vct
                    .search(board, side, self.config.runtime.root_vct_depth);
                trace.vct_ms = Some(elapsed_ms(vct_start));
                trace.vct_stats = Some(self.vct.stats.clone());
                trace.vct_found = vct_result.found;
                trace.vct_move = vct_result.move_;
                if let Some(move_) = vct_result.move_.filter(|_| vct_result.found) {
                    let (accepted, reason) = self.verify_root_vct_move(board, side, move_);
                    trace.vct_accepted = accepted;
                    trace.vct_reject_reason = reason;
                    if accepted {
                        trace.tactical_path = "vct";
                        self.last_trace = Some(trace);
                        return SearchResult {
                            move_,
                            score: INF,
                            depth: 0,
                            nodes: 0,
                        };
                    }
                }
            }
        }
        self.last_trace = Some(trace.clone());

        let ab_start = Instant::now();
        let result = self.search_alphabeta_only(board, side, limits, &mut trace, None);
        trace.alphabeta_ms = Some(elapsed_ms(ab_start));
        self.last_trace = Some(trace);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::config::load_default_config;
    use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, WHITE};

    #[test]
    fn root_search_handles_no_legal_fallback_move_without_panicking() {
        let mut board = Board::new();
        board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                if x == 0 && y == 0 {
                    continue;
                }
                board.grid_rows_mut()[y][x] = if (x + y) % 2 == 0 { BLACK } else { WHITE };
            }
        }
        board.force_side_to_move(BLACK).unwrap();

        let mut caches = EvalCaches::new();
        recompute_all(&mut board, &mut caches);
        let mut rng = new_classic_fallback_rng();
        assert!(fallback_ai_move(&board, &caches, BLACK, RuleSet::Renju, &mut rng).is_err());

        let mut config = load_default_config();
        config.rule_set = RuleSet::Renju;
        let mut searcher = RootSearcher::new(config);
        let result = searcher.search(
            &mut board,
            Some(SearchLimits {
                max_depth: 1,
                root_width: BOARD_AREA,
                ..SearchLimits::default()
            }),
        );
        assert_eq!(result.score, -INF);
    }
}
