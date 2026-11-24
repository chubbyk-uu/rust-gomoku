//! Lock-free ABDADA shared-TT coordination primitives for multi-threaded search.

use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread;
use std::time::Instant;

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::{INF, WIN};
use crate::eval::EvalCaches;
use crate::search::alphabeta::{AlphaBetaSearcher, SearchOptions, SearchStats};
use crate::search::tt::TranspositionTable;
use crate::types::{Move, Side};

/// Minimum remaining depth at which ABDADA tracks in-flight subtrees.
/// Keeping shallow nodes (`depth < 3.0`) out of the busy table avoids atomic
/// contention on 95%+ of the search tree.
pub(crate) const ABDADA_MIN_DEPTH: f64 = 3.0;

/// Minimum iterative-deepening depth at which worker threads are woken.
/// Keeps shallow depths (1..=4) zero-overhead on fast moves.
pub(crate) const ABDADA_PARALLEL_START_DEPTH: i32 = 5;

const ABDADA_TABLE_BITS: usize = 16;
const ABDADA_TABLE_SIZE: usize = 1 << ABDADA_TABLE_BITS;
const ABDADA_TABLE_MASK: usize = ABDADA_TABLE_SIZE - 1;

/// Fixed-size lock-free hash table tracking subtrees currently being searched.
///
/// Entries store `(zobrist_key ^ salt) | 1` so switching to a new depth or move
/// only bumps `salt` without clearing 512 KiB of memory.
#[derive(Debug)]
pub(crate) struct AbdadaTable {
    slots: Box<[AtomicU64]>,
}

impl Default for AbdadaTable {
    fn default() -> Self {
        Self::new()
    }
}

impl AbdadaTable {
    pub(crate) fn new() -> Self {
        let slots = (0..ABDADA_TABLE_SIZE)
            .map(|_| AtomicU64::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { slots }
    }

    #[inline(always)]
    fn slot_index(key: u64) -> usize {
        // Mix upper bits into the low index to reduce clustering.
        ((key ^ (key >> 16)) as usize) & ABDADA_TABLE_MASK
    }

    #[inline(always)]
    fn encode_tag(key: u64, salt: u64) -> u64 {
        (key ^ salt) | 1
    }

    #[inline(always)]
    pub(crate) fn is_busy(&self, key: u64, salt: u64) -> bool {
        self.slots[Self::slot_index(key)].load(Ordering::Relaxed) == Self::encode_tag(key, salt)
    }

    #[inline(always)]
    pub(crate) fn mark_busy(&self, key: u64, salt: u64) {
        self.slots[Self::slot_index(key)].store(Self::encode_tag(key, salt), Ordering::Relaxed);
    }

    #[inline(always)]
    pub(crate) fn clear_busy(&self, key: u64, salt: u64) {
        let idx = Self::slot_index(key);
        let tag = Self::encode_tag(key, salt);
        if self.slots[idx].load(Ordering::Relaxed) == tag {
            self.slots[idx].store(0, Ordering::Relaxed);
        }
    }
}

/// Shared best root `(alpha, move)` across threads working on the same depth.
/// Only strictly improved exact scores (`score > alpha`) may be published —
/// never zero-window fail-low bounds (`score == alpha`).
#[derive(Debug)]
pub(crate) struct SharedRootBest {
    alpha: AtomicI32,
    best: Mutex<(i32, Option<Move>)>,
}

impl SharedRootBest {
    pub(crate) fn new() -> Self {
        Self {
            alpha: AtomicI32::new(-INF),
            best: Mutex::new((-INF, None)),
        }
    }

    pub(crate) fn load_if_better(&self, current_alpha: i32) -> Option<(i32, Move)> {
        if self.alpha.load(Ordering::Relaxed) <= current_alpha {
            return None;
        }
        let guard = self.best.lock().expect("shared root best lock");
        if guard.0 > current_alpha {
            guard.1.map(|mv| (guard.0, mv))
        } else {
            None
        }
    }

