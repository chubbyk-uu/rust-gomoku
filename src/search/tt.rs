//! Transposition table aligned with the classic reference.

use std::array;
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc, OnceLock,
};

use crate::constants::{HASHF_ALPHA, HASHF_BETA, HASHF_EMPTY, HASHF_EXACT};
use crate::types::Move;

pub const MAX_TT_BUCKET_BITS: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TTError {
    BucketBitsOutOfRange(u32),
    AllocationFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TTEntry {
    pub key: u64,
    pub value: i32,
    pub flag: u8,
    pub depth: i32,
    pub priority: i32,
    pub best_move: Option<Move>,
    pub generation: u16,
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
            generation: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TTBestMoveHint {
    pub move_: Move,
    pub depth: i32,
    pub flag: u8,
    pub generation: u16,
}

impl TTBestMoveHint {
    fn from_entry(entry: TTEntry) -> Option<Self> {
        entry.best_move.map(|move_| Self {
            move_,
            depth: entry.depth,
            flag: entry.flag,
            generation: entry.generation,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub value: Option<i32>,
    pub best_move: Option<Move>,
    pub best_move_hint: Option<TTBestMoveHint>,
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
            best_move_hint: None,
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
    primary_mode: AtomicU8,
    secondary_buckets: OnceLock<Option<Vec<[RawEntry; 2]>>>,
}

impl TTStorage {
    fn alloc_buckets(bucket_count: usize) -> Result<Vec<[RawEntry; 2]>, TTError> {
        let mut buckets = Vec::new();
        buckets
            .try_reserve_exact(bucket_count)
            .map_err(|_| TTError::AllocationFailed)?;
        buckets.extend((0..bucket_count).map(|_| array::from_fn(|_| RawEntry::default())));
        Ok(buckets)
    }

    fn try_new(bucket_count: usize) -> Result<Self, TTError> {
        Ok(Self {
            buckets: Self::alloc_buckets(bucket_count)?,
            primary_mode: AtomicU8::new(0),
            secondary_buckets: OnceLock::new(),
        })
    }

    fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    #[inline]
    fn buckets_for_mode(&self, subply: bool) -> &[[RawEntry; 2]] {
        let wanted = if subply { 2_u8 } else { 1_u8 };
        let current = match self.primary_mode.load(Ordering::Acquire) {
            0 => match self.primary_mode.compare_exchange(
                0,
                wanted,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => wanted,
                Err(actual) => actual,
            },
            actual => actual,
        };
        if current == wanted {
            return &self.buckets;
        }
        if let Some(sec) = self
            .secondary_buckets
            .get_or_init(|| Self::alloc_buckets(self.buckets.len()).ok())
            .as_ref()
        {
            sec
        } else {
            &self.buckets
        }
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

    fn load_for_key(&self, key: u64, expected_subply: bool) -> TTEntry {
        let data = self.data.load(Ordering::Relaxed);
        let key_check = self.key_check.load(Ordering::Relaxed);
        if data == 0 || (key_check ^ data) != key || unpack_subply(data) != expected_subply {
            return TTEntry::default();
        }
        unpack_entry(key, data)
    }

    fn load_raw_with_mode(&self) -> (TTEntry, bool) {
        let data = self.data.load(Ordering::Relaxed);
        let key_check = self.key_check.load(Ordering::Relaxed);
        if data == 0 {
            return (TTEntry::default(), false);
        }
        (unpack_entry(key_check ^ data, data), unpack_subply(data))
    }

    fn store(&self, entry: TTEntry, subply: bool) {
        let data = pack_entry(entry, subply);
        self.data.store(data, Ordering::Relaxed);
        self.key_check.store(entry.key ^ data, Ordering::Relaxed);
    }
}

const VALUE_SHIFT: u64 = 48;
const FLAG_SHIFT: u64 = 46;
const DEPTH_SHIFT: u64 = 34;
const DEPTH_MASK: u64 = 0x0FFF;
pub(crate) const DEPTH_MAX: i32 = 0x0FFF;
const PRIORITY_SHIFT: u64 = 18;
const PRIORITY_MASK: u64 = 0xFFFF;
const PRIORITY_MAX: i32 = 0xFFFF;
const BEST_MOVE_SHIFT: u64 = 10;
const BEST_MOVE_NONE: u8 = 0xFF;
const SUBPLY_SHIFT: u64 = 9;
pub(crate) const GENERATION_MASK: u16 = 0x01FF;

#[inline]
fn unpack_subply(data: u64) -> bool {
    ((data >> SUBPLY_SHIFT) & 1) != 0
}

fn pack_entry(entry: TTEntry, subply: bool) -> u64 {
    debug_assert!(
        i32::from(entry.value as i16) == entry.value,
        "TT value {} does not fit the i16 packing field",
        entry.value
    );
    debug_assert!(
        (0..=DEPTH_MAX).contains(&entry.depth),
        "TT depth {} outside the 12-bit packing range",
        entry.depth
    );
    debug_assert!(
        (0..=PRIORITY_MAX).contains(&entry.priority),
        "TT priority {} outside the u16 packing range",
        entry.priority
    );
    let value = (entry.value as i16 as u16 as u64) << VALUE_SHIFT;
    let flag = (u64::from(entry.flag) & 0x3) << FLAG_SHIFT;
    let depth = (entry.depth.clamp(0, DEPTH_MAX) as u64) << DEPTH_SHIFT;
    let priority = (entry.priority.clamp(0, PRIORITY_MAX) as u64) << PRIORITY_SHIFT;
    let best_move = entry
        .best_move
        .and_then(|move_| u8::try_from(move_).ok())
        .unwrap_or(BEST_MOVE_NONE);
    let best_move = u64::from(best_move) << BEST_MOVE_SHIFT;
    let subply_bit = u64::from(subply) << SUBPLY_SHIFT;
    let generation = u64::from(entry.generation & GENERATION_MASK);
    value | flag | depth | priority | best_move | subply_bit | generation
}

fn unpack_entry(key: u64, data: u64) -> TTEntry {
    let value = ((data >> VALUE_SHIFT) as u16) as i16 as i32;
    let flag = ((data >> FLAG_SHIFT) & 0x3) as u8;
    let depth = ((data >> DEPTH_SHIFT) & DEPTH_MASK) as i32;
    let priority = ((data >> PRIORITY_SHIFT) & PRIORITY_MASK) as i32;
    let best_move = ((data >> BEST_MOVE_SHIFT) & 0xFF) as u8;
    let generation = (data & u64::from(GENERATION_MASK)) as u16;
    TTEntry {
        key,
        value,
        flag,
        depth,
        priority,
        best_move: (best_move != BEST_MOVE_NONE).then_some(best_move as Move),
        generation,
    }
}

#[derive(Clone, Debug)]
pub struct TranspositionTable {
    pub bucket_mask: u64,
    storage: Arc<TTStorage>,
    subply: bool,
}

impl TranspositionTable {
    pub fn try_new(bucket_bits: u32) -> Result<Self, TTError> {
        if bucket_bits > MAX_TT_BUCKET_BITS || bucket_bits >= usize::BITS {
            return Err(TTError::BucketBitsOutOfRange(bucket_bits));
        }
        let bucket_count = 1_usize
            .checked_shl(bucket_bits)
            .ok_or(TTError::BucketBitsOutOfRange(bucket_bits))?;
        Ok(Self {
            bucket_mask: (1_u64 << bucket_bits) - 1,
            storage: Arc::new(TTStorage::try_new(bucket_count)?),
            subply: false,
        })
    }

    pub fn new(bucket_bits: u32) -> Self {
        Self::try_new(bucket_bits).expect("trusted TT bucket bits stay within supported bounds")
    }

    /// Allocates a TT at `preferred_bits`, stepping down toward `min_bits` if
    /// memory allocation fails under tight process limits.
    pub fn try_new_with_fallback(
        preferred_bits: u32,
        min_bits: u32,
    ) -> Result<(Self, u32), TTError> {
        let floor = min_bits.min(preferred_bits);
        let mut bits = preferred_bits;
        loop {
            match Self::try_new(bits) {
                Ok(tt) => return Ok((tt, bits)),
                Err(TTError::AllocationFailed) if bits > floor => {
                    bits -= 1;
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn bucket(&self, key: u64) -> [TTEntry; 2] {
        self.storage.buckets_for_mode(self.subply)[self.bucket_index(key)]
            .each_ref()
            .map(|entry| entry.load_for_key(key, self.subply))
    }

    pub fn bucket_count(&self) -> usize {
        self.storage.bucket_count()
    }

    pub fn is_shared_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    pub fn clear(&self) {
        for bucket in &self.storage.buckets {
            for entry in bucket {
                entry.store_words(0, 0);
            }
        }
        if let Some(Some(sec)) = self.storage.secondary_buckets.get() {
            for bucket in sec {
                for entry in bucket {
                    entry.store_words(0, 0);
                }
            }
        }
    }

    /// Selects the depth encoding partition (`false` = integer-ply 1T mode,
    /// `true` = 1/64-ply multi-threaded mode) on this table handle.
    /// The first mode used claims the primary bucket array; if the other mode
    /// is also used on the same `Arc<TTStorage>`, it automatically routes to
    /// an isolated secondary bucket array (plus per-entry mode-bit tagging).
    pub fn set_subply(&mut self, subply: bool) {
        self.subply = subply;
    }

    pub fn is_subply(&self) -> bool {
        self.subply
    }

    #[cfg(test)]
    pub(crate) fn has_secondary_buckets(&self) -> bool {
        self.storage
            .secondary_buckets
            .get()
            .is_some_and(Option::is_some)
    }

    pub fn fork_snapshot(&self) -> Self {
        let snapshot = Self {
            bucket_mask: self.bucket_mask,
            storage: Arc::new(
                TTStorage::try_new(self.storage.bucket_count())
                    .expect("existing TT capacity remains allocatable for snapshot"),
            ),
            subply: self.subply,
        };
        let src_buckets = self.storage.buckets_for_mode(self.subply);
        let dst_buckets = snapshot.storage.buckets_for_mode(self.subply);
        for (src_bucket, dst_bucket) in src_buckets.iter().zip(dst_buckets.iter()) {
            for (src, dst) in src_bucket.iter().zip(dst_bucket.iter()) {
                let (key_check, data) = src.load_words();
                dst.store_words(key_check, data);
            }
        }
        snapshot
    }

    pub(crate) fn view(&self) -> TTTableView<'_> {
        TTTableView {
            buckets: self.storage.buckets_for_mode(self.subply),
            bucket_mask: self.bucket_mask,
            subply: self.subply,
        }
    }

    pub fn store(&self, entry: TTEntry) {
        self.view().store(entry);
    }

    pub fn probe(&self, key: u64, depth: i32, alpha: i32, beta: i32) -> ProbeResult {
        self.view().probe(key, depth, alpha, beta)
    }

    fn bucket_index(&self, key: u64) -> usize {
        (key & self.bucket_mask) as usize
    }
}

/// Borrowed view of the active TT bucket plane for a single search invocation.
/// Resolves the 1T/MT mode partition once at search entry so hot-path probes,
/// stores, and prefetches avoid per-node atomic mode checks.
#[derive(Clone, Copy)]
pub(crate) struct TTTableView<'a> {
    buckets: &'a [[RawEntry; 2]],
    bucket_mask: u64,
    subply: bool,
}

impl<'a> TTTableView<'a> {
    #[inline(always)]
    fn bucket_index(&self, key: u64) -> usize {
        (key & self.bucket_mask) as usize
    }

    #[inline(always)]
    pub(crate) fn is_subply(&self) -> bool {
        self.subply
    }

    #[inline(always)]
    pub(crate) fn prefetch(&self, key: u64) {
        #[cfg(target_arch = "x86_64")]
        {
            let ptr = self.buckets.as_ptr().wrapping_add(self.bucket_index(key)) as *const i8;
            // SAFETY: `bucket_index(key)` is masked within `self.buckets.len()`,
            // and `_mm_prefetch` is a non-faulting hint instruction.
            unsafe {
                std::arch::x86_64::_mm_prefetch(ptr, std::arch::x86_64::_MM_HINT_T0);
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = key;
        }
    }

    #[inline]
    pub(crate) fn store(&self, entry: TTEntry) {
        let bucket = &self.buckets[self.bucket_index(entry.key)];
        let (first, first_subply) = bucket[0].load_raw_with_mode();
        let mut slot = 0_usize;
        if first.flag != HASHF_EMPTY
            && (first_subply != self.subply || first.priority > entry.priority)
        {
            slot = 1;
        }
        bucket[slot].store(entry, self.subply);
    }

    #[inline]
    pub(crate) fn probe(&self, key: u64, depth: i32, alpha: i32, beta: i32) -> ProbeResult {
        let mut fallback_best_move = None;
        let mut fallback_best_move_hint = None;
        let raw_bucket = &self.buckets[self.bucket_index(key)];
        for raw_entry in raw_bucket {
            let entry = raw_entry.load_for_key(key, self.subply);
            if entry.key != key {
                continue;
            }
            let best_move_hint = TTBestMoveHint::from_entry(entry);
            if entry.depth >= depth {
                if entry.flag == HASHF_EXACT {
                    return ProbeResult {
                        value: Some(entry.value),
                        best_move: entry.best_move,
                        best_move_hint,
                        hit: true,
                        ..ProbeResult::default()
                    };
                }
                if entry.flag == HASHF_ALPHA {
                    if entry.value <= alpha {
                        return ProbeResult {
                            value: Some(entry.value),
                            best_move: None,
                            best_move_hint,
                            hit: true,
                            ..ProbeResult::default()
                        };
                    }
                    return ProbeResult {
                        value: None,
                        best_move: entry.best_move,
                        best_move_hint,
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
                            best_move_hint,
                            hit: true,
                            ..ProbeResult::default()
                        };
                    }
                    return ProbeResult {
                        value: None,
                        best_move: entry.best_move,
                        best_move_hint,
                        hit: false,
                        has_window: true,
                        window_alpha: alpha.max(entry.value),
                        window_beta: beta,
                    };
                }
            }
            fallback_best_move = entry.best_move;
            fallback_best_move_hint = best_move_hint;
        }
        ProbeResult {
            value: None,
            best_move: fallback_best_move,
            best_move_hint: fallback_best_move_hint,
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
