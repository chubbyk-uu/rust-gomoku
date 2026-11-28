//! Native Rapfi-aligned NNUE Alpha-Beta + QVCF + Pure Lazy SMP Searcher.
//!
//! Isolated completely from classical `pygomoku` (`src/search/`). Every
//! parameter, pruning margin, extension, LMR formula, aspiration loop,
//! QVCF search, and multi-thread depth/vote scheme follows
//! `opponent/rapfi/Rapfi/search/ab/{parameter.h,search.cpp}`.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::time::Instant;

use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, WHITE};
use crate::fast_engine::board::{chebyshev_distance, CandRange, FastBoard, NONE_MOVE, PASS_MOVE};
use crate::fast_engine::history::{rule_idx, CounterMoveHistory, HistoryTracker, MainHistory};
use crate::fast_engine::movegen::find_p4_pos;
use crate::fast_engine::movepick::{MovePicker, POLICY_SCORE_SCALE};
use crate::fast_engine::pattern::{Pattern4, FE_BLACK};
use crate::fast_engine::tt::{
    mate_in, mate_step, mated_in, FastTT, BOUND_EXACT, BOUND_LOWER, BOUND_NONE, BOUND_UPPER,
    DEPTH_LOWER_BOUND, DEPTH_NONE, DEPTH_QVCF, VALUE_DRAW, VALUE_EVAL_MAX, VALUE_EVAL_MIN,
    VALUE_INFINITE, VALUE_MATE, VALUE_MATED_IN_MAX_PLY, VALUE_MATE_IN_MAX_PLY, VALUE_NONE,
    VALUE_ZERO,
};
use crate::nnue::Mix9svqEvaluator;
use crate::rules::RuleSet;

pub const MAX_DEPTH: i32 = 200;
pub const MAX_PLY: usize = 256;
const PLY_BEFORE_ROOT: usize = 4;
const PLY_AFTER_MAX: usize = 2;
const STACK_LEN: usize = PLY_BEFORE_ROOT + MAX_PLY + PLY_AFTER_MAX;

const MARGIN_INFINITE: i32 = i16::MAX as i32;
const ASPIRATION_DEPTH: f32 = 5.3;
const IID_DEPTH: f32 = 14.5;

const IIR_REDUCTION: [f32; 3] = [0.71, 0.65, 0.65];
const IIR_REDUCTION_PV: [f32; 3] = [0.45, 0.43, 0.37];
const IIR_REDUCTION_TT: [f32; 3] = [0.19, 0.20, 0.17];
const IIR_REDUCTION_TT_MAX: [f32; 3] = [3.6, 3.8, 3.3];
const TRIVIAL_PRUN_DEPTH: f32 = 4.4;
const LMR_EXTRA_MAX_DEPTH: f32 = 5.1;
const LMR_TT_PV_SUBTRACT: [f32; 3] = [0.98, 0.81, 0.71];
const LMR_NO_ALPHA_ADD: [f32; 3] = [0.91, 0.96, 0.99];
const LMR_NO_KILLER_CUT_NODE_ADD: [f32; 3] = [1.8, 2.2, 1.8];
const FALSE_FORBID_LESS_REDUCTION: f32 = 1.03;
const ALPHA_IMPROVEMENT_REDUCTION: f32 = 1.05;
const OPPO_USELESS_DEFEND_REDUCTION: f32 = 2.07;
const SELF_USELESS_DEFEND_REDUCTION: f32 = 0.93;

const OPPO5_EXT: f32 = 1.1;
const SE_DEPTH: f32 = 6.85;
const SE_TTE_DEPTH: [f32; 3] = [2.09, 2.03, 1.95];
const SE_EXTRA_MAX_DEPTH: f32 = 10.8;
const SE_REDUCTION_FH: f32 = 1.48;
const TTM_EXT_PV: [f32; 3] = [0.16, 0.24, 0.14];
const TTM_EXT_NONPV: [f32; 3] = [0.08, 0.12, 0.10];
const NEARB4_EXT_DIST4: f32 = 0.27;
const NEARB4_EXT_DIST6: f32 = 0.05;
const CONTINUOUS_ATTACK_EXT: f32 = 0.41;

const NUM_ITERATION_AFTER_MATE: i32 = 24;
const NUM_ITERATION_AFTER_SINGULAR_ROOT: i32 = 24;

static LN_LUT: std::sync::LazyLock<[f32; 512]> = std::sync::LazyLock::new(|| {
    let mut lut = [0.0_f32; 512];
    for i in 1..512 {
        lut[i] = (i as f32).ln();
    }
    lut
});

#[inline(always)]
const fn rule_slot<const RENJU: bool>() -> usize {
    if RENJU {
        2
    } else {
        0
    }
}

/// Rapfi `Hash::LCHash` (`core/hash.h`) for singular-exclusion TT key perturbation.
#[inline(always)]
const fn lc_hash(x: u16) -> u64 {
    let mut k = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    k ^= k >> 30;
    k = k.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    k ^ (k >> 27)
}

#[inline(always)]
fn next_aspiration_window_delta(rule: RuleSet, prev_value: i32, prev_delta: i32) -> i32 {
    const INIT: [i32; 3] = [20, 10, 11];
    const ADD: [i32; 3] = [8, 3, 8];
    let r = rule_idx(rule);
    if prev_delta != 0 {
        prev_delta * (3 + prev_value.abs() / 1024) / 4 + ADD[r]
    } else {
        INIT[r]
    }
}

#[inline(always)]
fn razor_margin<const RENJU: bool>(d: f32) -> i32 {
    const QUAD: [f32; 3] = [0.17, 0.20, 0.17];
    const LIN: [i32; 3] = [15, -1, 8];
    const BIAS: [i32; 3] = [11, -1, 9];
    let r = rule_slot::<RENJU>();
    if d < 2.6 {
        ((QUAD[r] * d * d + LIN[r] as f32 * d) as i32 + BIAS[r]).max(0)
    } else {
        MARGIN_INFINITE
    }
}

#[inline(always)]
fn futility_margin<const RENJU: bool>(d: f32, no_tt_cut_node: bool, improving: bool) -> i32 {
    const SCALE: [i32; 3] = [68, 43, 63];
    const PENALTY: [i32; 3] = [16, 13, 13];
    let r = rule_slot::<RENJU>();
    let coeff = SCALE[r] - PENALTY[r] * (no_tt_cut_node as i32);
    (((coeff as f32) * (d - (improving as i32 as f32))) as i32).max(0)
}

#[inline(always)]
fn null_move_margin<const RENJU: bool>(d: f32) -> i32 {
    const NMM_DEPTH: [i32; 3] = [6, 15, 11];
    const NMM_SLOPE: [i32; 3] = [24, 32, 24];
    const NMM_FLOOR: [i32; 3] = [277, 197, 399];
    const NMM_HEADROOM: [i32; 3] = [38, 38, 6];
    let r = rule_slot::<RENJU>();
    let nmm_max = NMM_SLOPE[r] * NMM_DEPTH[r] + NMM_HEADROOM[r];
    let nmm_bias = nmm_max + NMM_FLOOR[r];
    if d >= NMM_DEPTH[r] as f32 {
        nmm_bias - (NMM_SLOPE[r] * (d as i32)).min(nmm_max)
    } else {
        MARGIN_INFINITE
    }
}

#[inline(always)]
fn null_move_reduction<const RENJU: bool>(d: f32) -> f32 {
    const BIAS: [f32; 3] = [3.70, 4.37, 4.21];
    const SCALE: [f32; 3] = [0.141, 0.156, 0.160];
    let r = rule_slot::<RENJU>();
    BIAS[r] + SCALE[r] * d
}

#[inline(always)]
fn iid_depth_reduction<const RENJU: bool>(d: f32) -> f32 {
    const BIAS: [f32; 3] = [8.4, 8.2, 9.7];
    const SCALE: [f32; 3] = [0.74, 0.56, 0.52];
    let r = rule_slot::<RENJU>();
    BIAS[r] + SCALE[r] * d
}

#[inline(always)]
fn fail_high_margin<const RENJU: bool>(d: f32, oppo4: bool) -> i32 {
    const BIAS: [i32; 3] = [-3, -11, -4];
    const SCALE: [i32; 3] = [20, 12, 16];
    const OPPO4_BONUS: [i32; 3] = [80, 91, 80];
    let r = rule_slot::<RENJU>();
    BIAS[r] + SCALE[r] * (d as i32) + OPPO4_BONUS[r] * (oppo4 as i32)
}

#[inline(always)]
fn futility_move_count(d: f32, improving: bool) -> i32 {
    let di = (d as i32).max(0);
    if di == 0 {
        return 0;
    }
    let mc = 3 + ((di as f64).powf(1.30) as i32);
    mc / (2 - improving as i32)
}

#[inline(always)]
fn singular_margin<const RENJU: bool>(d: f32, former_pv: bool) -> i32 {
    const SE_SCALE: [f32; 3] = [0.85, 0.85, 0.90];
    let r = rule_slot::<RENJU>();
    ((SE_SCALE[r] + former_pv as i32 as f32) * d) as i32
}

#[inline(always)]
fn singular_reduction(d: f32, former_pv: bool) -> f32 {
    d * 0.55 - (former_pv as i32 as f32)
}

#[inline(always)]
fn double_se_margin<const RENJU: bool>(d: f32) -> i32 {
    const SCALE: [f32; 3] = [0.71, 0.60, 0.58];
    const MAX_V: [i32; 3] = [4, 6, 8];
    let r = rule_slot::<RENJU>();
    29 - ((d * SCALE[r]) as i32).min(MAX_V[r])
}

#[inline(always)]
fn qvcf_delta_margin<const RENJU: bool>(d: f32) -> i32 {
    const BIAS: [i32; 3] = [2084, 1817, 2154];
    const SCALE: [i32; 3] = [62, 66, 53];
    let r = rule_slot::<RENJU>();
    (BIAS[r] + SCALE[r] * (d as i32)).max(768)
}

#[inline(always)]
fn lmr_extension<const RENJU: bool>(
    new_depth: f32,
    searched_depth: f32,
    value: i32,
    alpha: i32,
    best_value: i32,
) -> f32 {
    const BIAS: [i32; 3] = [22, 12, 20];
    const GAP: [i32; 3] = [321, 278, 253];
    const DIFF: [i32; 3] = [10, 3, 10];
    let r = rule_slot::<RENJU>();
    let dd = new_depth - searched_depth;
    let deeper = value > alpha + BIAS[r] + (DIFF[r] as f32 * dd) as i32;
    let even_deeper = value > alpha + BIAS[r] + GAP[r] + (27.0 * dd) as i32;
    let shallower = value < best_value + (new_depth as i32);
    (deeper as i32 + even_deeper as i32 - shallower as i32) as f32
}

#[inline(always)]
fn complexity_reduction<const RENJU: bool>(trivial: bool, important: bool, distract: bool) -> f32 {
    const CR1: [f32; 3] = [0.0920, 0.0697, 0.0600];
    const CR2: [f32; 3] = [0.0333, 0.0199, 0.0280];
    const CR3: [f32; 3] = [0.0250, 0.0206, 0.0229];
    const CR4: [f32; 3] = [0.0053, 0.0070, 0.0077];
    let r = rule_slot::<RENJU>();
    if trivial {
        if distract {
            CR1[r]
        } else {
            CR2[r]
        }
    } else if !important {
        CR3[r]
    } else {
        CR4[r]
    }
}

