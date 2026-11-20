//! Lock-free ABDADA shared-TT coordination primitives for multi-threaded search.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread;
use std::time::Instant;

use crate::board::Board;
use crate::config::EngineConfig;
use crate::constants::INF;
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
    pub config: EngineConfig,
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

struct PoolState {
    generation: u64,
    shutdown: bool,
    active_workers: usize,
    job: Option<Arc<DepthJob>>,
}

struct PoolShared {
    state: Mutex<PoolState>,
    wake_workers: Condvar,
    workers_done: Condvar,
}

/// Persistent worker pool owned by `RootSearcher` when `threads > 1`.
/// Eliminates per-move `thread::spawn` overhead while keeping single-threaded
/// searches completely free of worker threads.
pub(crate) struct SearchWorkerPool {
    num_workers: usize,
    shared: Arc<PoolShared>,
    handles: Vec<thread::JoinHandle<()>>,
    abdada_table: Arc<AbdadaTable>,
    salt_counter: u64,
}

impl std::fmt::Debug for SearchWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchWorkerPool")
            .field("num_workers", &self.num_workers)
            .finish()
    }
}

impl SearchWorkerPool {
    pub(crate) fn new(num_workers: usize) -> Self {
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
        let mut handles = Vec::with_capacity(num_workers);
        for worker_idx in 0..num_workers {
            let worker_id = worker_idx + 1;
            let shared_ref = Arc::clone(&shared);
            let handle = thread::Builder::new()
                .name(format!("gomoku-abdada-{worker_id}"))
                .spawn(move || worker_loop(shared_ref))
                .expect("spawn ABDADA worker thread");
            handles.push(handle);
        }
        Self {
            num_workers,
            shared,
            handles,
            abdada_table: Arc::new(AbdadaTable::new()),
            salt_counter: 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub(crate) fn num_workers(&self) -> usize {
        self.num_workers
    }

    pub(crate) fn next_salt(&mut self) -> u64 {
        self.salt_counter = self.salt_counter.wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.salt_counter
    }

    pub(crate) fn table(&self) -> Arc<AbdadaTable> {
        Arc::clone(&self.abdada_table)
    }

    pub(crate) fn start_depth(&self, job: Arc<DepthJob>) {
        let mut state = self.shared.state.lock().expect("pool state lock");
        state.job = Some(job);
        state.active_workers = self.num_workers;
        state.generation = state.generation.wrapping_add(1);
        self.shared.wake_workers.notify_all();
    }

    pub(crate) fn wait_depth_done(&self) {
        let mut state = self.shared.state.lock().expect("pool state lock");
        while state.active_workers > 0 {
            state = self
                .shared
                .workers_done
                .wait(state)
                .expect("pool wait lock");
        }
        state.job = None;
    }
}

impl Drop for SearchWorkerPool {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().expect("pool drop lock");
            state.shutdown = true;
            state.job = None;
            self.shared.wake_workers.notify_all();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_loop(shared: Arc<PoolShared>) {
    let mut seen_generation = 0_u64;
    loop {
        let job = {
            let mut state = shared.state.lock().expect("worker state lock");
            while !state.shutdown && state.generation == seen_generation {
                state = shared
                    .wake_workers
                    .wait(state)
                    .expect("worker condvar wait");
            }
            if state.shutdown {
                return;
            }
            seen_generation = state.generation;
            state.job.clone()
        };

        if let Some(job) = job {
            let mut board = job.board.clone();
            let mut caches = job.caches.clone();
            let mut searcher = AlphaBetaSearcher::with_tt(job.config.clone(), job.tt.clone());
            searcher.set_tt_generation(job.tt_generation);
            let mut stats = SearchStats {
                deadline: job.deadline,
                cancel: job.cancel.clone(),
                ..SearchStats::default()
            };
            let abdada_ctx = AbdadaContext {
                table: Arc::clone(&job.abdada_table),
                salt: job.abdada_salt,
                root_best: Arc::clone(&job.root_best),
                depth_done: Arc::clone(&job.depth_done),
            };
            let (score, move_) = searcher.search_with_abdada(
                &mut board,
                &mut caches,
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
                let mut guard = job.completed_result.lock().expect("completed result lock");
                if guard.is_none() {
                    *guard = Some((score, move_));
                }
                job.depth_done.store(true, Ordering::Relaxed);
            }
        }

        let mut state = shared.state.lock().expect("worker done lock");
        state.active_workers = state.active_workers.saturating_sub(1);
        if state.active_workers == 0 {
            shared.workers_done.notify_one();
        }
    }
}
