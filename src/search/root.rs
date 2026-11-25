//! Root iterative-deepening search for the classic mainline.

use super::abdada::{
    pick_best_thread, AbdadaContext, DepthJob, LazySmpJob, SearchWorkerPool, SharedRootBest,
    ThreadSearchOutcome, ABDADA_PARALLEL_START_DEPTH,
};
use super::budget::MoveBudget;
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Instant;

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::{default_tt_bits_for_search, EngineConfig};
use crate::constants::{INF, WIN};
use crate::eval::{recompute_all_for_rule, EvalCaches};
use crate::rules::RuleSet;
use crate::search::{
    AlphaBetaSearcher, RootCandidateProfile, SearchOptions, SearchStats, TranspositionTable,
};
use crate::threats::{
    has_vct_trigger_for_rule, DfpnSearcher, ThreatBoardView, VCFSearcher, VCTResult, VCTSearcher,
    VCTStats,
};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootSearchError {
    TerminalPosition,
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
    pub stop_reason: Option<&'static str>,
    pub root_profiles: Vec<RootDepthProfile>,
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
            stop_reason: None,
            root_profiles: Vec::new(),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallbackMoveScore {
    pub move_: Move,
    pub offensive: i64,
    pub defensive: i64,
    pub total: i64,
}

pub fn fallback_move_score(
    board: &Board,
    caches: &EvalCaches,
    side: Side,
    rule: RuleSet,
    move_: Move,
) -> Option<FallbackMoveScore> {
    let legal = if rule == RuleSet::Freestyle {
        board.is_legal_move(move_)
    } else {
        board.is_legal_move_for_rule(move_, side, rule)
    };
    if !legal {
        return None;
    }

    let player = if side == 1 { 0 } else { 1 };
    let opponent = 1 - player;
    let (x, y) = move_to_xy(move_).expect("legal move stays in range");

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

    Some(FallbackMoveScore {
        move_,
        offensive,
        defensive,
        total: 5 * offensive + 5 * defensive,
    })
}

pub fn fallback_ai_move(
    board: &Board,
    caches: &EvalCaches,
    side: Side,
    rule: RuleSet,
    rng: &mut ClassicFallbackRng,
) -> Result<Move, &'static str> {
    let mut best_value = i64::MIN;
    let mut best_moves = Vec::new();

    for move_index in 0..(board.size() * board.size()) {
        let move_ = move_index as Move;
        let Some(score) = fallback_move_score(board, caches, side, rule, move_) else {
            continue;
        };
        if score.total > best_value {
            best_value = score.total;
            best_moves.clear();
            best_moves.push(move_);
        } else if score.total == best_value {
            best_moves.push(move_);
        }
    }

    if best_moves.is_empty() {
        return Err("fallback AIs found no legal move on non-terminal board");
    }
    Ok(best_moves[rng.randrange(best_moves.len())?])
}

#[derive(Debug)]
pub struct RootSearcher {
    pub config: EngineConfig,
    pub tt: TranspositionTable,
    pub alphabeta: AlphaBetaSearcher,
    pub vcf: VCFSearcher,
    pub vct: VCTSearcher,
    pub dfpn: DfpnSearcher,
    pub fallback_rng: ClassicFallbackRng,
    pub last_trace: Option<RootTrace>,
    worker_pool: Option<SearchWorkerPool>,
}