#[inline(always)]
fn policy_reduction<const RENJU: bool>(scaled_score: f32) -> f32 {
    const SCALE: [f32; 3] = [3.60, 1.76, 3.15];
    const BIAS: [f32; 3] = [2.53, 2.64, 3.39];
    const MAX_R: [f32; 3] = [5.18, 3.07, 4.90];
    let r = rule_slot::<RENJU>();
    (BIAS[r] - SCALE[r] * scaled_score).clamp(0.0, MAX_R[r])
}

#[inline(always)]
fn policy_pruning_score<const RENJU: bool>(d: f32) -> i32 {
    const BIAS: [i32; 3] = [372, 363, 377];
    const SCALE: [i32; 3] = [48, 63, 62];
    let r = rule_slot::<RENJU>();
    BIAS[r] - (d * SCALE[r] as f32) as i32
}

#[inline(always)]
fn extension_from_stat_score<const RENJU: bool>(stat_score: i32, depth: f32) -> f32 {
    const EXT_STAT_DEPTH: [f32; 3] = [5.3, 5.4, 5.4];
    let r = rule_slot::<RENJU>();
    let denom = 12266.0
        + if depth > EXT_STAT_DEPTH[r] {
            4380.0
        } else {
            0.0
        };
    (stat_score as f32) / denom
}

fn build_reduction_lut(num_threads: usize) -> [[f32; BOARD_AREA + 2]; 3] {
    const FACTOR: [f64; 3] = [0.95, 0.78, 0.83];
    let thread_bias = 0.1 * (num_threads.max(1) as f64).ln();
    let mut lut = [[0.0_f32; BOARD_AREA + 2]; 3];
    for r in 0..3 {
        for i in 1..(BOARD_AREA + 2) {
            lut[r][i] = (FACTOR[r] * ((i as f64).ln() + thread_bias)) as f32;
        }
    }
    lut
}

#[inline(always)]
fn position_complexity(board: &FastBoard, self_side: usize, oppo: usize) -> f32 {
    let mut count = 1_usize;
    for p4_u8 in (Pattern4::LFlex2 as u8)..=(Pattern4::DBlock4Plus as u8) {
        let p4 = Pattern4::from_u8(p4_u8);
        count += board.p4_count(self_side, p4) as usize;
        count += board.p4_count(oppo, p4) as usize;
    }
    if count < 512 {
        LN_LUT[count]
    } else {
        (count as f32).ln()
    }
}

#[derive(Clone, Copy)]
struct MoveTraits {
    important: bool,
    trivial: bool,
    distract: bool,
    dist_self: i32,
    dist_oppo: i32,
}

#[derive(Clone, Copy)]
struct StackEntry {
    ply: i32,
    move_count: i32,
    extra_extension: f32,
    stat_score: i32,
    static_eval: i32,
    current_move: u16,
    skip_move: u16,
    killers: [u16; 2],
    move_p4: [Pattern4; 2],
    num_null_moves: i16,
    tt_pv: bool,
}

impl Default for StackEntry {
    fn default() -> Self {
        Self {
            ply: 0,
            move_count: 0,
            extra_extension: 0.0,
            stat_score: 0,
            static_eval: VALUE_ZERO,
            current_move: NONE_MOVE,
            skip_move: NONE_MOVE,
            killers: [NONE_MOVE; 2],
            move_p4: [Pattern4::None; 2],
            num_null_moves: 0,
            tt_pv: false,
        }
    }
}

impl StackEntry {
    #[inline(always)]
    fn is_killer(&self, mv: u16) -> bool {
        self.killers[0] == mv || self.killers[1] == mv
    }
}

#[derive(Clone, Debug)]
pub struct RootMove {
    pub mv: u16,
    pub value: i32,
    pub previous_value: i32,
    pub sel_depth: i32,
    pub pv: Vec<u16>,
    pub previous_pv: Vec<u16>,
    pub nodes: u64,
}

impl RootMove {
    fn new(mv: u16) -> Self {
        Self {
            mv,
            value: VALUE_NONE,
            previous_value: VALUE_NONE,
            sel_depth: 0,
            pv: vec![mv],
            previous_pv: vec![mv],
            nodes: 0,
        }
    }
}

/// Configuration for a `FastSearcher::search` call.
#[derive(Clone, Debug)]
pub struct FastSearchConfig {
    pub max_depth: i32,
    pub max_nodes: Option<u64>,
    pub time_limit_ms: Option<u64>,
    pub time_left_ms: Option<u64>,
    pub num_threads: usize,
    pub cand_range: CandRange,
}

impl Default for FastSearchConfig {
    fn default() -> Self {
        Self {
            max_depth: 99,
            max_nodes: None,
            time_limit_ms: Some(1000),
            time_left_ms: None,
            num_threads: 1,
            cand_range: CandRange::Square3Line4,
        }
    }
}

/// Result returned by `FastSearcher::search`.
#[derive(Clone, Debug)]
pub struct FastSearchResult {
    pub best_move: u16,
    pub score: i32,
    pub depth: i32,
    pub sel_depth: i32,
    pub nodes: u64,
    pub pv: Vec<u16>,
    pub elapsed_ms: f64,
}

/// Persistent native NNUE searcher holding a shared 5-tier `FastTT` and
/// reusable helper evaluators for multi-thread Lazy SMP (avoiding ~8.5 MiB
/// per-thread heap allocations/clones on every move).
#[derive(Debug)]
pub struct FastSearcher {
    tt: FastTT,
    helper_evals: std::sync::Mutex<Vec<Mix9svqEvaluator>>,
}

impl FastSearcher {
    pub fn new(tt_size_kb: usize) -> Self {
        Self {
            tt: FastTT::new_kb(tt_size_kb),
            helper_evals: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn resize_tt_kb(&mut self, tt_size_kb: usize) {
        self.tt.resize_kb(tt_size_kb);
    }

    pub fn clear_tt(&self) {
        self.tt.clear();
    }

    pub fn tt(&self) -> &FastTT {
        &self.tt
    }

    /// Run an iterative-deepening native NNUE search (single- or multi-thread
    /// Lazy SMP) starting from `root_board` and its synced `evaluator`.
    pub fn search(
        &self,
        root_board: &FastBoard,
        evaluator: &Mix9svqEvaluator,
        cfg: &FastSearchConfig,
    ) -> FastSearchResult {
        let mut root_eval = evaluator.clone();
        self.search_mut(root_board, &mut root_eval, cfg)
    }

    /// Zero-clone variant of [`Self::search`] that reuses `evaluator` in-place
    /// on Thread 0 (strictly restoring it to the root position upon return) and
    /// incrementally syncs cached helper evaluators on `num_threads > 1`.
    pub fn search_mut(
        &self,
        root_board: &FastBoard,
        evaluator: &mut Mix9svqEvaluator,
        cfg: &FastSearchConfig,
    ) -> FastSearchResult {
        let start = Instant::now();
        let num_threads = cfg.num_threads.clamp(1, 64);

        // Compute Rapfi-aligned optimum/maximum time budget (ms).
        let (optimum_ms, maximum_ms) =
            compute_time_budget(cfg.time_limit_ms, cfg.time_left_ms, root_board.moves_left());

        let mut root_board_mut = root_board.clone();

        // Empty board fast path: play center (7, 7) immediately.
        if root_board_mut.move_count() == 0 {
            let center = (7 * BOARD_SIZE + 7) as u16;
            return FastSearchResult {
                best_move: center,
                score: 0,
                depth: 1,
                sel_depth: 1,
                nodes: 1,
                pv: vec![center],
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            };
        }

        let root_move_list = MovePicker::generate_root_moves(&mut root_board_mut, evaluator);
        if root_move_list.is_empty() {
            // Fallback: pick any legal empty cell (in Renju, Black may be forced to pick a cell).
            let mut fallback = NONE_MOVE;
            for pos in 0..BOARD_AREA {
                if root_board_mut.is_empty(pos) {
                    fallback = pos as u16;
                    if root_board_mut.is_legal_non_pass(fallback) {
                        break;
                    }
                }
            }
            return FastSearchResult {
                best_move: fallback,
                score: mated_in(2),
                depth: 0,
                sel_depth: 0,
                nodes: 1,
                pv: if fallback != NONE_MOVE {
                    vec![fallback]
                } else {
                    vec![]
                },
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            };
        }

        // Immediate winning move (A_FIVE) — return without wasting time.
        let self_side = root_board_mut.side_to_move();
        if root_board_mut.p4_count(self_side, Pattern4::AFive) > 0 {
            let mv = root_move_list[0];
            return FastSearchResult {
                best_move: mv,
                score: mate_in(1),
                depth: 1,
                sel_depth: 1,
                nodes: 1,
                pv: vec![mv],
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            };
        }

        self.tt.inc_generation();
        let reduction_lut = build_reduction_lut(num_threads);
        let stop = AtomicBool::new(false);
        let total_nodes = AtomicU64::new(0);
        let completed_depths: Vec<AtomicI32> =
            (0..num_threads).map(|_| AtomicI32::new(0)).collect();

        let shared = SharedSearchState {
            tt: &self.tt,
            stop: &stop,
            total_nodes: &total_nodes,
            completed_depths: &completed_depths,
            reduction_lut: &reduction_lut,
            start,
            optimum_ms,
            maximum_ms,
            time_check_mask: match maximum_ms {
                Some(m) if m <= 20 => 63,
                _ => 2047,
            },
            max_depth: cfg.max_depth.clamp(1, MAX_DEPTH),
            max_nodes: cfg.max_nodes,
            num_threads,
        };

        let init_side = if self_side == FE_BLACK { BLACK } else { WHITE };
        let init_eval = evaluator
            .evaluate_value(init_side)
            .clamp(VALUE_EVAL_MIN, VALUE_EVAL_MAX);

        let thread_results: Vec<ThreadOutcome> = if num_threads == 1 {
            let mut worker = SearchWorker::new(
                0,
                root_board_mut,
                evaluator,
                &root_move_list,
                init_eval,
                &shared,
            );
            worker.iterative_deepening();
            vec![worker.into_outcome()]
        } else {
            let helper_count = num_threads - 1;
            let mut helpers: Vec<Mix9svqEvaluator> = {
                let mut pool = self.helper_evals.lock().unwrap_or_else(|e| e.into_inner());
                let mut taken = Vec::with_capacity(helper_count);
                while taken.len() < helper_count {
                    if let Some(ev) = pool.pop() {
                        taken.push(ev);
                    } else {
                        break;
                    }
                }
                taken
            };
            // Sync cached helpers via LCP move-cache diff (~50ns each; flushed on
            // each worker thread in parallel), only cloning on the very first search.
            for ev in &mut helpers {
                ev.sync_from_evaluator(evaluator);
            }
            while helpers.len() < helper_count {
                helpers.push(evaluator.clone());
            }

            let (outcomes, returned_helpers) = std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(helper_count);
                for (idx, mut ev) in helpers.into_iter().enumerate() {
                    let tid = idx + 1;
                    let b = root_board_mut.clone();
                    let rm_ref = &root_move_list;
                    let sh_ref = &shared;
                    handles.push(s.spawn(move || {
                        let outcome = {
                            let mut w =
                                SearchWorker::new(tid, b, &mut ev, rm_ref, init_eval, sh_ref);
                            w.iterative_deepening();
                            w.into_outcome()
                        };
                        (outcome, ev)
                    }));
                }
                let main_outcome = {
                    let mut main_w = SearchWorker::new(
                        0,
                        root_board_mut,
                        evaluator,
                        &root_move_list,
                        init_eval,
                        &shared,
                    );
                    main_w.iterative_deepening();
                    stop.store(true, Ordering::Relaxed);
                    main_w.into_outcome()
                };
                let mut outcomes = Vec::with_capacity(num_threads);
                let mut ret_helpers = Vec::with_capacity(helper_count);
                outcomes.push(main_outcome);
                for h in handles {
                    let (out, ev) = h.join().expect("worker thread panicked");
                    outcomes.push(out);
                    ret_helpers.push(ev);
                }
                (outcomes, ret_helpers)
            });

            if let Ok(mut pool) = self.helper_evals.lock() {
                pool.extend(returned_helpers);
            }
            outcomes
        };

        let best = pick_best_thread(&thread_results);
        let nodes = total_nodes.load(Ordering::Relaxed);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        FastSearchResult {
            best_move: best.best_move,
            score: best.best_value,
            depth: best.completed_depth,
            sel_depth: best.sel_depth,
            nodes,
            pv: best.pv.clone(),
            elapsed_ms,
        }
    }
}