    pub(crate) fn publish(&self, score: i32, move_: Move) {
        if score <= self.alpha.load(Ordering::Relaxed) {
            return;
        }
        let mut guard = self.best.lock().expect("shared root best lock");
        if score > guard.0 {
            guard.0 = score;
            guard.1 = Some(move_);
            self.alpha.store(score, Ordering::Relaxed);
        }
    }
}

/// Per-depth shared coordination context passed to each participating thread.
#[derive(Clone, Debug)]
pub(crate) struct AbdadaContext {
    pub worker_id: usize,
    pub table: Arc<AbdadaTable>,
    pub salt: u64,
    pub root_best: Arc<SharedRootBest>,
    pub depth_done: Arc<AtomicBool>,
}

impl AbdadaContext {
    #[inline(always)]
    pub(crate) fn is_stopped(&self) -> bool {
        self.depth_done.load(Ordering::Relaxed)
    }
}

pub(crate) struct DepthJob {
    pub config: Arc<EngineConfig>,
    pub tt: TranspositionTable,
    pub tt_generation: u16,
    pub board: Board,
    pub caches: EvalCaches,
    pub side: Side,
    pub depth: i32,
    pub wide: usize,
    pub root_allowed_moves: Option<HashSet<Move>>,
    pub root_preferred_move: Option<Move>,
    pub deadline: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub depth_done: Arc<AtomicBool>,
    pub abdada_table: Arc<AbdadaTable>,
    pub abdada_salt: u64,
    pub root_best: Arc<SharedRootBest>,
    pub completed_result: Mutex<Option<(i32, Option<Move>)>>,
    pub helper_nodes: AtomicUsize,
    pub helper_tt_curr: AtomicUsize,
    pub helper_tt_old: AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ThreadSearchOutcome {
    pub completed_depth: i32,
    pub score: i32,
    pub best_move: Option<Move>,
}

/// Rapfi-style asynchronous Lazy SMP job shared across all threads for a full
/// move search (no per-depth global barriers).
pub(crate) struct LazySmpJob {
    pub config: Arc<EngineConfig>,
    pub tt: TranspositionTable,
    pub tt_generation: u16,
    pub board: Board,
    pub caches: EvalCaches,
    pub side: Side,
    pub max_depth: i32,
    pub wide: usize,
    pub root_allowed_moves: Option<HashSet<Move>>,
    pub deadline: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub stop_flag: Arc<AtomicBool>,
    pub abdada_table: Arc<AbdadaTable>,
    pub abdada_salt: u64,
    pub completed_depths: Vec<AtomicI32>,
    pub thread_outcomes: Mutex<Vec<ThreadSearchOutcome>>,
    pub helper_nodes: AtomicUsize,
    pub helper_tt_curr: AtomicUsize,
    pub helper_tt_old: AtomicUsize,
}

/// Rapfi `ABSearcher::pickNextDepth` (search.cpp:515-536):
/// Main thread always advances `last_depth + 1`; helper threads skip a depth
/// when more than half of all threads are already at or above it.
pub(crate) fn pick_next_depth(
    completed_depths: &[AtomicI32],
    tid: usize,
    last_depth: i32,
    max_depth: i32,
) -> i32 {
    let n = completed_depths.len();
    if tid == 0 || n < 3 {
        return (last_depth + 1).min(max_depth + 1);
    }
    let mut next_depth = last_depth + 1;
    while next_depth <= max_depth {
        let mut above = 0_usize;
        for (i, d) in completed_depths.iter().enumerate() {
            if i != tid && d.load(Ordering::Relaxed) + 1 >= next_depth {
                above += 1;
            }
        }
        if above > n / 2 {
            next_depth += 1;
        } else {
            return next_depth;
        }
    }
    max_depth + 1
}

/// Rapfi `ABSearcher::pickBestThread` (search.cpp:538-580):
/// Depth-weighted voting across completed thread outcomes, with proven mates
/// taking priority (and shorter winning mates winning over longer ones).
pub(crate) fn pick_best_thread(outcomes: &[ThreadSearchOutcome]) -> Option<ThreadSearchOutcome> {
    let valid: Vec<ThreadSearchOutcome> = outcomes
        .iter()
        .copied()
        .filter(|o| o.completed_depth > 0 && o.best_move.is_some())
        .collect();
    if valid.is_empty() {
        return None;
    }
    let mut best = valid[0];
    let min_score = valid.iter().map(|o| o.score).min().unwrap_or(0);
    let vote_for = |mv: Move| -> i64 {
        let mut total = 0_i64;
        for o in &valid {
            if o.best_move == Some(mv) {
                total +=
                    (i64::from(o.score) - i64::from(min_score) + 15) * i64::from(o.completed_depth);
            }
        }
        total
    };
    for &cand in &valid[1..] {
        if best.score.abs() >= WIN {
            if cand.score > best.score {
                best = cand;
            }
        } else if cand.score.abs() >= WIN {
            best = cand;
        } else if cand.score > -WIN
            && vote_for(cand.best_move.expect("checked"))
                > vote_for(best.best_move.expect("checked"))
        {
            best = cand;
        }
    }
    Some(best)
}

enum PoolJob {
    Abdada(Arc<DepthJob>),
    LazySmp(Arc<LazySmpJob>),
}

struct PoolState {
    generation: u64,
    shutdown: bool,
    active_workers: usize,
    job: Option<PoolJob>,
}

struct PoolShared {
    state: Mutex<PoolState>,
    wake_workers: Condvar,
    workers_done: Condvar,
}

/// Persistent worker pool owned by `RootSearcher` when `threads > 1`.
/// Eliminates per-move `thread::spawn` overhead while keeping single-threaded
/// searches completely free of worker threads.
///
/// Worker spawning is best-effort: if the OS refuses to create some or all
/// worker threads (e.g. under tight memory/ulimit constraints), the pool
/// degrades gracefully to however many workers started (including 0, which
/// falls back to single-threaded search) instead of panicking.
pub(crate) struct SearchWorkerPool {
    requested_workers: usize,
    num_workers: usize,
    shared: Arc<PoolShared>,
    handles: Vec<thread::JoinHandle<()>>,
    abdada_table: Option<Arc<AbdadaTable>>,
    salt_counter: u64,
}

impl std::fmt::Debug for SearchWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchWorkerPool")
            .field("requested_workers", &self.requested_workers)
            .field("num_workers", &self.num_workers)
            .finish()
    }
}

