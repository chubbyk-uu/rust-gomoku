//! Transposition table aligned with the classic reference.

use std::sync::{Arc, RwLock};

use crate::constants::{HASHF_ALPHA, HASHF_BETA, HASHF_EMPTY, HASHF_EXACT};
use crate::types::Move;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TTEntry {
    pub key: u64,
    pub value: i32,
    pub flag: u8,
    pub depth: i32,
    pub priority: i32,
    pub best_move: Option<Move>,
}

impl Default for TTEntry {
    fn default() -> Self {
        Self {
            key: 0,
            value: 0,
            flag: HASHF_EMPTY,
            depth: 0,
            priority: 0,
            best_move: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub value: Option<i32>,
    pub best_move: Option<Move>,
    pub hit: bool,
    pub has_window: bool,
    pub window_alpha: i32,
    pub window_beta: i32,
}

impl Default for ProbeResult {
    fn default() -> Self {
        Self {
            value: None,
            best_move: None,
            hit: false,
            has_window: false,
            window_alpha: 0,
            window_beta: 0,
        }
    }
}

#[derive(Debug)]
struct TTStorage {
    shards: Vec<RwLock<Vec<[TTEntry; 2]>>>,
}

impl TTStorage {
    fn new(bucket_count: usize) -> Self {
        let shard_count = TT_SHARD_COUNT.min(bucket_count.max(1));
        let base_len = bucket_count / shard_count;
        let remainder = bucket_count % shard_count;
        let shards = (0..shard_count)
            .map(|shard_index| {
                let len = base_len + usize::from(shard_index < remainder);
                RwLock::new(vec![[TTEntry::default(), TTEntry::default()]; len])
            })
            .collect();
        Self { shards }
    }

    fn bucket_count(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.read().expect("TT shard lock is not poisoned").len())
            .sum()
    }
}

const TT_SHARD_COUNT: usize = 256;

#[derive(Clone, Debug)]
pub struct TranspositionTable {
    pub bucket_mask: u64,
    storage: Arc<TTStorage>,
}

impl TranspositionTable {
    pub fn new(bucket_bits: u32) -> Self {
        let bucket_count = 1_usize << bucket_bits;
        Self {
            bucket_mask: (1_u64 << bucket_bits) - 1,
            storage: Arc::new(TTStorage::new(bucket_count)),
        }
    }

    pub fn bucket(&self, key: u64) -> [TTEntry; 2] {
        let (shard_index, local_index) = self.shard_index(key);
        self.storage.shards[shard_index]
            .read()
            .expect("TT shard lock is not poisoned")[local_index]
    }

    pub fn bucket_count(&self) -> usize {
        self.storage.bucket_count()
    }

    pub fn is_shared_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    pub fn store(&self, entry: TTEntry) {
        let (shard_index, local_index) = self.shard_index(entry.key);
        let mut shard = self.storage.shards[shard_index]
            .write()
            .expect("TT shard lock is not poisoned");
        let bucket = &mut shard[local_index];
        let mut slot = 0_usize;
        if bucket[0].flag != HASHF_EMPTY && bucket[0].priority > entry.priority {
            slot = 1;
        }
        bucket[slot] = entry;
    }

    pub fn probe(&self, key: u64, depth: i32, alpha: i32, beta: i32) -> ProbeResult {
        let mut fallback_best_move = None;
        for entry in self.bucket(key) {
            if entry.key != key {
                continue;
            }
            if entry.depth >= depth {
                if entry.flag == HASHF_EXACT {
                    return ProbeResult {
                        value: Some(entry.value),
                        best_move: entry.best_move,
                        hit: true,
                        ..ProbeResult::default()
                    };
                }
                if entry.flag == HASHF_ALPHA {
                    if entry.value <= alpha {
                        return ProbeResult {
                            value: Some(entry.value),
                            best_move: None,
                            hit: true,
                            ..ProbeResult::default()
                        };
                    }
                    return ProbeResult {
                        value: None,
                        best_move: entry.best_move,
                        hit: false,
                        has_window: true,
                        window_alpha: alpha,
                        window_beta: beta.min(entry.value + 1),
                    };
                }
                if entry.flag == HASHF_BETA {
                    if entry.value >= beta {
                        return ProbeResult {
                            value: Some(entry.value),
                            best_move: None,
                            hit: true,
                            ..ProbeResult::default()
                        };
                    }
                    return ProbeResult {
                        value: None,
                        best_move: entry.best_move,
                        hit: false,
                        has_window: true,
                        window_alpha: alpha.max(entry.value),
                        window_beta: beta,
                    };
                }
            }
            fallback_best_move = entry.best_move;
        }
        ProbeResult {
            value: None,
            best_move: fallback_best_move,
            hit: false,
            ..ProbeResult::default()
        }
    }

    fn bucket_index(&self, key: u64) -> usize {
        (key & self.bucket_mask) as usize
    }

    fn shard_index(&self, key: u64) -> (usize, usize) {
        let bucket_index = self.bucket_index(key);
        let shard_count = self.storage.shards.len();
        let shard_index = bucket_index % shard_count;
        let local_index = bucket_index / shard_count;
        (shard_index, local_index)
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(20)
    }
}