/// Rapfi-aligned time budget (`search/timecontrol.cpp`).
fn compute_time_budget(
    turn_ms: Option<u64>,
    time_left_ms: Option<u64>,
    moves_left: usize,
) -> (Option<u64>, Option<u64>) {
    const TURN_RESERVE_MS: u64 = 30;
    let safe_budget = |raw_ms: u64| -> u64 {
        if raw_ms >= TURN_RESERVE_MS * 2 {
            raw_ms - TURN_RESERVE_MS
        } else if raw_ms > 2 {
            (raw_ms / 2).max(1)
        } else {
            0
        }
    };
    let moves_est = ((moves_left / 2).max(1)).min(64) as u64;
    let max_ms = match (turn_ms, time_left_ms) {
        (Some(turn), Some(left)) => {
            let max_turn = safe_budget(turn);
            let safe_left = safe_budget(left);
            if max_turn == 0 || safe_left == 0 {
                0
            } else if turn.saturating_mul(moves_est) <= left {
                max_turn.min(safe_left)
            } else {
                // Tight match clock: allocate proportionally without raising above remaining time.
                let opt_left = (safe_left / moves_est.max(4)).max(1).min(safe_left);
                opt_left.min(max_turn)
            }
        }
        (Some(turn), None) => safe_budget(turn),
        (None, Some(left)) => {
            // Match-clock only (no per-turn limit): pace strictly inside remaining match time.
            let safe_left = safe_budget(left);
            if safe_left == 0 {
                0
            } else {
                (safe_left / moves_est.max(4)).max(1).min(safe_left)
            }
        }
        (None, None) => return (None, None),
    };
    let opt_ms = if max_ms == 0 {
        0
    } else {
        ((max_ms as f64) * 0.90) as u64
    };
    (Some(opt_ms.min(max_ms)), Some(max_ms))
}

struct SharedSearchState<'a> {
    tt: &'a FastTT,
    stop: &'a AtomicBool,
    total_nodes: &'a AtomicU64,
    completed_depths: &'a [AtomicI32],
    reduction_lut: &'a [[f32; BOARD_AREA + 2]; 3],
    start: Instant,
    optimum_ms: Option<u64>,
    maximum_ms: Option<u64>,
    time_check_mask: u64,
    max_depth: i32,
    max_nodes: Option<u64>,
    num_threads: usize,
}