impl Clone for RootSearcher {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            tt: self.tt.clone(),
            alphabeta: self.alphabeta.clone(),
            vcf: self.vcf.clone(),
            vct: self.vct.clone(),
            dfpn: self.dfpn.clone(),
            fallback_rng: self.fallback_rng.clone(),
            last_trace: self.last_trace.clone(),
            worker_pool: None,
        }
    }
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
        let depth = config.root_search.depth;
        let bits = default_tt_bits_for_search(config.runtime.threads, depth);
        let nnue = super::alphabeta::load_nnue_for_config(&config);
        let (tt, _) = TranspositionTable::try_new_with_fallback_layout(bits, 16, nnue.is_some())
            .expect("transposition table allocation succeeds");
        Self::with_tt_and_nnue(config, tt, nnue)
    }

    pub fn new_for_limits(config: EngineConfig, limits: SearchLimits) -> Self {
        let effective_threads = if limits.node_limit.is_some() {
            1
        } else {
            config.runtime.threads
        };
        let bits = default_tt_bits_for_search(effective_threads, limits.max_depth);
        let nnue = super::alphabeta::load_nnue_for_config(&config);
        let (tt, _) = TranspositionTable::try_new_with_fallback_layout(bits, 16, nnue.is_some())
            .expect("transposition table allocation succeeds");
        Self::with_tt_and_nnue(config, tt, nnue)
    }

    pub fn with_tt(config: EngineConfig, tt: TranspositionTable) -> Self {
        let nnue = super::alphabeta::load_nnue_for_config(&config);
        Self::with_tt_and_nnue(config, tt, nnue)
    }

    /// Construct a `RootSearcher` with an explicit TT bit size, choosing the
    /// single TT plane layout (4-way vs 2-way) from the actually loaded evaluator.
    pub fn try_with_tt_bits(
        config: EngineConfig,
        bits: u32,
    ) -> Result<Self, crate::search::TTError> {
        let nnue = super::alphabeta::load_nnue_for_config(&config);
        let tt = TranspositionTable::try_new_with_layout(bits, nnue.is_some())?;
        Ok(Self::with_tt_and_nnue(config, tt, nnue))
    }

    pub(crate) fn with_tt_and_nnue(
        config: EngineConfig,
        mut tt: TranspositionTable,
        nnue: Option<crate::nnue::Mix9svqEvaluator>,
    ) -> Self {
        tt.set_subply(config.runtime.threads > 1 || nnue.is_some());
        let alphabeta = AlphaBetaSearcher::with_tt_and_nnue(config.clone(), tt.clone(), nnue);
        Self {
            config,
            tt,
            alphabeta,
            vcf: VCFSearcher::default(),
            vct: VCTSearcher::default(),
            dfpn: DfpnSearcher::default(),
            fallback_rng: new_classic_fallback_rng(),
            last_trace: None,
            worker_pool: None,
        }
    }

    pub fn has_nnue(&self) -> bool {
        self.alphabeta.has_nnue()
    }

    pub(crate) fn take_worker_pool(&mut self) -> Option<SearchWorkerPool> {
        self.worker_pool.take()
    }

    pub(crate) fn set_worker_pool(&mut self, pool: Option<SearchWorkerPool>) {
        self.worker_pool = pool;
    }

    fn ensure_worker_pool(&mut self, threads: usize) -> &mut SearchWorkerPool {
        let desired_workers = threads.saturating_sub(1);
        let needs_rebuild = self
            .worker_pool
            .as_ref()
            .is_none_or(|pool| pool.requested_workers() != desired_workers);
        if needs_rebuild {
            self.worker_pool = Some(SearchWorkerPool::new(desired_workers));
        }
        self.worker_pool.as_mut().expect("worker pool initialized")
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
        let (mx, my) = move_to_xy(move_).expect("verified move stays in bounds");
        let mut view = ThreatBoardView::from_board_with_rule(trial.clone(), self.config.rule_set);
        if view.first_counter_win(-side, false).is_some() {
            return (false, Some("opponent_forcing"));
        }
        if view.has_winning_a4(mx, my) {
            if self.vcf.budget.stopped() {
                return (false, Some("budget"));
            }
            return (true, None);
        }
        if !view.forcing_threat_moves(-side).is_empty() {
            return (false, Some("opponent_forcing"));
        }
        if self.config.runtime.compute_vcf
            && self
                .vcf
                .search_with_multi_reply_for_rule(
                    &trial,
                    -side,
                    self.config.runtime.vct_verify_opponent_vcf_depth,
                    self.config.runtime.vcf_multi_reply,
                    self.config.rule_set,
                )
                .found
        {
            return (false, Some("opponent_vcf"));
        }
        if self.vcf.budget.stopped() {
            return (false, Some("budget"));
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
        if !self.config.runtime.compute_vcf {
            return allowed_moves;
        }
        let opponent_vcf = self.vcf.search_with_multi_reply_for_rule(
            board,
            -side,
            self.config.runtime.opponent_vcf_depth,
            self.config.runtime.vcf_multi_reply,
            self.config.rule_set,
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
            if self.vcf.budget.stopped() {
                return None;
            }
            let mut trial = board.clone();
            trial
                .play_assuming_rule_legal(move_, Some(side), self.config.rule_set)
                .expect("candidate move stays legal on trial board");
            if !self
                .vcf
                .search_with_multi_reply_for_rule(
                    &trial,
                    -side,
                    self.config.runtime.opponent_vcf_depth,
                    self.config.runtime.vcf_multi_reply,
                    self.config.rule_set,
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
        recompute_all_for_rule(board, &mut caches, self.config.rule_set);

        self.alphabeta.sync_config(&self.config);
        let subply = self.config.runtime.threads > 1 || self.alphabeta.has_nnue();
        self.tt.set_subply(subply);
        self.alphabeta.tt.set_subply(subply);
        self.alphabeta.advance_tt_generation();
        let mut best_move = None;
        let mut best_score = -INF;
        let mut total_nodes = 0_usize;
        if self.vcf.budget.stopped()
            || cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return self.fallback_search_result(board, &caches, side, best_score, 0, 0);
        }
        let root_allowed_moves =
            self.apply_opponent_vcf_filter(board, side, self.root_allowed_moves(board));
        if self.vcf.budget.stopped()
            || cancel
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

        if self.config.runtime.threads > 1
            && self.alphabeta.has_nnue()
            && limits.node_limit.is_none()
            && limits.max_depth >= 2
        {
            let threads = self.config.runtime.threads;
            let pool = self.ensure_worker_pool(threads);
            let num_workers = pool.num_workers();
            let table_and_salt = (num_workers > 0)
                .then(|| pool.table().map(|table| (table, pool.next_salt())))
                .flatten();
            if let Some((abdada_table, abdada_salt)) = table_and_salt {
                let total_threads = num_workers + 1;
                let stop_flag = Arc::new(AtomicBool::new(false));
                let completed_depths: Vec<AtomicI32> =
                    (0..total_threads).map(|_| AtomicI32::new(0)).collect();
                let latest_scores: Vec<AtomicI32> =
                    (0..total_threads).map(|_| AtomicI32::new(0)).collect();
                let job = Arc::new(LazySmpJob {
                    config: Arc::new(self.config.clone()),
                    tt: self.tt.clone(),
                    tt_generation: self.alphabeta.tt_generation(),
                    board: board.clone(),
                    caches: caches.clone(),
                    side,
                    max_depth: limits.max_depth,
                    wide: limits.root_width,
                    root_allowed_moves: root_allowed_moves.clone(),
                    deadline: self.vcf.budget.deadline(),
                    cancel: cancel.clone(),
                    stop_flag: Arc::clone(&stop_flag),
                    abdada_table: Arc::clone(&abdada_table),
                    abdada_salt,
                    completed_depths,
                    latest_scores,
                    thread_outcomes: Mutex::new(Vec::with_capacity(total_threads)),
                    helper_nodes: AtomicUsize::new(0),
                    helper_tt_curr: AtomicUsize::new(0),
                    helper_tt_old: AtomicUsize::new(0),
                });
                self.worker_pool
                    .as_ref()
                    .expect("worker pool initialized")
                    .start_lazy_smp(Arc::clone(&job));

                const WIN_MATE_CONFIRM_ITERS: i32 = 2;
                const LOSS_MATE_CONFIRM_ITERS: i32 = 6;
                let mut completed_depth = 0_i32;
                let mut first_win_mate_depth: Option<i32> = None;
                let mut first_loss_mate_depth: Option<i32> = None;
                let mut real_win_mate_confirms = 0_i32;
                let mut real_loss_mate_confirms = 0_i32;
                let mut stop_reason: &'static str = "max_depth";
                for depth in 1..=limits.max_depth {
                    if self.vcf.budget.stopped()
                        || cancel
                            .as_ref()
                            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
                    {
                        stop_reason = "budget_or_cancel";
                        break;
                    }
                    let depth_start = Instant::now();
                    let main_abdada = AbdadaContext {
                        worker_id: 0,
                        table: Arc::clone(&abdada_table),
                        salt: abdada_salt,
                        root_best: Arc::new(SharedRootBest::new()),
                        depth_done: Arc::clone(&stop_flag),
                    };
                    let root_preferred_move = if root_allowed_moves.is_some() {
                        best_move
                    } else {
                        None
                    };
                    let mut stats = SearchStats {
                        root_profile: self.config.runtime.root_profile,
                        cancel: cancel.clone(),
                        ..SearchStats::default()
                    };
                    let (score, move_) = self.alphabeta.search_with_abdada(
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
                            root_preferred_move,
                            ..SearchOptions::default()
                        },
                        Some(&main_abdada),
                    );
                    let depth_elapsed_us = depth_start.elapsed().as_micros();
                    total_nodes += stats.nodes;
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
                    if stats.stop || (best_move.is_some() && self.vcf.budget.stopped()) {
                        stop_reason = "budget_or_cancel";
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
                    job.latest_scores[0].store(best_score, Ordering::Relaxed);
                    job.completed_depths[0].store(completed_depth, Ordering::Release);
                    // An iteration that only returned via root TT cutoff
                    // (`nodes == 1, tt_hits == 1`) or solely via 1-node TT
                    // cutoffs on root children (`nodes == k + 1, tt_hits == k`)
                    // did not perform an actual deeper re-search and must not
                    // count as an independent mate confirmation round.
                    let tt_only_iter = stats.nodes <= stats.tt_hits + 1;
                    if best_score >= WIN {
                        first_loss_mate_depth = None;
                        real_loss_mate_confirms = 0;
                        if first_win_mate_depth.is_none() {
                            first_win_mate_depth = Some(depth);
                            real_win_mate_confirms = 0;
                        } else if !tt_only_iter {
                            real_win_mate_confirms += 1;
                        }
                        if best_score >= INF - 2 || real_win_mate_confirms >= WIN_MATE_CONFIRM_ITERS
                        {
                            stop_reason = "win_mate_confirm";
                            break;
                        }
                    } else if best_score <= -WIN {
                        first_win_mate_depth = None;
                        real_win_mate_confirms = 0;
                        let start = if let Some(s) = first_loss_mate_depth {
                            if !tt_only_iter {
                                real_loss_mate_confirms += 1;
                            }
                            s
                        } else {
                            first_loss_mate_depth = Some(depth);
                            real_loss_mate_confirms = 0;
                            depth
                        };
                        if real_loss_mate_confirms >= LOSS_MATE_CONFIRM_ITERS {
                            let helper_disagrees = (1..total_threads).any(|tid| {
                                job.completed_depths[tid].load(Ordering::Acquire) >= start
                                    && job.latest_scores[tid].load(Ordering::Relaxed) > -WIN
                            });
                            if !helper_disagrees {
                                stop_reason = "loss_mate_confirm";
                                break;
                            }
                        }
                    } else {
                        first_win_mate_depth = None;
                        first_loss_mate_depth = None;
                        real_win_mate_confirms = 0;
                        real_loss_mate_confirms = 0;
                    }
                }
                trace.stop_reason = Some(stop_reason);
                if self.config.runtime.root_profile {
                    self.last_trace = Some(trace.clone());
                }

                stop_flag.store(true, Ordering::Relaxed);
                self.worker_pool
                    .as_ref()
                    .expect("worker pool initialized")
                    .wait_depth_done();
                let helper_nodes = job.helper_nodes.load(Ordering::Relaxed);
                self.vcf.budget.add_nodes(helper_nodes);
                total_nodes += helper_nodes;
                trace.tt_bestmove_current_generation += job.helper_tt_curr.load(Ordering::Relaxed);
                trace.tt_bestmove_old_generation += job.helper_tt_old.load(Ordering::Relaxed);

                let mut outcomes = Vec::with_capacity(total_threads);
                if completed_depth > 0 && best_move.is_some() {
                    outcomes.push(ThreadSearchOutcome {
                        thread_id: 0,
                        completed_depth,
                        score: best_score,
                        best_move,
                    });
                }
                {
                    let guard = job
                        .thread_outcomes
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    outcomes.extend(guard.iter().copied());
                }
                if let Some(chosen) = pick_best_thread(&outcomes) {
                    return SearchResult {
                        move_: chosen.best_move.expect("checked"),
                        score: chosen.score,
                        depth: chosen.completed_depth,
                        nodes: total_nodes,
                    };
                }
                if let Some(move_) = best_move {
                    return SearchResult {
                        move_,
                        score: best_score,
                        depth: completed_depth,
                        nodes: total_nodes,
                    };
                }
                return self.fallback_search_result(
                    board,
                    &caches,
                    side,
                    best_score,
                    completed_depth,
                    total_nodes,
                );
            }
        }

        let mut completed_depth = 0_i32;
        for depth in 1..=limits.max_depth {
            if self.vcf.budget.stopped()
                || cancel
                    .as_ref()
                    .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
            {
                break;
            }
            let depth_start = Instant::now();
            let use_parallel_depth = self.config.runtime.threads > 1
                && limits.node_limit.is_none()
                && depth >= ABDADA_PARALLEL_START_DEPTH;
            let root_preferred_move = if root_allowed_moves.is_some() {
                best_move
            } else {
                None
            };
            let parallel_setup = if use_parallel_depth {
                let threads = self.config.runtime.threads;
                let pool = self.ensure_worker_pool(threads);
                if pool.num_workers() > 0 {
                    pool.table().map(|table| (table, pool.next_salt()))
                } else {
                    None
                }
            } else {
                None
            };
            let (mut score, mut move_, mut stats) = if let Some((abdada_table, abdada_salt)) =
                parallel_setup
            {
                let config_arc = Arc::new(self.config.clone());
                let tt_clone = self.tt.clone();
                let tt_gen = self.alphabeta.tt_generation();
                let deadline = self.vcf.budget.deadline();
                let depth_done = Arc::new(AtomicBool::new(false));
                let root_best = Arc::new(SharedRootBest::new());
                let job = Arc::new(DepthJob {
                    config: config_arc,
                    tt: tt_clone,
                    tt_generation: tt_gen,
                    board: board.clone(),
                    caches: caches.clone(),
                    side,
                    depth,
                    wide: limits.root_width,
                    root_allowed_moves: root_allowed_moves.clone(),
                    root_preferred_move,
                    deadline,
                    cancel: cancel.clone(),
                    depth_done: Arc::clone(&depth_done),
                    abdada_table: Arc::clone(&abdada_table),
                    abdada_salt,
                    root_best: Arc::clone(&root_best),
                    completed_result: Mutex::new(None),
                    helper_nodes: AtomicUsize::new(0),
                    helper_tt_curr: AtomicUsize::new(0),
                    helper_tt_old: AtomicUsize::new(0),
                });
                self.worker_pool
                    .as_ref()
                    .expect("worker pool initialized")
                    .start_depth(Arc::clone(&job));
                let mut stats = SearchStats {
                    root_profile: self.config.runtime.root_profile,
                    cancel: cancel.clone(),
                    ..SearchStats::default()
                };
                let main_abdada = AbdadaContext {
                    worker_id: 0,
                    table: abdada_table,
                    salt: abdada_salt,
                    root_best,
                    depth_done: Arc::clone(&depth_done),
                };
                let (mut main_score, mut main_move) = self.alphabeta.search_with_abdada(
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
                        root_preferred_move,
                        ..SearchOptions::default()
                    },
                    Some(&main_abdada),
                );
                depth_done.store(true, Ordering::Relaxed);
                self.worker_pool
                    .as_ref()
                    .expect("worker pool initialized")
                    .wait_depth_done();
                let helper_nodes = job.helper_nodes.load(Ordering::Relaxed);
                self.vcf.budget.add_nodes(helper_nodes);
                stats.nodes += helper_nodes;
                stats.tt_bestmove_current_generation += job.helper_tt_curr.load(Ordering::Relaxed);
                stats.tt_bestmove_old_generation += job.helper_tt_old.load(Ordering::Relaxed);
                if stats.stop {
                    let externally_stopped = self.vcf.budget.stopped()
                        || cancel
                            .as_ref()
                            .is_some_and(|cancel| cancel.load(Ordering::Relaxed));
                    if !externally_stopped {
                        if let Some((w_score, w_move)) = *job
                            .completed_result
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                        {
                            main_score = w_score;
                            main_move = w_move;
                            stats.stop = false;
                        }
                    }
                }
                if !stats.stop {
                    if let Some((shared_score, shared_move)) =
                        job.root_best.load_if_better(main_score)
                    {
                        main_score = shared_score;
                        main_move = Some(shared_move);
                    }
                }
                (main_score, main_move, stats)
            } else {
                let mut stats = SearchStats {
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
                        root_preferred_move,
                        ..SearchOptions::default()
                    },
                );
                (score, move_, stats)
            };
            let _ = (&mut score, &mut move_);
            let depth_elapsed_us = depth_start.elapsed().as_micros();
            total_nodes += stats.nodes;
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
            if stats.stop || (best_move.is_some() && self.vcf.budget.stopped()) {
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

    fn run_root_vct(
        &mut self,
        board: &Board,
        side: Side,
        sub_budget: Option<MoveBudget>,
    ) -> (VCTResult, VCTStats) {
        if self.config.runtime.vct_use_dfpn {
            let parent_budget = sub_budget
                .as_ref()
                .map(|sub| std::mem::replace(&mut self.dfpn.budget, sub.clone()));
            let dfpn_max_ply = (self.config.runtime.root_vct_depth * 3).clamp(2, 22);
            let result = self
                .dfpn
                .search_for_rule(board, side, dfpn_max_ply, self.config.rule_set);
            if let (Some(sub), Some(parent)) = (sub_budget, parent_budget) {
                self.dfpn.budget = parent;
                self.vcf.budget.add_nodes(sub.nodes().unwrap_or(0));
            }
            let mut stats = VCTStats::default();
            stats.depth_limit = dfpn_max_ply;
            stats.depth_completed = if self.dfpn.stats.aborted {
                0
            } else {
                dfpn_max_ply
            };
            stats.or_nodes = self.dfpn.stats.or_nodes;
            stats.and_nodes = self.dfpn.stats.and_nodes;
            stats.memo_exact_hits = self.dfpn.stats.tt_hits;
            stats.elapsed_us = (self.dfpn.stats.elapsed_ms * 1000.0) as u128;
            (result, stats)
        } else {
            self.vct.memo_diagnostics_enabled = self.config.runtime.root_profile;
            self.vct.strict_and_memo_key = self.config.runtime.vct_strict_and_memo_key;
            let parent_budget = sub_budget
                .as_ref()
                .map(|sub| std::mem::replace(&mut self.vct.budget, sub.clone()));
            let result = self.vct.search_for_rule(
                board,
                side,
                self.config.runtime.root_vct_depth,
                self.config.rule_set,
            );
            if let (Some(sub), Some(parent)) = (sub_budget, parent_budget) {
                self.vct.budget = parent;
                self.vcf.budget.add_nodes(sub.nodes().unwrap_or(0));
            }
            (result, self.vct.stats.clone())
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
        let subply = self.config.runtime.threads > 1 || self.alphabeta.has_nnue();
        self.tt.set_subply(subply);
        self.alphabeta.tt.set_subply(subply);
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
        let (vct_result, vct_stats) = self.run_root_vct(board, side, None);
        trace.vct_ms = Some(elapsed_ms(vct_start));
        trace.vct_stats = Some(vct_stats);
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
        trace.tt_bestmove_current_generation = ab_result.trace.tt_bestmove_current_generation;
        trace.tt_bestmove_old_generation = ab_result.trace.tt_bestmove_old_generation;
        self.last_trace = Some(trace);
        ab_result.result
    }

    pub fn try_search(
        &mut self,
        board: &mut Board,
        limits: Option<SearchLimits>,
    ) -> Result<SearchResult, RootSearchError> {
        if board.is_terminal() {
            self.last_trace = Some(RootTrace::default());
            return Err(RootSearchError::TerminalPosition);
        }
        Ok(self.search(board, limits))
    }

    pub fn search(&mut self, board: &mut Board, limits: Option<SearchLimits>) -> SearchResult {
        let limits = limits.unwrap_or_else(|| SearchLimits::fixed_from_config(&self.config));
        let budget = MoveBudget::new(limits);
        self.vcf.budget = budget.clone();
        self.vct.budget = budget.clone();
        self.dfpn.budget = budget.clone();
        self.alphabeta.vcf.budget = budget.clone();
        let mut result = self.search_move(board, limits);
        if let Some(nodes) = budget.nodes() {
            result.nodes = nodes;
            self.vcf.budget = MoveBudget::default();
            self.vct.budget = MoveBudget::default();
            self.dfpn.budget = MoveBudget::default();
            self.alphabeta.vcf.budget = MoveBudget::default();
        }
        result
    }

    fn search_move(&mut self, board: &mut Board, limits: SearchLimits) -> SearchResult {
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

        self.alphabeta.sync_config(&self.config);
        let side = board.side_to_move();
        let mut trace = RootTrace::default();
        self.last_trace = Some(trace.clone());
        let vcf_enabled = self.config.runtime.compute_vcf;

        if vcf_enabled {
            trace.used_vcf = true;
            let effective_vcf_depth = self
                .config
                .runtime
                .effective_root_vcf_depth(self.has_nnue());
            let vcf_result = if self.has_nnue() && effective_vcf_depth > 0 {
                let deep_depth = effective_vcf_depth;
                const TIMED_DEEP_VCF_BUDGET_FRACTION: f64 = 0.03;
                let vcf_sub_budget = self
                    .vcf
                    .budget
                    .timed_sub_budget(TIMED_DEEP_VCF_BUDGET_FRACTION);
                let parent_budget = vcf_sub_budget
                    .as_ref()
                    .map(|sub| std::mem::replace(&mut self.vcf.budget, sub.clone()));
                let res = self.vcf.search_deep_for_rule(
                    board,
                    side,
                    deep_depth,
                    self.config.runtime.vcf_multi_reply,
                    self.config.rule_set,
                    20_000,
                );
                if let (Some(sub), Some(parent)) = (vcf_sub_budget, parent_budget) {
                    self.vcf.budget = parent;
                    self.vcf.budget.add_nodes(sub.nodes().unwrap_or(0));
                }
                res
            } else {
                self.vcf.search_with_multi_reply_for_rule(
                    board,
                    side,
                    effective_vcf_depth,
                    self.config.runtime.vcf_multi_reply,
                    self.config.rule_set,
                )
            };
            if vcf_result.found && !self.vcf.budget.stopped() {
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

        if !self.vcf.budget.stopped()
            && self.config.runtime.compute_vct
            && self.config.runtime.root_vct_depth > 0
        {
            trace.used_vct = true;
            if has_vct_trigger_for_rule(board, side, self.config.rule_set) {
                trace.vct_triggered = true;
                if self.can_overlap_vct_alphabeta(&limits) {
                    return self.search_vct_and_alphabeta_overlap(board, side, limits, trace);
                }

                let vct_start = Instant::now();
                // Under a timed move limit, cap root VCT to 30% of the remaining
                // move time so a slow tactical tree cannot starve alpha-beta into
                // a depth-0 static fallback.
                const TIMED_VCT_BUDGET_FRACTION: f64 = 0.30;
                let vct_sub_budget = self.vcf.budget.timed_sub_budget(TIMED_VCT_BUDGET_FRACTION);
                let (vct_result, vct_stats) = self.run_root_vct(board, side, vct_sub_budget);
                trace.vct_ms = Some(elapsed_ms(vct_start));
                trace.vct_stats = Some(vct_stats);
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
    use crate::rules::ForbiddenKind;

    #[derive(serde::Deserialize)]
    struct FallbackStone {
        x: usize,
        y: usize,
        side: Side,
    }

    #[derive(serde::Deserialize)]
    struct FallbackPoint {
        x: usize,
        y: usize,
    }

    #[derive(serde::Deserialize)]
    struct FallbackFixture {
        name: String,
        moves: Vec<FallbackStone>,
        candidate: FallbackPoint,
        expected: String,
    }

    #[derive(serde::Deserialize)]
    struct RootRegressionFixture {
        moves: Vec<[usize; 2]>,
    }

    fn fallback_fixture(name: &str) -> (Board, Move, String) {
        let raw = include_str!("../../cases/renju/forbidden_hand_cases.jsonl");
        let case = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<FallbackFixture>(line).unwrap())
            .find(|case| case.name == name)
            .unwrap_or_else(|| panic!("missing fallback fixture {name}"));
        let mut board = Board::new();
        for stone in case.moves {
            board.grid_rows_mut()[stone.y][stone.x] = stone.side;
        }
        let candidate = xy_to_move(case.candidate.x, case.candidate.y).unwrap();
        (board, candidate, case.expected)
    }

    #[test]
    fn renju_fallback_scoring_matches_slowrenju_for_forbidden_and_recursive_cases() {
        let cases = [
            ("black_exact_five_priority", ForbiddenKind::None),
            ("black_overline_six", ForbiddenKind::Overline),
            ("simple_cross_double_four", ForbiddenKind::DoubleFour),
            ("true_cross_double_three", ForbiddenKind::DoubleThree),
            (
                "recursive_fake_three_both_gains_forbidden",
                ForbiddenKind::None,
            ),
        ];

        for (name, expected_kind) in cases {
            let (mut board, candidate, expected) = fallback_fixture(name);
            let mut freestyle_caches = EvalCaches::new();
            recompute_all_for_rule(&mut board, &mut freestyle_caches, RuleSet::Freestyle);
            let freestyle = fallback_move_score(
                &board,
                &freestyle_caches,
                BLACK,
                RuleSet::Freestyle,
                candidate,
            )
            .unwrap_or_else(|| panic!("{name}: freestyle candidate must be scored"));

            let mut renju_caches = EvalCaches::new();
            recompute_all_for_rule(&mut board, &mut renju_caches, RuleSet::Renju);
            let kind = board
                .forbidden_kind_for_rule(candidate, BLACK, RuleSet::Renju)
                .unwrap();
            assert_eq!(kind, expected_kind, "{name}: detector result changed");
            assert_eq!(
                expected != "none",
                expected_kind.is_forbidden(),
                "{name}: fixture expectation changed"
            );

            let renju =
                fallback_move_score(&board, &renju_caches, BLACK, RuleSet::Renju, candidate);
            if expected_kind.is_forbidden() {
                assert!(
                    renju.is_none(),
                    "{name}: forbidden point must be excluded from fallback scoring"
                );
            } else {
                let renju = renju.unwrap_or_else(|| panic!("{name}: legal point was suppressed"));
                assert_eq!(
                    renju, freestyle,
                    "{name}: legal point must keep the SlowRenju value1b score"
                );
            }
        }

        let (mut exact_five_board, exact_five, _) = fallback_fixture("black_exact_five_priority");
        let mut exact_five_caches = EvalCaches::new();
        recompute_all_for_rule(
            &mut exact_five_board,
            &mut exact_five_caches,
            RuleSet::Renju,
        );
        assert_eq!(
            fallback_move_score(
                &exact_five_board,
                &exact_five_caches,
                BLACK,
                RuleSet::Renju,
                exact_five,
            )
            .unwrap()
            .total,
            5_000_015
        );

        let (mut recursive_board, recursive, _) =
            fallback_fixture("recursive_fake_three_both_gains_forbidden");
        let mut recursive_caches = EvalCaches::new();
        recompute_all_for_rule(&mut recursive_board, &mut recursive_caches, RuleSet::Renju);
        assert_eq!(
            fallback_move_score(
                &recursive_board,
                &recursive_caches,
                BLACK,
                RuleSet::Renju,
                recursive,
            )
            .unwrap()
            .total,
            670
        );
    }

    // Renju: black's only "forcing" reply is the raw four 5,6,_,8,9 on row 7,
    // whose sole five point (7,7) completes the overline 3..9 and is therefore
    // forbidden — black has no real counter. The VCT verification must not
    // reject white's move because of that fake forcing threat.
    #[test]
    fn renju_vct_verification_ignores_fake_black_forcing() {
        let mut board = Board::new();
        for (x, y, side) in [
            (3, 7, BLACK),
            (4, 7, BLACK),
            (5, 7, BLACK),
            (6, 7, BLACK),
            (8, 7, BLACK),
            (2, 7, WHITE),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }

        let mut config = load_default_config();
        config.rule_set = RuleSet::Renju;
        let mut searcher = RootSearcher::new(config);
        let (accepted, reason) =
            searcher.verify_root_vct_move(&board, WHITE, xy_to_move(12, 12).unwrap());
        assert_eq!(reason, None, "black's fake four must not count as forcing");
        assert!(accepted);
    }

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
        recompute_all_for_rule(&mut board, &mut caches, RuleSet::Renju);
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

    #[test]
    fn root_win_priority_selects_actual_winning_move_without_vcf_or_vct() {
        let fixture: RootRegressionFixture =
            serde_json::from_str(include_str!("../../cases/renju/gui_normal_blunder_31.json"))
                .unwrap();

        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            for (move_count, expected_xy) in [(25, (5, 8)), (31, (8, 11))] {
                let mut board = Board::new();
                for &[x, y] in fixture.moves.iter().take(move_count) {
                    board
                        .play_for_rule(xy_to_move(x, y).unwrap(), None, rule)
                        .unwrap();
                }

                let mut config = load_default_config();
                config.rule_set = rule;
                config.runtime.compute_vcf = false;
                config.runtime.compute_vct = false;
                let mut searcher = RootSearcher::new(config);
                let result = searcher.search(
                    &mut board,
                    Some(SearchLimits {
                        max_depth: 4,
                        root_width: 20,
                        ..SearchLimits::default()
                    }),
                );

                assert_eq!(
                    move_to_xy(result.move_).unwrap(),
                    expected_xy,
                    "wrong {rule:?} win-priority move after {move_count} moves"
                );
                assert_eq!(result.score, INF);
            }
        }
    }

    #[test]
    fn root_vct_verification_accepts_winning_open_four_against_opponent_non_immediate_forcing() {
        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            let mut board = Board::new();
            // Black has three in a row at (6,7),(7,7),(8,7); playing (5,7) makes an unstoppable open four.
            // White has three in a row at (6,10),(7,10),(8,10), so white has a forcing b4/a4 threat
            // on row 10, but NO immediate 1-move five.
            for (x, y, side) in [
                (6, 7, BLACK),
                (6, 10, WHITE),
                (7, 7, BLACK),
                (7, 10, WHITE),
                (8, 7, BLACK),
                (8, 10, WHITE),
            ] {
                board
                    .play_for_rule(xy_to_move(x, y).unwrap(), Some(side), rule)
                    .unwrap();
            }

            let mut config = load_default_config();
            config.rule_set = rule;
            let mut searcher = RootSearcher::new(config);
            let (accepted, reason) =
                searcher.verify_root_vct_move(&board, BLACK, xy_to_move(5, 7).unwrap());
            assert!(
                accepted,
                "{rule:?}: winning open four should not be rejected when opponent has no immediate five (reason={reason:?})"
            );
            assert_eq!(reason, None);
        }
    }

    #[test]
    fn root_vct_verification_rejects_open_four_when_opponent_has_immediate_five() {
        for rule in [RuleSet::Freestyle, RuleSet::Renju] {
            let mut board = Board::new();
            // Black can make an open four at (5,7), but White already has four in a row at
            // (6,10)..(9,10) and wins immediately on the next move.
            for (x, y, side) in [
                (6, 7, BLACK),
                (6, 10, WHITE),
                (7, 7, BLACK),
                (7, 10, WHITE),
                (8, 7, BLACK),
                (8, 10, WHITE),
                (1, 1, BLACK),
                (9, 10, WHITE),
            ] {
                board
                    .play_for_rule(xy_to_move(x, y).unwrap(), Some(side), rule)
                    .unwrap();
            }

            let mut config = load_default_config();
            config.rule_set = rule;
            // Even with compute_vcf disabled, immediate 1-move opponent five must be rejected.
            config.runtime.compute_vcf = false;
            let mut searcher = RootSearcher::new(config);
            let (accepted, reason) =
                searcher.verify_root_vct_move(&board, BLACK, xy_to_move(5, 7).unwrap());
            assert!(
                !accepted,
                "{rule:?}: opponent immediate five must reject black open four"
            );
            assert_eq!(reason, Some("opponent_forcing"));
        }
    }

    #[test]
    fn renju_root_vct_verification_rejects_fake_open_four_with_overline_endpoint() {
        let mut board = Board::new();
        // Black stones at (6,7),(7,7),(8,7) plus (10,7): playing (5,7) makes an apparent open four
        // (5,7)..(8,7), but the right endpoint (9,7) would complete a 6-in-a-row overline
        // (5,7)..(10,7), so it is NOT a winning open four under Renju!
        // White has three in a row on row 10 (forcing counter-threat).
        for (x, y, side) in [
            (6, 7, BLACK),
            (6, 10, WHITE),
            (7, 7, BLACK),
            (7, 10, WHITE),
            (8, 7, BLACK),
            (8, 10, WHITE),
            (10, 7, BLACK),
            (1, 1, WHITE),
        ] {
            board.grid_rows_mut()[y][x] = side;
        }

        let mut config = load_default_config();
        config.rule_set = RuleSet::Renju;
        let mut searcher = RootSearcher::new(config);
        let (accepted, reason) =
            searcher.verify_root_vct_move(&board, BLACK, xy_to_move(5, 7).unwrap());
        assert!(
            !accepted,
            "Renju fake open four with overline endpoint must not bypass opponent_forcing"
        );
        assert_eq!(reason, Some("opponent_forcing"));
    }

    #[test]
    fn root_vct_verification_respects_expired_budget_even_on_winning_open_four() {
        let mut board = Board::new();
        for (x, y, side) in [
            (6, 7, BLACK),
            (6, 10, WHITE),
            (7, 7, BLACK),
            (7, 10, WHITE),
            (8, 7, BLACK),
            (8, 10, WHITE),
        ] {
            board
                .play_for_rule(xy_to_move(x, y).unwrap(), Some(side), RuleSet::Renju)
                .unwrap();
        }

        let mut config = load_default_config();
        config.rule_set = RuleSet::Renju;
        let mut searcher = RootSearcher::new(config);
        searcher.vcf.budget = MoveBudget::new(SearchLimits {
            time_limit_ms: Some(0.0),
            ..SearchLimits::default()
        });
        let (accepted, reason) =
            searcher.verify_root_vct_move(&board, BLACK, xy_to_move(5, 7).unwrap());
        assert!(!accepted);
        assert_eq!(reason, Some("budget"));
    }

    #[test]
    fn root_searcher_hot_reloads_nnue_on_config_mutation() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("opponent/rapfi-networks/mix9svq");
        if !dir.is_dir() {
            return;
        }
        let mut board = Board::new();
        for (x, y) in [(7, 7), (8, 7), (7, 8), (6, 6)] {
            board
                .play_for_rule(xy_to_move(x, y).unwrap(), None, RuleSet::Freestyle)
                .unwrap();
        }
        let limits = SearchLimits {
            max_depth: 2,
            root_width: 8,
            ..SearchLimits::default()
        };
        let mut config = load_default_config();
        config.rule_set = RuleSet::Freestyle;
        let mut searcher = RootSearcher::new_for_limits(config, limits);
        assert!(!searcher.has_nnue());

        // Hot-enable NNUE on an existing RootSearcher
        searcher.config.runtime.use_nnue = true;
        searcher.config.runtime.nnue_dir = Some(dir.to_string_lossy().into_owned());
        let _ = searcher.search(&mut board, Some(limits));
        assert!(searcher.has_nnue());

        // Hot-switch rule_set from Freestyle to Renju and reload Renju weights
        searcher.config.rule_set = RuleSet::Renju;
        let _ = searcher.search(&mut board, Some(limits));
        assert!(searcher.has_nnue());

        // Hot-disable NNUE
        searcher.config.runtime.use_nnue = false;
        let _ = searcher.search(&mut board, Some(limits));
        assert!(!searcher.has_nnue());
    }
}
