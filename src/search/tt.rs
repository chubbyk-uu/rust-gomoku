//! Transposition table aligned with the classic reference.

use std::array;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

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
    buckets: Vec<[RawEntry; 2]>,
}

impl TTStorage {
    fn new(bucket_count: usize) -> Self {
        let buckets = (0..bucket_count)
            .map(|_| array::from_fn(|_| RawEntry::default()))
            .collect();
        Self { buckets }
    }

    fn bucket_count(&self) -> usize {
        self.buckets.len()
    }
}

#[derive(Debug)]
struct RawEntry {
    key_check: AtomicU64,
    data: AtomicU64,
}

impl Default for RawEntry {
    fn default() -> Self {
        Self {
            key_check: AtomicU64::new(0),
            data: AtomicU64::new(0),
        }
    }
}

impl RawEntry {
    fn load_words(&self) -> (u64, u64) {
        (
            self.key_check.load(Ordering::Relaxed),
            self.data.load(Ordering::Relaxed),
        )
    }

    fn store_words(&self, key_check: u64, data: u64) {
        self.data.store(data, Ordering::Relaxed);
        self.key_check.store(key_check, Ordering::Relaxed);
    }

    fn load_for_key(&self, key: u64) -> TTEntry {
        let data = self.data.load(Ordering::Relaxed);
        let key_check = self.key_check.load(Ordering::Relaxed);
        if data == 0 || (key_check ^ data) != key {
            return TTEntry::default();
        }
        unpack_entry(key, data)
    }

    fn load_raw(&self) -> TTEntry {
        let data = self.data.load(Ordering::Relaxed);
        let key_check = self.key_check.load(Ordering::Relaxed);
        if data == 0 {
            return TTEntry::default();
        }
        unpack_entry(key_check ^ data, data)
    }

    fn store(&self, entry: TTEntry) {
        let data = pack_entry(entry);
        self.data.store(data, Ordering::Relaxed);
        self.key_check.store(entry.key ^ data, Ordering::Relaxed);
    }
}

const VALUE_SHIFT: u64 = 48;
const FLAG_SHIFT: u64 = 46;
const DEPTH_SHIFT: u64 = 38;
const PRIORITY_SHIFT: u64 = 22;
const BEST_MOVE_SHIFT: u64 = 14;
const BEST_MOVE_NONE: u8 = 0xFF;

fn pack_entry(entry: TTEntry) -> u64 {
    let value = (entry.value as i16 as u16 as u64) << VALUE_SHIFT;
    let flag = (u64::from(entry.flag) & 0x3) << FLAG_SHIFT;
    let depth = (entry.depth.clamp(0, u8::MAX as i32) as u64) << DEPTH_SHIFT;
    let priority = (entry.priority.clamp(0, u16::MAX as i32) as u64) << PRIORITY_SHIFT;
    let best_move = entry
        .best_move
        .and_then(|move_| u8::try_from(move_).ok())
        .unwrap_or(BEST_MOVE_NONE);
    let best_move = u64::from(best_move) << BEST_MOVE_SHIFT;
    value | flag | depth | priority | best_move
}

fn unpack_entry(key: u64, data: u64) -> TTEntry {
    let value = ((data >> VALUE_SHIFT) as u16) as i16 as i32;
    let flag = ((data >> FLAG_SHIFT) & 0x3) as u8;
    let depth = ((data >> DEPTH_SHIFT) & 0xFF) as i32;
    let priority = ((data >> PRIORITY_SHIFT) & 0xFFFF) as i32;
    let best_move = ((data >> BEST_MOVE_SHIFT) & 0xFF) as u8;
    TTEntry {
        key,
        value,
        flag,
        depth,
        priority,
        best_move: (best_move != BEST_MOVE_NONE).then_some(best_move as Move),
    }
}

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
        self.storage.buckets[self.bucket_index(key)]
            .each_ref()
            .map(|entry| entry.load_for_key(key))
    }

    pub fn bucket_count(&self) -> usize {
        self.storage.bucket_count()
    }

    pub fn is_shared_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    pub fn fork_snapshot(&self) -> Self {
        let snapshot = Self {
            bucket_mask: self.bucket_mask,
            storage: Arc::new(TTStorage::new(self.storage.bucket_count())),
        };
        for (src_bucket, dst_bucket) in self
            .storage
            .buckets
            .iter()
            .zip(snapshot.storage.buckets.iter())
        {
            for (src, dst) in src_bucket.iter().zip(dst_bucket.iter()) {
                let (key_check, data) = src.load_words();
                dst.store_words(key_check, data);
            }
        }
        snapshot
    }

    pub fn store(&self, entry: TTEntry) {
        let bucket = &self.storage.buckets[self.bucket_index(entry.key)];
        let first = bucket[0].load_raw();
        let mut slot = 0_usize;
        if first.flag != HASHF_EMPTY && first.priority > entry.priority {
            slot = 1;
        }
        bucket[slot].store(entry);
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
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(20)
    }
}