impl<'a> SharedSearchState<'a> {
    #[inline(always)]
    fn is_terminating(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    #[inline]
    fn check_exit(&self, local_nodes: u64) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }
        if let Some(max_n) = self.max_nodes {
            if self.total_nodes.load(Ordering::Relaxed) + local_nodes >= max_n {
                self.stop.store(true, Ordering::Relaxed);
                return true;
            }
        }
        if let Some(max_ms) = self.maximum_ms {
            if (self.start.elapsed().as_millis() as u64) >= max_ms {
                self.stop.store(true, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    fn pick_next_depth(&self, tid: usize, last_depth: i32) -> i32 {
        if tid == 0 || self.num_threads < 3 {
            return last_depth + 1;
        }
        let mut next_depth = last_depth + 1;
        loop {
            let mut above = 0_usize;
            for (i, d) in self.completed_depths.iter().enumerate() {
                if i != tid && d.load(Ordering::Relaxed) + 1 >= next_depth {
                    above += 1;
                }
            }
            if above > self.num_threads / 2 {
                next_depth += 1;
            } else {
                return next_depth;
            }
        }
    }
}

struct ThreadOutcome {
    best_move: u16,
    best_value: i32,
    completed_depth: i32,
    sel_depth: i32,
    pv: Vec<u16>,
}

/// Rapfi `ABSearcher::pickBestThread` (`search/ab/search.cpp:538-581`).
fn pick_best_thread(outcomes: &[ThreadOutcome]) -> &ThreadOutcome {
    let mut best = &outcomes[0];
    if outcomes.len() <= 1 {
        return best;
    }
    let mut min_value = best.best_value;
    for o in &outcomes[1..] {
        if o.completed_depth > 0
            && o.best_value != VALUE_NONE
            && (min_value == VALUE_NONE || o.best_value < min_value)
        {
            min_value = o.best_value;
        }
    }
    if min_value == VALUE_NONE {
        return best;
    }
    let mut votes = [0_i64; BOARD_AREA];
    const DEPTH_BIAS: i64 = 15;
    for o in outcomes {
        if o.completed_depth <= 0
            || (o.best_move as usize) >= BOARD_AREA
            || o.best_value == VALUE_NONE
        {
            continue;
        }
        let mv_idx = o.best_move as usize;
        votes[mv_idx] +=
            (o.best_value as i64 - min_value as i64 + DEPTH_BIAS) * (o.completed_depth as i64);

        if best.best_value != VALUE_NONE && best.best_value.abs() >= VALUE_MATE_IN_MAX_PLY {
            if o.best_value > best.best_value {
                best = o;
            }
        } else if o.best_value.abs() >= VALUE_MATE_IN_MAX_PLY {
            best = o;
        } else if o.best_value > VALUE_MATED_IN_MAX_PLY
            && votes[mv_idx] > votes[best.best_move as usize]
        {
            best = o;
        }
    }
    best
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeType {
    Root,
    Pv,
    NonPv,
}

struct SearchWorker<'a> {
    tid: usize,
    board: FastBoard,
    evaluator: &'a mut Mix9svqEvaluator,
    main_hist: MainHistory,
    cm_hist: CounterMoveHistory,
    stack: Vec<StackEntry>,
    pv_table: Vec<[u16; MAX_PLY + 2]>,
    root_moves: Vec<RootMove>,
    shared: &'a SharedSearchState<'a>,
    local_nodes: u64,
    total_local_nodes: u64,
    sel_depth: i32,
    root_depth: i32,
    completed_depth: i32,
    root_alpha: i32,
    root_delta: i32,
    singular_root: bool,
}

impl<'a> SearchWorker<'a> {
    fn new(
        tid: usize,
        board: FastBoard,
        evaluator: &'a mut Mix9svqEvaluator,
        root_move_list: &[u16],
        init_eval: i32,
        shared: &'a SharedSearchState<'a>,
    ) -> Self {
        let mut stack = vec![StackEntry::default(); STACK_LEN];
        for i in 0..STACK_LEN {
            stack[i].ply = (i as i32) - (PLY_BEFORE_ROOT as i32);
        }
        let mut se = init_eval;
        for i in (0..=PLY_BEFORE_ROOT).rev() {
            stack[i].static_eval = se;
            se = -se;
        }
        let pv_table = vec![[NONE_MOVE; MAX_PLY + 2]; MAX_PLY + 2];
        let root_moves = root_move_list.iter().map(|&m| RootMove::new(m)).collect();
        Self {
            tid,
            board,
            evaluator,
            main_hist: MainHistory::new(),
            cm_hist: CounterMoveHistory::new(),
            stack,
            pv_table,
            root_moves,
            shared,
            local_nodes: 0,
            total_local_nodes: 0,
            sel_depth: 0,
            root_depth: 0,
            completed_depth: 0,
            root_alpha: -VALUE_INFINITE,
            root_delta: VALUE_INFINITE * 2,
            singular_root: false,
        }
    }

    fn flush_nodes(&mut self) {
        if self.local_nodes > 0 {
            let n = self.local_nodes;
            self.local_nodes = 0;
            self.total_local_nodes += n;
            self.shared.total_nodes.fetch_add(n, Ordering::Relaxed);
        }
    }

    fn into_outcome(mut self) -> ThreadOutcome {
        self.flush_nodes();
        let rm = &self.root_moves[0];
        let best_val = if rm.value != VALUE_NONE {
            rm.value
        } else {
            rm.previous_value
        };
        let pv = if !rm.pv.is_empty() {
            rm.pv.clone()
        } else {
            vec![rm.mv]
        };
        ThreadOutcome {
            best_move: rm.mv,
            best_value: best_val,
            completed_depth: self.completed_depth,
            sel_depth: self.sel_depth.max(rm.sel_depth),
            pv,
        }
    }

    #[inline(always)]
    fn inc_node_and_check_exit(&mut self) -> bool {
        if let Some(max_n) = self.shared.max_nodes {
            if self.shared.num_threads == 1 {
                if self.total_local_nodes + self.local_nodes >= max_n {
                    self.shared.stop.store(true, Ordering::Relaxed);
                    return true;
                }
                self.local_nodes += 1;
                let worker_total = self.total_local_nodes + self.local_nodes;
                if (worker_total & self.shared.time_check_mask) == 0 {
                    self.flush_nodes();
                    if self.tid == 0 {
                        return self.shared.check_exit(0);
                    }
                    return self.shared.is_terminating();
                }
                return false;
            }
            // Multi-threaded `max_nodes`: claim exact node quota atomically so
            // concurrent workers never overshoot `max_nodes`, and check the
            // clock on the worker's monotonic node count.
            if self.shared.is_terminating() {
                return true;
            }
            let prev = self.shared.total_nodes.fetch_add(1, Ordering::Relaxed);
            if prev >= max_n {
                self.shared.total_nodes.fetch_sub(1, Ordering::Relaxed);
                self.shared.stop.store(true, Ordering::Relaxed);
                return true;
            }
            self.total_local_nodes += 1;
            if (self.total_local_nodes & self.shared.time_check_mask) == 0 {
                if self.tid == 0 {
                    return self.shared.check_exit(0);
                }
                return self.shared.is_terminating();
            }
            return false;
        }
        // Fast path for normal timed / fixed-depth search (`max_nodes == None`).
        self.local_nodes += 1;
        if (self.local_nodes & self.shared.time_check_mask) == 0 {
            self.flush_nodes();
            if self.tid == 0 {
                return self.shared.check_exit(0);
            }
            return self.shared.is_terminating();
        }
        false
    }

    #[inline(always)]
    fn make_move<const RENJU: bool>(&mut self, pos: u16) {
        let side_val = if self.board.side_to_move() == FE_BLACK {
            BLACK
        } else {
            WHITE
        };
        let p = pos as usize;
        self.evaluator
            .before_move(side_val, p % BOARD_SIZE, p / BOARD_SIZE);
        self.board.play_typed::<RENJU, true>(p);
    }

    #[inline(always)]
    fn unmake_move<const RENJU: bool>(&mut self, pos: u16) {
        self.board.undo_typed::<RENJU, true>();
        let side_val = if self.board.side_to_move() == FE_BLACK {
            BLACK
        } else {
            WHITE
        };
        let p = pos as usize;
        self.evaluator
            .after_undo(side_val, p % BOARD_SIZE, p / BOARD_SIZE);
    }

    #[inline(always)]
    fn evaluate_static<const RENJU: bool>(&mut self, alpha: i32, beta: i32) -> i32 {
        if let Some(cm) = self.board.classical() {
            if let Some(classical_eval) = self.board.evaluate_classical::<RENJU>() {
                let margin = cm.margin::<RENJU>(classical_eval);
                if classical_eval < alpha - margin || classical_eval > beta + margin {
                    return classical_eval;
                }
            }
        }
        let side_val = if self.board.side_to_move() == FE_BLACK {
            BLACK
        } else {
            WHITE
        };
        self.evaluator
            .evaluate_value(side_val)
            .clamp(VALUE_EVAL_MIN, VALUE_EVAL_MAX)
    }

    #[inline]
    fn update_pv(&mut self, ply: usize, mv: u16) {
        self.pv_table[ply][0] = mv;
        let mut i = 0_usize;
        while i + 1 < MAX_PLY + 1 {
            let child_mv = self.pv_table[ply + 1][i];
            self.pv_table[ply][i + 1] = child_mv;
            if child_mv == NONE_MOVE {
                break;
            }
            i += 1;
        }
    }

    fn sort_root_moves(&mut self) {
        self.root_moves.sort_by(|a, b| {
            b.value
                .cmp(&a.value)
                .then_with(|| b.previous_value.cmp(&a.previous_value))
        });
    }

    fn iterative_deepening(&mut self) {
        let renju = self.board.rule() == RuleSet::Renju;
        let max_depth = self.shared.max_depth;
        let mut first_mate_depth = 0_i32;
        let mut first_singular_depth = 0_i32;

        self.root_depth = 2.min(max_depth);
        while self.root_depth <= max_depth && !self.shared.is_terminating() {
            for rm in &mut self.root_moves {
                rm.previous_value = rm.value;
                rm.previous_pv.clone_from(&rm.pv);
            }

            self.sel_depth = 0;
            let prev_value = self.root_moves[0].previous_value;
            let d = self.root_depth as f32;
            if renju {
                self.aspiration_search::<true>(prev_value, d);
            } else {
                self.aspiration_search::<false>(prev_value, d);
            }

            if !self.shared.is_terminating() {
                self.completed_depth = self.root_depth;
                self.shared.completed_depths[self.tid]
                    .store(self.completed_depth, Ordering::Relaxed);
            }

            if self.tid == 0 {
                let best_val = self.root_moves[0].value;
                let is_mate = best_val != VALUE_NONE && best_val.abs() >= VALUE_MATE_IN_MAX_PLY;
                if is_mate && first_mate_depth == 0 {
                    first_mate_depth = self.root_depth;
                }
                if self.singular_root && first_singular_depth == 0 {
                    first_singular_depth = self.root_depth;
                }
                if is_mate && self.root_depth - first_mate_depth >= NUM_ITERATION_AFTER_MATE {
                    self.shared.stop.store(true, Ordering::Relaxed);
                    break;
                }
                if self.singular_root
                    && self.root_depth - first_singular_depth >= NUM_ITERATION_AFTER_SINGULAR_ROOT
                {
                    self.shared.stop.store(true, Ordering::Relaxed);
                    break;
                }
                if let Some(opt_ms) = self.shared.optimum_ms {
                    if (self.shared.start.elapsed().as_millis() as u64) >= opt_ms {
                        self.shared.stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                if let Some(max_n) = self.shared.max_nodes {
                    if self.shared.total_nodes.load(Ordering::Relaxed) + self.local_nodes >= max_n {
                        self.shared.stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }

            self.root_depth = self.shared.pick_next_depth(self.tid, self.root_depth);
        }
    }

    fn aspiration_search<const RENJU: bool>(&mut self, prev_value: i32, depth: f32) {
        let rule = self.board.rule();
        let (mut alpha, mut beta, mut delta) =
            if depth >= ASPIRATION_DEPTH && prev_value != VALUE_NONE {
                let d = next_aspiration_window_delta(rule, prev_value, 0);
                (
                    (prev_value - d).max(-VALUE_INFINITE),
                    (prev_value + d).min(VALUE_INFINITE),
                    d,
                )
            } else {
                (-VALUE_INFINITE, VALUE_INFINITE, -VALUE_INFINITE)
            };

        let mut fail_high_cnt = 0_i32;

        loop {
            self.root_alpha = alpha;
            let adj_depth = (depth - (fail_high_cnt / 3) as f32).max(1.0);
            let value = self.search::<RENJU, { NodeType::Root as u8 }>(
                PLY_BEFORE_ROOT,
                alpha,
                beta,
                adj_depth,
                false,
            );

            self.sort_root_moves();

            if self.shared.is_terminating() {
                break;
            }

            self.root_moves[0].sel_depth = self.sel_depth;

            if value <= alpha {
                beta = (alpha + beta) / 2;
                alpha = (value - delta).max(-VALUE_INFINITE);
                fail_high_cnt = 0;
            } else if value >= beta {
                beta = (value + delta).min(VALUE_INFINITE);
                fail_high_cnt += 1;
            } else {
                break;
            }

            delta = next_aspiration_window_delta(rule, value, delta);
        }
    }

    fn search<const RENJU: bool, const NT: u8>(
        &mut self,
        ss: usize,
        mut alpha: i32,
        mut beta: i32,
        mut depth: f32,
        cut_node: bool,
    ) -> i32 {
        let pv_node = NT != NodeType::NonPv as u8;
        let root_node = NT == NodeType::Root as u8;

        if self.inc_node_and_check_exit() {
            return VALUE_NONE;
        }

        let self_side = self.board.side_to_move();
        let oppo = self_side ^ 1;
        let oppo5 = self.board.p4_count(oppo, Pattern4::AFive);
        let oppo4 = oppo5 + self.board.p4_count(oppo, Pattern4::BFlex4);

        if depth <= 0.0 {
            return if oppo5 > 0 {
                self.vcfdefend::<RENJU, NT>(ss, alpha, beta, 0.0)
            } else {
                self.vcfsearch::<RENJU, NT>(ss, alpha, beta, 0.0)
            };
        }

        let ply = self.stack[ss].ply;
        if pv_node && self.sel_depth <= ply {
            self.sel_depth = ply + 1;
        }

        if !root_node {
            if self.board.moves_left() == 0 {
                return VALUE_DRAW;
            }
            if ply as usize >= MAX_PLY {
                return self.evaluate_static::<RENJU>(alpha, beta);
            }
            let qw = self.board.quick_win_check::<RENJU>(ply, beta);
            if qw != VALUE_ZERO {
                return qw;
            }

            // Step 3. Mate distance pruning
            alpha = alpha.max(mated_in(ply));
            beta = beta.min(mate_in(ply + 1));
            if alpha >= beta {
                return alpha;
            }

            self.stack[ss + 2].stat_score = 0;
            self.stack[ss + 1].num_null_moves = self.stack[ss].num_null_moves;
        } else {
            self.root_delta = (beta - alpha).max(1);
        }

        // Step 4. TT lookup
        let skip_move = self.stack[ss].skip_move;
        let pos_key = self.board.zobrist_key()
            ^ if skip_move != NONE_MOVE {
                lc_hash(skip_move)
            } else {
                0
            };

        let tt_res = self.shared.tt.probe(pos_key, ply);
        let tt_hit = tt_res.is_some();
        let mut tt_value = tt_res.map_or(VALUE_NONE, |r| r.value);
        let tt_eval = tt_res.map_or(VALUE_NONE, |r| r.eval);
        let tt_is_pv = tt_res.is_some_and(|r| r.is_pv);
        let tt_bound = tt_res.map_or(BOUND_NONE, |r| r.bound);
        let mut tt_move = tt_res.map_or(NONE_MOVE, |r| r.best_move);
        let tt_depth = tt_res.map_or(0, |r| r.depth);

        if root_node && self.completed_depth > 0 {
            tt_move = self.root_moves[0].pv[0];
        }
        if skip_move == NONE_MOVE {
            self.stack[ss].tt_pv = pv_node || (tt_hit && tt_is_pv);
        }
        self.stack[ss + 1].tt_pv = false;

        if !pv_node
            && tt_hit
            && (tt_depth as f32) >= depth
            && (tt_bound
                & if tt_value >= beta {
                    BOUND_LOWER
                } else {
                    BOUND_UPPER
                })
                != 0
        {
            HistoryTracker::update_tt_move_stats(
                &mut self.board,
                &mut self.main_hist,
                &mut self.stack[ss].killers,
                depth,
                tt_move,
                tt_value,
                beta,
            );
            return tt_value;
        }

        // Step 6. Static evaluation
        let mut eval = VALUE_NONE;
        let mut improvement = 0_i32;

        self.stack[ss + 2].killers = [NONE_MOVE; 2];

        let mut skip_to_moves = false;
        if oppo4 > 0 {
            self.stack[ss].static_eval = -self.stack[ss - 1].static_eval;
            if oppo5 > 0 {
                skip_to_moves = true;
            }
        } else if !root_node {
            if tt_hit {
                let mut e = tt_eval;
                if e == VALUE_NONE {
                    e = self.evaluate_static::<RENJU>(alpha, beta);
                }
                self.stack[ss].static_eval = e;
                eval = e;
                if tt_value != VALUE_NONE
                    && (tt_bound
                        & if tt_value > eval {
                            BOUND_LOWER
                        } else {
                            BOUND_UPPER
                        })
                        != 0
                {
                    eval = tt_value;
                }
            } else {
                let e = self.evaluate_static::<RENJU>(alpha, beta);
                self.stack[ss].static_eval = e;
                eval = e;
                if skip_move == NONE_MOVE {
                    self.shared.tt.store(
                        pos_key,
                        VALUE_NONE,
                        e,
                        self.stack[ss].tt_pv,
                        BOUND_NONE,
                        NONE_MOVE,
                        DEPTH_NONE as i32,
                        ply,
                    );
                }
            }
            improvement = self.stack[ss].static_eval - self.stack[ss - 2].static_eval;
        }

        if !skip_to_moves {
            // Step 7. Razoring with VCF (~68 elo, exact Rapfi `search.cpp:1316-1320`)
            if !pv_node
                && (alpha < VALUE_MATE_IN_MAX_PLY || !tt_hit)
                && eval + razor_margin::<RENJU>(depth) < alpha
            {
                return self.vcfsearch::<RENJU, { NodeType::NonPv as u8 }>(
                    ss,
                    alpha,
                    alpha + 1,
                    0.0,
                );
            }

            // Step 8. Reverse futility pruning (~121 elo)
            if !pv_node
                && eval != VALUE_NONE
                && eval < VALUE_MATE_IN_MAX_PLY
                && beta > VALUE_MATED_IN_MAX_PLY
                && eval
                    - futility_margin::<RENJU>(depth - 1.0, cut_node && !tt_hit, improvement > 0)
                    >= beta
                && !(self.stack[ss - 2].move_p4[self_side] >= Pattern4::EBlock4
                    && self.stack[ss - 4].move_p4[self_side] >= Pattern4::EBlock4)
            {
                return eval;
            }

            // Step 9. Null move pruning (~3 elo)
            if !pv_node
                && oppo4 == 0
                && skip_move == NONE_MOVE
                && eval >= beta
                && self.board.last_move() != PASS_MOVE
                && self.stack[ss].static_eval >= beta + null_move_margin::<RENJU>(depth)
            {
                let r = null_move_reduction::<RENJU>(depth);
                self.stack[ss].current_move = PASS_MOVE;
                self.stack[ss + 1].num_null_moves += 1;
                self.shared.tt.prefetch(self.board.zobrist_key_after_pass());
                self.board.play_pass();
                let mut val = -self.search::<RENJU, { NodeType::NonPv as u8 }>(
                    ss + 1,
                    -beta,
                    -beta + 1,
                    depth - r,
                    !cut_node,
                );
                self.board.undo_typed::<RENJU, true>();
                self.stack[ss + 1].num_null_moves -= 1;

                if self.shared.is_terminating() {
                    return VALUE_NONE;
                }

                if val >= beta {
                    if val >= VALUE_MATE_IN_MAX_PLY {
                        val = beta;
                    }
                    if beta.abs() < VALUE_MATE_IN_MAX_PLY {
                        return val;
                    } else {
                        let v = self.search::<RENJU, { NodeType::NonPv as u8 }>(
                            ss,
                            beta - 1,
                            beta,
                            depth - r,
                            false,
                        );
                        if self.shared.is_terminating() {
                            return VALUE_NONE;
                        }
                        if v >= beta {
                            return val;
                        }
                    }
                }
            }

            // Step 10. IIR / IID
            let rs = rule_slot::<RENJU>();
            if !root_node && pv_node && tt_move == NONE_MOVE {
                depth -= IIR_REDUCTION_PV[rs];
            }
            if pv_node && depth > 1.0 && tt_move != NONE_MOVE {
                depth -= ((depth - tt_depth as f32) * IIR_REDUCTION_TT[rs])
                    .clamp(0.0, IIR_REDUCTION_TT_MAX[rs]);
            }
            if depth <= 0.0 {
                return self.vcfsearch::<RENJU, NT>(ss, alpha, beta, 0.0);
            }
            if depth >= IID_DEPTH && tt_move == NONE_MOVE {
                depth -= IIR_REDUCTION[rs];
                let _ = self.search::<RENJU, NT>(
                    ss,
                    alpha,
                    beta,
                    depth - iid_depth_reduction::<RENJU>(depth),
                    cut_node,
                );
                if self.shared.is_terminating() {
                    return VALUE_NONE;
                }
                if let Some(r2) = self.shared.tt.probe(pos_key, ply) {
                    tt_value = r2.value;
                    tt_move = r2.best_move;
                }
            }
        }

        // moves_loop:
        let complexity = position_complexity(&self.board, self_side, oppo);
        let likely_fail_high = !pv_node
            && cut_node
            && eval != VALUE_NONE
            && eval >= beta + fail_high_margin::<RENJU>(depth, oppo4 > 0);

        let mut mp = MovePicker::new_main::<RENJU>(&mut self.board, tt_move);
        let mut hist_tracker = HistoryTracker::new();

        let mut move_count = 0_i32;
        let mut non_mated_count = 0_i32;
        let mut best_value = -VALUE_INFINITE;
        let mut best_move = NONE_MOVE;

        // Step 11. Move loop
        loop {
            let mv = mp.next_move::<RENJU>(
                &mut self.board,
                Some(&mut *self.evaluator),
                Some(&self.main_hist),
                Some(&self.cm_hist),
            );
            if mv == NONE_MOVE {
                break;
            }

            if !root_node && mv == skip_move {
                continue;
            }

            let mut root_rm_idx = 0_usize;
            let mut cur_nodes_before = 0_u64;
            if root_node {
                let Some(idx) = self.root_moves.iter().position(|r| r.mv == mv) else {
                    continue;
                };
                root_rm_idx = idx;
                cur_nodes_before = self.local_nodes;
            }

            move_count += 1;
            self.stack[ss].move_count = move_count;
            let p4_b = self.board.pattern4(mv as usize, 0);
            let p4_w = self.board.pattern4(mv as usize, 1);
            self.stack[ss].move_p4 = [p4_b, p4_w];

            let self_p4 = self.stack[ss].move_p4[self_side];
            let oppo_p4 = self.stack[ss].move_p4[oppo];

            let dist_oppo = chebyshev_distance(mv, self.stack[ss - 1].current_move);
            let dist_self = chebyshev_distance(mv, self.stack[ss - 2].current_move);
            let mt = MoveTraits {
                important: self_p4 >= Pattern4::JFlex2x2
                    || oppo_p4 >= Pattern4::HFlex3
                    || (RENJU && p4_b == Pattern4::Forbid),
                trivial: p4_b == Pattern4::None && p4_w == Pattern4::None,
                distract: dist_self > (if RENJU { 5 } else { 4 }) && dist_oppo > 4,
                dist_self,
                dist_oppo,
            };

            // Step 12. Pruning at shallow depth
            if !root_node && best_value > VALUE_MATED_IN_MAX_PLY {
                if move_count >= futility_move_count(depth, improvement > 0) {
                    continue;
                }
                if mt.trivial && depth < TRIVIAL_PRUN_DEPTH {
                    continue;
                }
                if mp.has_policy_score()
                    && (mp.cur_move_score() as i32) < policy_pruning_score::<RENJU>(depth)
                {
                    continue;
                }
                if oppo4 > 0
                    && depth < TRIVIAL_PRUN_DEPTH
                    && oppo_p4 < Pattern4::EBlock4
                    && mt.distract
                {
                    continue;
                }
            }

            // Step 13. Extensions
            let mut extension = 0.0_f32;
            let rs = rule_slot::<RENJU>();

            if oppo5 > 0 {
                extension = OPPO5_EXT;
            } else if !root_node
                && depth >= SE_DEPTH
                && mv == tt_move
                && skip_move == NONE_MOVE
                && tt_value != VALUE_NONE
                && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
                && (tt_bound & BOUND_LOWER) != 0
                && (tt_depth as f32) >= depth - SE_TTE_DEPTH[rs]
            {
                let former_pv = !pv_node && self.stack[ss].tt_pv;
                let singular_beta =
                    (tt_value - singular_margin::<RENJU>(depth, former_pv)).max(-VALUE_MATE);

                self.stack[ss].skip_move = mv;
                let se_val = self.search::<RENJU, { NodeType::NonPv as u8 }>(
                    ss,
                    singular_beta - 1,
                    singular_beta,
                    depth - singular_reduction(depth, former_pv),
                    cut_node,
                );
                self.stack[ss].skip_move = NONE_MOVE;

                if self.shared.is_terminating() {
                    return VALUE_NONE;
                }

                if se_val < singular_beta {
                    if !pv_node
                        && se_val < singular_beta - double_se_margin::<RENJU>(depth)
                        && self.stack[ss].extra_extension < SE_EXTRA_MAX_DEPTH
                    {
                        extension = 2.0;
                    } else {
                        extension = 1.0;
                    }
                } else if singular_beta >= beta {
                    return beta;
                } else if tt_value >= beta {
                    extension = -SE_REDUCTION_FH;
                }
            } else if mv == tt_move {
                extension = if pv_node {
                    TTM_EXT_PV[rs]
                } else {
                    TTM_EXT_NONPV[rs]
                };
                if self_p4 >= Pattern4::EBlock4 && mt.dist_self <= 6 {
                    extension += if mt.dist_self <= 4 {
                        NEARB4_EXT_DIST4
                    } else {
                        NEARB4_EXT_DIST6
                    };
                }
            }

            if likely_fail_high {
                if self.stack[ss].move_p4[self_side] >= Pattern4::EBlock4 {
                    if self.stack[ss - 2].move_p4[self_side] >= Pattern4::EBlock4 {
                        extension += 1.0;
                    }
                } else {
                    extension -= 1.0;
                }
            }

            let mut new_depth = depth - 1.0 + extension;
            self.stack[ss].current_move = mv;
            self.stack[ss].extra_extension =
                self.stack[ss - 1].extra_extension + (extension - 1.0).max(0.0);

            // Step 14. Prefetch + make move
            self.shared
                .tt
                .prefetch(self.board.zobrist_key_after(mv as usize));
            self.make_move::<RENJU>(mv);

            // Step 15. LMR
            let mut value = -VALUE_INFINITE;
            if depth >= 2.0 && move_count > 1 + (root_node as i32) {
                let r = self.compute_lmr_reduction::<RENJU, NT>(
                    ss,
                    &mp,
                    mv,
                    self_side,
                    depth,
                    move_count,
                    improvement,
                    alpha,
                    beta,
                    best_value,
                    cut_node,
                    oppo4 > 0,
                    complexity,
                    &mt,
                );
                let d = (new_depth - r).min(new_depth + 1.0).max(1.0);
                value = -self.search::<RENJU, { NodeType::NonPv as u8 }>(
                    ss + 1,
                    -(alpha + 1),
                    -alpha,
                    d,
                    true,
                );

                if !self.shared.is_terminating() && value > alpha && d < new_depth {
                    let mut ext = lmr_extension::<RENJU>(new_depth, d, value, alpha, best_value);
                    if self.stack[ss].extra_extension >= LMR_EXTRA_MAX_DEPTH {
                        ext = ext.min(1.0);
                    } else {
                        self.stack[ss].extra_extension += (ext - 1.0).max(0.0);
                    }
                    new_depth += ext;
                    if d < new_depth {
                        value = -self.search::<RENJU, { NodeType::NonPv as u8 }>(
                            ss + 1,
                            -(alpha + 1),
                            -alpha,
                            new_depth,
                            !cut_node,
                        );
                    }
                }
            } else if !pv_node || move_count > 1 {
                // Step 16. Full-depth zero-window search
                value = -self.search::<RENJU, { NodeType::NonPv as u8 }>(
                    ss + 1,
                    -(alpha + 1),
                    -alpha,
                    new_depth,
                    !cut_node,
                );
            }

            // Full-window PV search
            if !self.shared.is_terminating()
                && pv_node
                && (move_count == 1 || (value > alpha && (root_node || value < beta)))
            {
                let child_ply = (ply + 1) as usize;
                if child_ply < self.pv_table.len() {
                    self.pv_table[child_ply][0] = NONE_MOVE;
                }
                value = -self.search::<RENJU, { NodeType::Pv as u8 }>(
                    ss + 1,
                    -beta,
                    -alpha,
                    new_depth,
                    false,
                );
            }

            // Step 17. Unmake move
            self.unmake_move::<RENJU>(mv);

            // Step 18. Check termination & update best move
            if self.shared.is_terminating() {
                return VALUE_NONE;
            }

            if root_node {
                let rm = &mut self.root_moves[root_rm_idx];
                rm.nodes += self.local_nodes.saturating_sub(cur_nodes_before);
                rm.sel_depth = self.sel_depth;
                if move_count == 1 || value > alpha {
                    rm.value = value;
                    rm.pv.clear();
                    rm.pv.push(mv);
                    let child_pv = &self.pv_table[1];
                    for &cmv in child_pv {
                        if cmv == NONE_MOVE {
                            break;
                        }
                        rm.pv.push(cmv);
                    }
                } else {
                    rm.value = VALUE_NONE;
                }
                if value > VALUE_MATED_IN_MAX_PLY {
                    non_mated_count += 1;
                }
            }

            if value > best_value {
                best_value = value;
                if value > alpha {
                    best_move = mv;
                    if pv_node && !root_node {
                        self.update_pv(ply as usize, mv);
                    }
                    if value >= beta {
                        break;
                    } else {
                        alpha = value;
                        if root_node {
                            self.root_alpha = alpha;
                        }
                        if depth > 2.0 && depth < 12.0 && value.abs() < 2000 {
                            depth -= ALPHA_IMPROVEMENT_REDUCTION;
                        }
                    }
                }
            }

            hist_tracker.add_searched_move(
                &self.board,
                mv,
                best_move,
                self.stack[ss].move_p4[self_side],
            );
        }

        if root_node && non_mated_count == 1 && (move_count > 1 || self.root_moves.len() == 1) {
            self.singular_root = true;
        }

        // Step 19. Check for mate / update heuristics
        if move_count == 0 {
            best_value = if skip_move != NONE_MOVE {
                alpha
            } else if oppo5 > 0 {
                mated_in(ply + 2)
            } else {
                mated_in(ply + 4)
            };
            if root_node {
                for rm in &mut self.root_moves {
                    rm.value = best_value;
                }
            }
        } else if best_move != NONE_MOVE {
            hist_tracker.update_bestmove_stats(
                &self.board,
                &mut self.main_hist,
                &mut self.cm_hist,
                &mut self.stack[ss].killers,
                depth,
                best_move,
            );
        }

        // Step 21. Save TT entry
        let bound = if best_value >= beta {
            BOUND_LOWER
        } else if pv_node && best_move != NONE_MOVE {
            BOUND_EXACT
        } else {
            BOUND_UPPER
        };

        if best_value <= alpha {
            self.stack[ss].tt_pv =
                self.stack[ss].tt_pv || (self.stack[ss - 1].tt_pv && depth > 3.0);
        } else if depth > 3.0 {
            self.stack[ss].tt_pv = self.stack[ss].tt_pv && self.stack[ss + 1].tt_pv;
        }

        if skip_move == NONE_MOVE {
            self.shared.tt.store(
                pos_key,
                best_value,
                self.stack[ss].static_eval,
                self.stack[ss].tt_pv,
                bound,
                best_move,
                depth as i32,
                ply,
            );
        }

        best_value
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_lmr_reduction<const RENJU: bool, const NT: u8>(
        &mut self,
        ss: usize,
        mp: &MovePicker,
        mv: u16,
        self_side: usize,
        depth: f32,
        move_count: i32,
        improvement: i32,
        alpha: i32,
        beta: i32,
        best_value: i32,
        cut_node: bool,
        oppo4: bool,
        complexity: f32,
        mt: &MoveTraits,
    ) -> f32 {
        let pv_node = NT != NodeType::NonPv as u8;
        let root_node = NT == NodeType::Root as u8;
        let rs = rule_slot::<RENJU>();
        let lut = &self.shared.reduction_lut[rs];

        let d_idx = (depth as usize).clamp(1, BOARD_AREA + 1);
        let mc_idx = (move_count as usize).clamp(1, BOARD_AREA + 1);
        let base = lut[d_idx] * lut[mc_idx];
        let mut r = if pv_node {
            (base - ((beta - alpha) as f32) / (self.root_delta.max(1) as f32)).max(0.0)
        } else {
            base + if improvement <= 0 && base > 1.0 {
                1.0
            } else {
                0.0
            }
        };

        if mp.has_policy_score() {
            r += policy_reduction::<RENJU>(
                (mp.cur_move_score() as f32) * (0.1 / POLICY_SCORE_SCALE),
            );
        }

        r += complexity * complexity_reduction::<RENJU>(mt.trivial, mt.important, mt.distract);

        if self.stack[ss].tt_pv {
            r -= LMR_TT_PV_SUBTRACT[rs];
        }

        if !root_node && (self.stack[ss].ply & 1) != 0 && best_value >= -self.root_alpha {
            r += LMR_NO_ALPHA_ADD[rs];
        }

        let self_p4 = self.stack[ss].move_p4[self_side];
        let oppo_p4 = self.stack[ss].move_p4[self_side ^ 1];
        if cut_node && !(!oppo4 && self.stack[ss].is_killer(mv) && self_p4 < Pattern4::HFlex3) {
            r += LMR_NO_KILLER_CUT_NODE_ADD[rs];
        }

        if oppo4 && oppo_p4 < Pattern4::EBlock4 {
            if mt.dist_oppo > 4 {
                r += OPPO_USELESS_DEFEND_REDUCTION;
            }
            if mt.dist_self > 4 {
                r += SELF_USELESS_DEFEND_REDUCTION;
            }
        }

        if !oppo4
            && self.stack[ss - 2].move_p4[self_side] >= Pattern4::HFlex3
            && (self_p4 >= Pattern4::HFlex3 || (mt.dist_self <= 4 && self_p4 >= Pattern4::JFlex2x2))
        {
            r -= CONTINUOUS_ATTACK_EXT;
        }

        if RENJU && self.stack[ss].move_p4[0] == Pattern4::Forbid {
            r -= FALSE_FORBID_LESS_REDUCTION;
        }

        let stat = self.main_hist.stat_score(self_side, mv as usize);
        self.stack[ss].stat_score = stat;
        r -= extension_from_stat_score::<RENJU>(stat, depth);

        r
    }

    fn vcfsearch<const RENJU: bool, const NT: u8>(
        &mut self,
        ss: usize,
        mut alpha: i32,
        mut beta: i32,
        depth: f32,
    ) -> i32 {
        let pv_node = NT != NodeType::NonPv as u8;
        if self.inc_node_and_check_exit() {
            return VALUE_NONE;
        }

        let ply = self.stack[ss].ply;
        if pv_node && self.sel_depth <= ply {
            self.sel_depth = ply + 1;
        }

        if self.board.moves_left() == 0 {
            return VALUE_DRAW;
        }
        if ply as usize >= MAX_PLY {
            return self.evaluate_static::<RENJU>(alpha, beta);
        }
        let qw = self.board.quick_win_check::<RENJU>(ply, beta);
        if qw != VALUE_ZERO {
            return qw;
        }

        // Mate distance pruning
        alpha = alpha.max(mated_in(ply));
        beta = beta.min(mate_in(ply + 1));
        if alpha >= beta {
            return alpha;
        }

        let old_alpha = alpha;
        let pos_key = self.board.zobrist_key();
        let tt_res = self.shared.tt.probe(pos_key, ply);
        let tt_hit = tt_res.is_some();
        let tt_value = tt_res.map_or(VALUE_NONE, |r| r.value);
        let tt_eval = tt_res.map_or(VALUE_NONE, |r| r.eval);
        let tt_bound = tt_res.map_or(BOUND_NONE, |r| r.bound);
        let tt_move = tt_res.map_or(NONE_MOVE, |r| r.best_move);
        let tt_depth = tt_res.map_or(DEPTH_LOWER_BOUND, |r| r.depth);

        if tt_hit && (tt_depth as f32) >= depth && (!pv_node || self.tid != 0) {
            if (tt_bound & BOUND_LOWER) != 0 {
                alpha = alpha.max(tt_value);
            }
            if (tt_bound & BOUND_UPPER) != 0 {
                beta = beta.min(tt_value);
            }
            if alpha >= beta {
                return tt_value;
            }
        }

        // Static position evaluation (stand pat)
        let mut best_value = if tt_hit {
            let mut e = tt_eval;
            if e == VALUE_NONE {
                e = self.evaluate_static::<RENJU>(alpha, beta);
            }
            self.stack[ss].static_eval = e;
            if tt_value != VALUE_NONE
                && (tt_bound
                    & if tt_value > e {
                        BOUND_LOWER
                    } else {
                        BOUND_UPPER
                    })
                    != 0
            {
                tt_value
            } else {
                e
            }
        } else {
            let e = if self.stack[ss - 1].current_move == PASS_MOVE {
                -self.stack[ss - 1].static_eval
            } else {
                self.evaluate_static::<RENJU>(alpha, beta)
            };
            self.stack[ss].static_eval = e;
            e
        };

        if best_value >= beta {
            if !tt_hit {
                self.shared.tt.store(
                    pos_key,
                    best_value,
                    self.stack[ss].static_eval,
                    false,
                    BOUND_LOWER,
                    NONE_MOVE,
                    DEPTH_NONE as i32,
                    ply,
                );
            }
            return best_value;
        } else if pv_node && best_value > alpha {
            alpha = best_value;
        }

        // Delta pruning at non-PV node
        if !pv_node && best_value + qvcf_delta_margin::<RENJU>(depth) < alpha {
            if !tt_hit {
                self.shared.tt.store(
                    pos_key,
                    alpha,
                    self.stack[ss].static_eval,
                    false,
                    BOUND_UPPER,
                    NONE_MOVE,
                    DEPTH_NONE as i32,
                    ply,
                );
            }
            return alpha;
        }

        let self_side = self.board.side_to_move();
        let prev_p4 = [
            self.stack[ss - 2].move_p4[self_side],
            self.stack[ss - 4].move_p4[self_side],
        ];
        let mut mp = MovePicker::new_qvcf(&mut self.board, tt_move, depth, prev_p4);
        let mut move_count = 0_i32;
        let mut best_move = NONE_MOVE;

        loop {
            let mv = mp.next_move::<RENJU>(&mut self.board, None, None, None);
            if mv == NONE_MOVE {
                break;
            }
            move_count += 1;
            self.stack[ss].current_move = mv;
            self.stack[ss].move_count = move_count;
            self.stack[ss].move_p4 = [
                self.board.pattern4(mv as usize, 0),
                self.board.pattern4(mv as usize, 1),
            ];
            if pv_node {
                let child_ply = (ply + 1) as usize;
                if child_ply < self.pv_table.len() {
                    self.pv_table[child_ply][0] = NONE_MOVE;
                }
            }

            self.make_move::<RENJU>(mv);
            let value = -self.vcfdefend::<RENJU, NT>(ss + 1, -beta, -alpha, depth - 1.0);
            self.unmake_move::<RENJU>(mv);

            if self.shared.is_terminating() {
                return VALUE_NONE;
            }

            if value > best_value {
                best_value = value;
                if value > alpha {
                    best_move = mv;
                    if pv_node {
                        self.update_pv(ply as usize, mv);
                    }
                    if pv_node && value < beta {
                        alpha = value;
                    } else {
                        break;
                    }
                }
            }
        }

        let bound = if best_value >= beta {
            BOUND_LOWER
        } else if pv_node && best_value > old_alpha {
            BOUND_EXACT
        } else {
            BOUND_UPPER
        };
        self.shared.tt.store(
            pos_key,
            best_value,
            self.stack[ss].static_eval,
            pv_node,
            bound,
            best_move,
            depth.max(DEPTH_QVCF) as i32,
            ply,
        );
        best_value
    }

    fn vcfdefend<const RENJU: bool, const NT: u8>(
        &mut self,
        ss: usize,
        alpha: i32,
        beta: i32,
        depth: f32,
    ) -> i32 {
        let pv_node = NT != NodeType::NonPv as u8;
        if self.inc_node_and_check_exit() {
            return VALUE_NONE;
        }

        let ply = self.stack[ss].ply;
        if pv_node && self.sel_depth <= ply {
            self.sel_depth = ply + 1;
        }

        let self_side = self.board.side_to_move();
        let oppo = self_side ^ 1;
        let oppo5 = self.board.p4_count(oppo, Pattern4::AFive);

        if oppo5 == 0 {
            return self.evaluate_static::<RENJU>(alpha, beta);
        }
        if self.board.moves_left() == 0 {
            return VALUE_DRAW;
        }
        if ply as usize >= MAX_PLY {
            return self.evaluate_static::<RENJU>(alpha, beta);
        }

        let mv = find_p4_pos(&self.board, oppo, Pattern4::AFive);
        if (mv as usize) >= BOARD_AREA {
            return self.evaluate_static::<RENJU>(alpha, beta);
        }

        if RENJU && self_side == FE_BLACK && self.board.check_forbidden_point(mv as usize) {
            return mated_in(ply + 2);
        }

        self.stack[ss].current_move = mv;
        self.stack[ss].move_count = 1;
        self.stack[ss].move_p4 = [
            self.board.pattern4(mv as usize, 0),
            self.board.pattern4(mv as usize, 1),
        ];
        if pv_node {
            let child_ply = (ply + 1) as usize;
            if child_ply < self.pv_table.len() {
                self.pv_table[child_ply][0] = NONE_MOVE;
            }
        }

        self.shared
            .tt
            .prefetch(self.board.zobrist_key_after(mv as usize));
        self.make_move::<RENJU>(mv);
        let value = -self.vcfsearch::<RENJU, NT>(ss + 1, -beta, -alpha, depth);
        self.unmake_move::<RENJU>(mv);

        if self.shared.is_terminating() {
            return VALUE_NONE;
        }
        if pv_node {
            self.update_pv(ply as usize, mv);
        }
        value
    }

    #[allow(dead_code)]
    fn _mate_step_unused(v: i32, ply: i32) -> i32 {
        mate_step(v, ply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{xy_to_move, Board};
    use crate::nnue::load_mix9svq_evaluator_for_rule;
    use std::path::Path;

    #[test]
    fn futility_margin_clamps_negative_to_zero() {
        assert_eq!(futility_margin::<true>(-0.5, false, true), 0);
        assert_eq!(futility_margin::<false>(0.5, true, true), 0);
        assert_eq!(futility_margin::<true>(2.0, false, true), 63);
    }

    #[test]
    fn fast_search_smoke_finds_tactical_win_and_runs_1t_and_4t() {
        let dir = Path::new("opponent/rapfi-networks/mix9svq");
        if !dir.exists() {
            return;
        }
        let mut eval = load_mix9svq_evaluator_for_rule(dir, RuleSet::Renju).expect("load mix9svq");
        let mut legacy = Board::new();
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        // Simple opening position (5 moves)
        for &(x, y) in &[(7, 7), (7, 6), (8, 6), (6, 8), (6, 6)] {
            let m = xy_to_move(x, y).unwrap();
            legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
            fb.play(m as usize);
        }
        eval.sync_with_board(&legacy);

        let searcher = FastSearcher::new(4096);
        let r1 = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 10,
                time_limit_ms: Some(200),
                num_threads: 1,
                ..Default::default()
            },
        );
        assert!((r1.best_move as usize) < BOARD_AREA);
        assert!(r1.depth >= 4, "expected depth >= 4, got {}", r1.depth);
        assert!(r1.nodes > 50);

        // 4T Lazy SMP smoke
        searcher.clear_tt();
        let r4 = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 10,
                time_limit_ms: Some(200),
                num_threads: 4,
                ..Default::default()
            },
        );
        assert!((r4.best_move as usize) < BOARD_AREA);
        assert!(r4.depth >= 4, "expected 4T depth >= 4, got {}", r4.depth);

        // Forced win in 3 (open three -> open four): Black has (5,7),(6,7),(7,7)
        let mut legacy_w = Board::new();
        let mut fb_w = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(5, 7), (0, 0), (6, 7), (0, 2), (7, 7), (0, 4)] {
            let m = xy_to_move(x, y).unwrap();
            legacy_w.play_for_rule(m, None, RuleSet::Renju).unwrap();
            fb_w.play(m as usize);
        }
        eval.sync_with_board(&legacy_w);
        searcher.clear_tt();
        let rw = searcher.search(
            &fb_w,
            &eval,
            &FastSearchConfig {
                max_depth: 8,
                time_limit_ms: Some(200),
                num_threads: 1,
                ..Default::default()
            },
        );
        assert!(
            rw.score >= VALUE_MATE_IN_MAX_PLY,
            "expected forced mate score, got {}",
            rw.score
        );
        let bx = rw.best_move as usize % BOARD_SIZE;
        let by = rw.best_move as usize / BOARD_SIZE;
        assert!(
            (bx == 4 && by == 7) || (bx == 8 && by == 7),
            "expected (4,7) or (8,7) open four, got ({bx},{by})"
        );
    }

    #[test]
    fn compute_time_budget_handles_match_only_and_turn_limits() {
        assert_eq!(compute_time_budget(None, None, 200), (None, None));

        // Match-only tight clock (e.g. time_left=100ms, no timeout_turn) must stay well under 100ms.
        let (opt, max) = compute_time_budget(None, Some(100), 220);
        let opt = opt.expect("match-only optimum budget");
        let max = max.expect("match-only maximum budget");
        assert!(max <= 70, "expected max <= 70ms for 100ms left, got {max}");
        assert!(opt <= max);

        // Ultra-short clocks (e.g. 5ms or 0ms) must NEVER be raised above remaining time,
        // and budgets must be monotonically non-decreasing with given time.
        let (opt5, max5) = compute_time_budget(None, Some(5), 220);
        assert!(
            max5.unwrap() <= 2,
            "expected <= 2ms for 5ms left, got {max5:?}"
        );
        assert!(opt5.unwrap() <= max5.unwrap());
        assert_eq!(compute_time_budget(None, Some(0), 220), (Some(0), Some(0)));
        assert_eq!(compute_time_budget(Some(5), None, 220), (Some(1), Some(2)));
        assert_eq!(compute_time_budget(Some(0), None, 220), (Some(0), Some(0)));

        let mut prev_turn_max = 0_u64;
        let mut prev_left_max = 0_u64;
        for ms in 0..=200_u64 {
            let (to, tm) = compute_time_budget(Some(ms), None, 200);
            let tm = tm.unwrap();
            assert!(to.unwrap() <= tm && tm <= ms);
            assert!(
                tm >= prev_turn_max,
                "turn budget decreased at {ms}ms: {prev_turn_max} -> {tm}"
            );
            prev_turn_max = tm;

            let (lo, lm) = compute_time_budget(None, Some(ms), 200);
            let lm = lm.unwrap();
            assert!(lo.unwrap() <= lm && lm <= ms);
            assert!(
                lm >= prev_left_max,
                "match budget decreased at {ms}ms: {prev_left_max} -> {lm}"
            );
            prev_left_max = lm;
        }

        // Match-only 60s clock paces opening move around ~937ms.
        let (opt60, max60) = compute_time_budget(None, Some(60_000), 220);
        assert!(
            (500..=1500).contains(&max60.unwrap()),
            "unexpected 60s opening max budget: {max60:?}"
        );
        assert!(opt60.unwrap() <= max60.unwrap());

        // Pure per-turn limit keeps original 970ms max / 873ms opt behavior.
        assert_eq!(
            compute_time_budget(Some(1000), None, 200),
            (Some(873), Some(970))
        );
    }

    #[test]
    fn fast_search_respects_small_max_nodes_and_match_only_clock() {
        let dir = Path::new("opponent/rapfi-networks/mix9svq");
        if !dir.exists() {
            return;
        }
        let mut eval = load_mix9svq_evaluator_for_rule(dir, RuleSet::Renju).expect("load mix9svq");
        let mut legacy = Board::new();
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(7, 7), (7, 6), (8, 6), (6, 8), (6, 6)] {
            let m = xy_to_move(x, y).unwrap();
            legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
            fb.play(m as usize);
        }
        eval.sync_with_board(&legacy);

        let searcher = FastSearcher::new(1024);

        // Small max_nodes must stop at or before the limit rather than waiting for 2048 nodes.
        let r_nodes = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 18,
                max_nodes: Some(32),
                time_limit_ms: None,
                time_left_ms: None,
                num_threads: 1,
                ..Default::default()
            },
        );
        assert!((r_nodes.best_move as usize) < BOARD_AREA);
        assert!(
            r_nodes.nodes <= 32,
            "expected <= 32 visited nodes, got {}",
            r_nodes.nodes
        );

        // Match-only time_left=100ms without time_limit_ms must respect the clock.
        searcher.clear_tt();
        let r_time = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 18,
                max_nodes: None,
                time_limit_ms: None,
                time_left_ms: Some(100),
                num_threads: 1,
                ..Default::default()
            },
        );
        assert!((r_time.best_move as usize) < BOARD_AREA);
        assert!(
            r_time.elapsed_ms < 250.0,
            "match-only time_left=100ms should stop quickly, took {:.1}ms",
            r_time.elapsed_ms
        );

        // Multi-threaded 4T exact max_nodes=1 must visit <= 1 node (not 32).
        searcher.clear_tt();
        let r_4t_1 = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 18,
                max_nodes: Some(1),
                time_limit_ms: None,
                time_left_ms: None,
                num_threads: 4,
                ..Default::default()
            },
        );
        assert!((r_4t_1.best_move as usize) < BOARD_AREA);
        assert!(
            r_4t_1.nodes <= 1,
            "4T max_nodes=1 must not overshoot, got {}",
            r_4t_1.nodes
        );

        // Multi-threaded 4T simultaneous huge max_nodes + short time_limit must still honor the clock.
        searcher.clear_tt();
        let r_4t_both = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 30,
                max_nodes: Some(10_000_000),
                time_limit_ms: Some(50),
                time_left_ms: None,
                num_threads: 4,
                ..Default::default()
            },
        );
        assert!((r_4t_both.best_move as usize) < BOARD_AREA);
        assert!(
            r_4t_both.elapsed_ms < 200.0,
            "4T max_nodes + 50ms clock must still fire in-iteration timer, took {:.1}ms",
            r_4t_both.elapsed_ms
        );

        // Ultra-short time_left=5ms must not be lifted to 10ms.
        searcher.clear_tt();
        let r_5ms = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 18,
                max_nodes: None,
                time_limit_ms: None,
                time_left_ms: Some(5),
                num_threads: 1,
                ..Default::default()
            },
        );
        assert!((r_5ms.best_move as usize) < BOARD_AREA);
        assert!(
            r_5ms.elapsed_ms < 5.0,
            "prewarmed 5ms time_left should finish < 5ms, took {:.2}ms",
            r_5ms.elapsed_ms
        );
    }

    #[test]
    #[ignore]
    fn fast_engine_component_timing() {
        let dir = Path::new("opponent/rapfi-networks/mix9svq");
        if !dir.exists() {
            return;
        }
        let mut eval = load_mix9svq_evaluator_for_rule(dir, RuleSet::Renju).expect("load mix9svq");
        let mut legacy = Board::new();
        let mut fb = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[
            (7, 7),
            (7, 6),
            (8, 6),
            (6, 8),
            (6, 6),
            (8, 7),
            (7, 5),
            (7, 8),
            (8, 5),
            (6, 5),
        ] {
            let m = xy_to_move(x, y).unwrap();
            legacy.play_for_rule(m, None, RuleSet::Renju).unwrap();
            fb.play(m as usize);
        }
        eval.sync_with_board(&legacy);
        let _ = eval.evaluate_value(BLACK);
        let _ = eval.evaluate_value(WHITE);

        // 1. Measure evaluator.clone() cost (1x and 4x)
        let t0 = Instant::now();
        for _ in 0..10 {
            let c = eval.clone();
            std::hint::black_box(&c);
        }
        let clone_ms = t0.elapsed().as_secs_f64() * 1000.0 / 10.0;

        // 2. Measure play_move + undo (flushed)
        let n_pm = 20_000;
        let t1 = Instant::now();
        for _ in 0..n_pm {
            eval.before_move(BLACK, 9, 8);
            let v = eval.evaluate_value(WHITE);
            std::hint::black_box(v);
            eval.after_undo(BLACK, 9, 8);
            let _ = eval.evaluate_value(WHITE);
        }
        let pm_val_ns = t1.elapsed().as_nanos() as f64 / n_pm as f64;

        // 3. Measure pure evaluate_value (already flushed)
        let n_ev = 50_000;
        let t2 = Instant::now();
        for _ in 0..n_ev {
            let v = eval.evaluate_value(BLACK);
            std::hint::black_box(v);
        }
        let val_ns = t2.elapsed().as_nanos() as f64 / n_ev as f64;

        // 4. Measure pure evaluate_policy_moves (30 candidates, already flushed)
        let mut cands = Vec::new();
        for pos in 0..BOARD_AREA {
            if fb.is_empty(pos) && cands.len() < 30 {
                cands.push(pos as u16);
            }
        }
        let mut scores = [0.0_f32; 30];
        let n_pol = 20_000;
        let t3 = Instant::now();
        for _ in 0..n_pol {
            eval.evaluate_policy_moves(BLACK, &cands, &mut scores);
            std::hint::black_box(&scores);
        }
        let pol_ns = t3.elapsed().as_nanos() as f64 / n_pol as f64;

        // 5. Measure FastBoard play + undo
        let n_fb = 200_000;
        let t4 = Instant::now();
        let mv_pos = 8 * BOARD_SIZE + 9;
        for _ in 0..n_fb {
            fb.play_typed::<true, true>(mv_pos);
            fb.undo_typed::<true, true>();
        }
        let fb_ns = t4.elapsed().as_nanos() as f64 / n_fb as f64;

        // 6. Measure full 1T and 4T search on mid10 and open4 with exact breakdown
        let searcher = FastSearcher::new(32 * 1024);
        let read_huge_kb = || -> String {
            std::fs::read_to_string("/proc/self/smaps_rollup")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("AnonHugePages:"))
                        .map(|l| l.trim().to_string())
                })
                .unwrap_or_else(|| "AnonHugePages: n/a".to_string())
        };
        let run_bench = |label: &str, board: &FastBoard, ev: &Mix9svqEvaluator, threads: usize| {
            searcher.clear_tt();
            crate::nnue::mix9svq::test_counters::reset();
            crate::fast_engine::board::TEST_FORBID_CALLS.store(0, Ordering::Relaxed);
            let r = searcher.search(
                board,
                ev,
                &FastSearchConfig {
                    max_depth: 99,
                    time_limit_ms: Some(1000),
                    num_threads: threads,
                    ..Default::default()
                },
            );
            let pm = crate::nnue::mix9svq::test_counters::PLAY_MOVE_CALLS.load(Ordering::Relaxed);
            let vc = crate::nnue::mix9svq::test_counters::VALUE_CALLS.load(Ordering::Relaxed);
            let pc = crate::nnue::mix9svq::test_counters::POLICY_CALLS.load(Ordering::Relaxed);
            let pcands = crate::nnue::mix9svq::test_counters::POLICY_CANDS.load(Ordering::Relaxed);
            let fc = crate::fast_engine::board::TEST_FORBID_CALLS.load(Ordering::Relaxed);
            let pm_ms = pm as f64 * (pm_val_ns - 2.0 * val_ns) / 1e6;
            let val_ms = vc as f64 * val_ns / 1e6;
            let pol_ms = pc as f64 * pol_ns / 1e6;
            let fb_ms = r.nodes as f64 * fb_ns / 1e6;
            let avg_cands = if pc > 0 {
                pcands as f64 / pc as f64
            } else {
                0.0
            };
            eprintln!(
                "{label:12}: d{}/{} nodes={} time={:.1}ms nps={:.0} | pm={} (~{:.0}ms) val={} (~{:.0}ms) pol={} avg_c={:.1} (~{:.0}ms) fb(~{:.0}ms) forbid={}",
                r.depth,
                r.sel_depth,
                r.nodes,
                r.elapsed_ms,
                r.nodes as f64 / (r.elapsed_ms / 1000.0),
                pm,
                pm_ms,
                vc,
                val_ms,
                pc,
                avg_cands,
                pol_ms,
                fb_ms,
                fc,
            );
        };

        // Also construct open4 board
        let mut leg4 = Board::new();
        let mut fb4 = FastBoard::new(RuleSet::Renju, CandRange::Square3Line4);
        for &(x, y) in &[(7, 7), (7, 6), (8, 6), (6, 8)] {
            let m = xy_to_move(x, y).unwrap();
            leg4.play_for_rule(m, None, RuleSet::Renju).unwrap();
            fb4.play(m as usize);
        }
        let mut ev4 = eval.clone();
        ev4.sync_with_board(&leg4);
        let _ = ev4.evaluate_value(BLACK);
        let _ = ev4.evaluate_value(WHITE);

        // Fixed d15 1T deterministic benchmark on mid10 & open4
        searcher.clear_tt();
        let d15_m = searcher.search(
            &fb,
            &eval,
            &FastSearchConfig {
                max_depth: 15,
                time_limit_ms: None,
                num_threads: 1,
                ..Default::default()
            },
        );
        searcher.clear_tt();
        let d15_o = searcher.search(
            &fb4,
            &ev4,
            &FastSearchConfig {
                max_depth: 15,
                time_limit_ms: None,
                num_threads: 1,
                ..Default::default()
            },
        );

        eprintln!(
            "=== COMPONENT TIMING ({}) ===\n\
             eval.clone()       : {clone_ms:.2} ms (4x = {:.2} ms)\n\
             eval_value (pure)  : {val_ns:.0} ns\n\
             play_move+val+undo : {pm_val_ns:.0} ns (implied play_move ~ {:.0} ns)\n\
             eval_policy (30mv) : {pol_ns:.0} ns\n\
             FastBoard play+undo: {fb_ns:.0} ns\n\
             fixed d15 mid10    : nodes={} time={:.2}ms mv={}\n\
             fixed d15 open4    : nodes={} time={:.2}ms mv={}",
            read_huge_kb(),
            clone_ms * 4.0,
            pm_val_ns - 2.0 * val_ns,
            d15_m.nodes,
            d15_m.elapsed_ms,
            d15_m.best_move,
            d15_o.nodes,
            d15_o.elapsed_ms,
            d15_o.best_move,
        );
        run_bench("mid10 1T 1s", &fb, &eval, 1);
        run_bench("mid10 4T 1s", &fb, &eval, 4);
        run_bench("open4 1T 1s", &fb4, &ev4, 1);
        run_bench("open4 4T 1s", &fb4, &ev4, 4);

        // Freestyle component timing comparison
        let mut ev_fs =
            load_mix9svq_evaluator_for_rule(dir, RuleSet::Freestyle).expect("load fs mix9svq");
        let fs_cases: &[(&str, &[(usize, usize)])] = &[
            (
                "FS case36",
                &[(7, 7), (8, 6), (4, 6), (6, 7), (10, 8), (7, 5), (6, 6)],
            ),
            (
                "FS mid10",
                &[
                    (7, 7),
                    (7, 6),
                    (8, 6),
                    (6, 8),
                    (6, 6),
                    (8, 8),
                    (5, 7),
                    (9, 7),
                    (7, 8),
                    (6, 7),
                ],
            ),
            ("FS open6", &[(7, 7), (5, 4), (9, 6), (6, 6)]),
        ];
        for &(name, mvs) in fs_cases {
            let mut l = Board::new();
            let mut f = FastBoard::new(RuleSet::Freestyle, CandRange::Square3Line4);
            for &(x, y) in mvs {
                let m = xy_to_move(x, y).unwrap();
                l.play_for_rule(m, None, RuleSet::Freestyle).unwrap();
                f.play(m as usize);
            }
            ev_fs.sync_with_board(&l);
            let _ = ev_fs.evaluate_value(BLACK);
            let _ = ev_fs.evaluate_value(WHITE);
            run_bench(&format!("{name} 1T"), &f, &ev_fs, 1);
            run_bench(&format!("{name} 4T"), &f, &ev_fs, 4);
        }
    }

    #[test]
    fn fast_search_nearb4_dist_and_post_se_move_p4_parity() {
        let dir = Path::new("opponent/rapfi-networks/mix9svq");
        let cm = Path::new("opponent/rapfi-networks/classical/model210901.bin");
        if !dir.exists() || !cm.exists() {
            return;
        }
        let check = |rule: RuleSet,
                     moves: &[(usize, usize)],
                     max_depth: i32,
                     exp_mv: (usize, usize),
                     exp_score: i32,
                     exp_sel: i32,
                     exp_nodes: u64| {
            let mut eval = load_mix9svq_evaluator_for_rule(dir, rule).expect("load mix9svq");
            let mut leg = Board::new();
            let mut fb = FastBoard::new(rule, CandRange::Square3Line4);
            for &(x, y) in moves {
                let m = xy_to_move(x, y).unwrap();
                leg.play_for_rule(m, None, rule).unwrap();
                fb.play(m as usize);
            }
            eval.sync_with_board(&leg);
            let searcher = FastSearcher::new(32 * 1024);
            let r = searcher.search(
                &fb,
                &eval,
                &FastSearchConfig {
                    max_depth,
                    time_limit_ms: None,
                    num_threads: 1,
                    ..Default::default()
                },
            );
            assert_eq!(
                (
                    r.best_move as usize % BOARD_SIZE,
                    r.best_move as usize / BOARD_SIZE
                ),
                exp_mv,
                "best_move mismatch"
            );
            assert_eq!(r.score, exp_score, "score mismatch");
            assert_eq!(r.sel_depth, exp_sel, "sel_depth mismatch");
            assert_eq!(r.nodes, exp_nodes, "nodes mismatch");
        };

        // Regression for post-SE `stack[ss].move_p4[self_side]` in add_searched_move / likely_fail_high.
        check(
            RuleSet::Freestyle,
            &[
                (7, 7),
                (8, 6),
                (4, 6),
                (6, 7),
                (10, 8),
                (7, 5),
                (6, 6),
                (8, 4),
            ],
            10,
            (8, 8),
            826,
            17,
            3052,
        );
        // Regression for `mt.dist_self <= 6` (without `>= 0`) on ply 0/1 near-B4 TT extensions.
        check(
            RuleSet::Freestyle,
            &[(7, 7), (8, 6), (4, 6), (6, 7), (10, 8), (7, 5), (6, 6)],
            10,
            (5, 5),
            -62,
            17,
            3319,
        );
        // Verify Renju deterministic parity is preserved.
        check(
            RuleSet::Renju,
            &[
                (7, 7),
                (7, 6),
                (8, 6),
                (6, 8),
                (6, 6),
                (8, 8),
                (5, 7),
                (9, 7),
                (7, 8),
                (6, 7),
            ],
            12,
            (5, 8),
            57,
            13,
            5459,
        );
    }
}