fn lock_pool_state(shared: &PoolShared) -> std::sync::MutexGuard<'_, PoolState> {
    shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_condvar<'a>(
    condvar: &Condvar,
    guard: std::sync::MutexGuard<'a, PoolState>,
) -> std::sync::MutexGuard<'a, PoolState> {
    condvar
        .wait(guard)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ActiveWorkerGuard {
    shared: Arc<PoolShared>,
}

impl Drop for ActiveWorkerGuard {
    fn drop(&mut self) {
        let mut state = lock_pool_state(&self.shared);
        state.active_workers = state.active_workers.saturating_sub(1);
        if state.active_workers == 0 {
            self.shared.workers_done.notify_one();
        }
    }
}

impl SearchWorkerPool {
    pub(crate) fn new(requested_workers: usize) -> Self {
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState {
                generation: 0,
                shutdown: false,
                active_workers: 0,
                job: None,
            }),
            wake_workers: Condvar::new(),
            workers_done: Condvar::new(),
        });
        const WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;
        let mut handles = Vec::with_capacity(requested_workers);
        let mut spawn_failed = false;
        for worker_idx in 0..requested_workers {
            let worker_id = worker_idx + 1;
            let shared_ref = Arc::clone(&shared);
            match thread::Builder::new()
                .name(format!("gomoku-abdada-{worker_id}"))
                .stack_size(WORKER_STACK_BYTES)
                .spawn(move || worker_loop(shared_ref, worker_id))
            {
                Ok(handle) => handles.push(handle),
                Err(_) => {
                    spawn_failed = true;
                    break;
                }
            }
        }
        if spawn_failed && !handles.is_empty() {
            // Reclaim partially spawned worker stacks immediately so a tight
            // virtual-memory limit (e.g. ulimit -v) still leaves headroom for
            // single-threaded search.
            {
                let mut state = lock_pool_state(&shared);
                state.shutdown = true;
                shared.wake_workers.notify_all();
            }
            for handle in handles.drain(..) {
                let _ = handle.join();
            }
            lock_pool_state(&shared).shutdown = false;
        }
        let num_workers = handles.len();
        let abdada_table = (num_workers > 0).then(|| Arc::new(AbdadaTable::new()));
        Self {
            requested_workers,
            num_workers,
            shared,
            handles,
            abdada_table,
            salt_counter: 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub(crate) fn requested_workers(&self) -> usize {
        self.requested_workers
    }

    pub(crate) fn num_workers(&self) -> usize {
        self.num_workers
    }

    pub(crate) fn next_salt(&mut self) -> u64 {
        self.salt_counter = self.salt_counter.wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.salt_counter
    }

    pub(crate) fn table(&self) -> Option<Arc<AbdadaTable>> {
        self.abdada_table.as_ref().map(Arc::clone)
    }

    pub(crate) fn start_depth(&self, job: Arc<DepthJob>) {
        if self.num_workers == 0 {
            return;
        }
        let mut state = lock_pool_state(&self.shared);
        state.job = Some(PoolJob::Abdada(job));
        state.active_workers = self.num_workers;
        state.generation = state.generation.wrapping_add(1);
        self.shared.wake_workers.notify_all();
    }

    pub(crate) fn start_lazy_smp(&self, job: Arc<LazySmpJob>) {
        if self.num_workers == 0 {
            return;
        }
        let mut state = lock_pool_state(&self.shared);
        state.job = Some(PoolJob::LazySmp(job));
        state.active_workers = self.num_workers;
        state.generation = state.generation.wrapping_add(1);
        self.shared.wake_workers.notify_all();
    }

    pub(crate) fn wait_depth_done(&self) {
        if self.num_workers == 0 {
            return;
        }
        let mut state = lock_pool_state(&self.shared);
        while state.active_workers > 0 {
            state = wait_condvar(&self.shared.workers_done, state);
        }
        state.job = None;
    }
}

