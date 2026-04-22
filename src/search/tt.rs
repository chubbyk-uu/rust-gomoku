//! Transposition table aligned with the classic reference.

use std::collections::HashMap;

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

#[derive(Clone, Debug)]
pub struct TranspositionTable {
    pub bucket_mask: u64,
    pub buckets: HashMap<u64, [TTEntry; 2]>,
}

impl TranspositionTable {
    pub fn new(bucket_bits: u32) -> Self {
        Self {
            bucket_mask: (1_u64 << bucket_bits) - 1,
            buckets: HashMap::new(),
        }
    }

    pub fn bucket(&self, key: u64) -> [TTEntry; 2] {
        self.buckets
            .get(&(key & self.bucket_mask))
            .copied()
            .unwrap_or([TTEntry::default(), TTEntry::default()])
    }

    pub fn store(&mut self, entry: TTEntry) {
        let slot_key = entry.key & self.bucket_mask;
        let bucket = self
            .buckets
            .entry(slot_key)
            .or_insert([TTEntry::default(), TTEntry::default()]);
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
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(20)
    }
}