impl Drop for SearchWorkerPool {
    fn drop(&mut self) {
        {
            let mut state = lock_pool_state(&self.shared);
            state.shutdown = true;
            state.job = None;
            self.shared.wake_workers.notify_all();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

struct WorkerLocal {
    caches: EvalCaches,
    searcher: Option<AlphaBetaSearcher>,
}

impl WorkerLocal {
    fn new() -> Self {
        Self {
            caches: EvalCaches::new(),
            searcher: None,
        }
    }
}

fn run_depth_job(job: &DepthJob, worker_id: usize, local: &mut WorkerLocal) {
    let mut board = job.board.clone();
    local.caches.restore_from(&job.caches);
    let searcher = match local.searcher.as_mut() {
        Some(existing) => {
            existing.sync_worker_state(&job.config, &job.tt, job.tt_generation);
            existing
        }
        None => {
            let mut created = AlphaBetaSearcher::with_tt((*job.config).clone(), job.tt.clone());
            created.set_tt_generation(job.tt_generation);
            local.searcher.insert(created)
        }
    };
    let mut stats = SearchStats {
        deadline: job.deadline,
        cancel: job.cancel.clone(),
        ..SearchStats::default()
    };
    let abdada_ctx = AbdadaContext {
        worker_id,
        table: Arc::clone(&job.abdada_table),
        salt: job.abdada_salt,
        root_best: Arc::clone(&job.root_best),
        depth_done: Arc::clone(&job.depth_done),
    };
    let (score, move_) = searcher.search_with_abdada(
        &mut board,
        &mut local.caches,
        job.side,
        f64::from(job.depth),
        -INF,
        INF,
        job.wide,
        &mut stats,
        SearchOptions {
            opo: 1,
            root: true,
            root_allowed_moves: job.root_allowed_moves.as_ref(),
            root_preferred_move: job.root_preferred_move,
            ..SearchOptions::default()
        },
        Some(&abdada_ctx),
    );
    job.helper_nodes.fetch_add(stats.nodes, Ordering::Relaxed);
    job.helper_tt_curr
        .fetch_add(stats.tt_bestmove_current_generation, Ordering::Relaxed);
    job.helper_tt_old
        .fetch_add(stats.tt_bestmove_old_generation, Ordering::Relaxed);
    if !stats.stop {
        let mut guard = job
            .completed_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some((score, move_));
        }
        job.depth_done.store(true, Ordering::Relaxed);
    }
}

fn run_lazy_smp_helper(job: &LazySmpJob, worker_id: usize, local: &mut WorkerLocal) {
    let mut board = job.board.clone();
    local.caches.restore_from(&job.caches);
    let searcher = match local.searcher.as_mut() {
        Some(existing) => {
            existing.sync_worker_state(&job.config, &job.tt, job.tt_generation);
            existing
        }
        None => {
            let mut created = AlphaBetaSearcher::with_tt((*job.config).clone(), job.tt.clone());
            created.set_tt_generation(job.tt_generation);
            local.searcher.insert(created)
        }
    };
    let mut completed_depth = 0_i32;
    let mut best_move: Option<Move> = None;
    let mut best_score = -INF;
    let mut last_depth = 0_i32;
    let mut total_nodes = 0_usize;
    let mut total_tt_curr = 0_usize;
    let mut total_tt_old = 0_usize;

    loop {
        if job.stop_flag.load(Ordering::Relaxed)
            || job
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
            || job.deadline.is_some_and(|dl| Instant::now() >= dl)
        {
            break;
        }
        let next_depth =
            pick_next_depth(&job.completed_depths, worker_id, last_depth, job.max_depth);
        if next_depth > job.max_depth {
            break;
        }
        let abdada_ctx = AbdadaContext {
            worker_id,
            table: Arc::clone(&job.abdada_table),
            salt: job.abdada_salt,
            root_best: Arc::new(SharedRootBest::new()),
            depth_done: Arc::clone(&job.stop_flag),
        };
        let mut stats = SearchStats {
            deadline: job.deadline,
            cancel: job.cancel.clone(),
            ..SearchStats::default()
        };
        let root_preferred_move = if job.root_allowed_moves.is_some() {
            best_move
        } else {
            None
        };
        let (score, mv) = searcher.search_with_abdada(
            &mut board,
            &mut local.caches,
            job.side,
            f64::from(next_depth),
            -INF,
            INF,
            job.wide,
            &mut stats,
            SearchOptions {
                opo: 1,
                root: true,
                root_allowed_moves: job.root_allowed_moves.as_ref(),
                root_preferred_move,
                ..SearchOptions::default()
            },
            Some(&abdada_ctx),
        );
        total_nodes += stats.nodes;
        total_tt_curr += stats.tt_bestmove_current_generation;
        total_tt_old += stats.tt_bestmove_old_generation;
        if stats.stop {
            break;
        }
        last_depth = next_depth;
        completed_depth = next_depth;
        if let Some(m) = mv {
            best_move = Some(m);
            best_score = score;
        }
        if worker_id < job.completed_depths.len() {
            job.completed_depths[worker_id].store(completed_depth, Ordering::Relaxed);
        }
        if score >= WIN || score <= -WIN {
            break;
        }
    }

    job.helper_nodes.fetch_add(total_nodes, Ordering::Relaxed);
    job.helper_tt_curr
        .fetch_add(total_tt_curr, Ordering::Relaxed);
    job.helper_tt_old.fetch_add(total_tt_old, Ordering::Relaxed);
    if completed_depth > 0 && best_move.is_some() {
        let mut guard = job
            .thread_outcomes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.push(ThreadSearchOutcome {
            completed_depth,
            score: best_score,
            best_move,
        });
    }
}

fn worker_loop(shared: Arc<PoolShared>, worker_id: usize) {
    let mut seen_generation = 0_u64;
    let mut local = WorkerLocal::new();
    loop {
        let job = {
            let mut state = lock_pool_state(&shared);
            while !state.shutdown && state.generation == seen_generation {
                state = wait_condvar(&shared.wake_workers, state);
            }
            if state.shutdown {
                return;
            }
            seen_generation = state.generation;
            match state.job.as_ref() {
                Some(PoolJob::Abdada(j)) => Some(PoolJob::Abdada(Arc::clone(j))),
                Some(PoolJob::LazySmp(j)) => Some(PoolJob::LazySmp(Arc::clone(j))),
                None => None,
            }
        };

        // Guard guarantees `active_workers` is decremented and `workers_done` is
        // notified even if a job unwinds, while `catch_unwind` keeps the worker
        // alive for subsequent depths so `num_workers` stays accurate.
        let _active_guard = ActiveWorkerGuard {
            shared: Arc::clone(&shared),
        };
        if let Some(job) = job {
            let _ = catch_unwind(AssertUnwindSafe(|| match job {
                PoolJob::Abdada(j) => run_depth_job(&j, worker_id, &mut local),
                PoolJob::LazySmp(j) => run_lazy_smp_helper(&j, worker_id, &mut local),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_worker_pool_degrades_without_allocating_table_or_hanging() {
        let pool = SearchWorkerPool::new(0);
        assert_eq!(pool.requested_workers(), 0);
        assert_eq!(pool.num_workers(), 0);
        assert!(pool.table().is_none());
        pool.wait_depth_done();
    }

    #[test]
    fn active_worker_guard_releases_waiter_even_on_panic() {
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState {
                generation: 1,
                shutdown: false,
                active_workers: 1,
                job: None,
            }),
            wake_workers: Condvar::new(),
            workers_done: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || {
            let _guard = ActiveWorkerGuard {
                shared: worker_shared,
            };
            let _ = catch_unwind(AssertUnwindSafe(|| {
                panic!("simulated helper search panic");
            }));
        });

        let mut state = lock_pool_state(&shared);
        while state.active_workers > 0 {
            state = wait_condvar(&shared.workers_done, state);
        }
        assert_eq!(state.active_workers, 0);
        drop(state);
        worker.join().unwrap();
    }
}
