//! Lockless 5-tier 12-byte/entry compressed transposition table aligned with
//! Rapfi `search/hashtable.cpp`.
//!
//! Layout:
//! - `TTEntry` = 12 bytes (`key32` XOR-protected with `data[0] ^ data[1]`,
//!   `value16`, `eval16`, `pv_bound_best16`, `depth8`, `generation8`).
//! - `TTBucket` = 64 bytes (`[TTEntry; 5]` + 4B padding), 64-byte aligned so
//!   each 5-way probe touches exactly one CPU cache line.
//! - Uses 128-bit `mulhi64(key, num_buckets)` fast range mapping (works for
//!   any memory size, not just powers of two).
//! - Integer ply depth range: `DEPTH_LOWER_BOUND (-20)..=DEPTH_UPPER_BOUND (235)`.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use crate::fast_engine::board::NONE_MOVE;

pub const VALUE_ZERO: i32 = 0;
pub const VALUE_DRAW: i32 = 0;
pub const VALUE_MATE: i32 = 30_000;
pub const VALUE_INFINITE: i32 = 30_001;
pub const VALUE_NONE: i32 = -30_002;
pub const VALUE_MATE_IN_MAX_PLY: i32 = VALUE_MATE - 500;
pub const VALUE_MATED_IN_MAX_PLY: i32 = -VALUE_MATE + 500;
pub const VALUE_EVAL_MAX: i32 = 6000;
pub const VALUE_EVAL_MIN: i32 = -6000;

pub const DEPTH_QVCF_FULL: f32 = -1.0;
pub const DEPTH_QVCF: f32 = -2.0;
pub const DEPTH_NONE: f32 = -3.0;
pub const DEPTH_LOWER_BOUND: i32 = -20;
pub const DEPTH_UPPER_BOUND: i32 = DEPTH_LOWER_BOUND + 255;

pub const BOUND_NONE: u8 = 0;
pub const BOUND_UPPER: u8 = 1;
pub const BOUND_LOWER: u8 = 2;
pub const BOUND_EXACT: u8 = 3;

#[inline(always)]
pub const fn mate_in(ply: i32) -> i32 {
    VALUE_MATE - ply
}

#[inline(always)]
pub const fn mated_in(ply: i32) -> i32 {
    -VALUE_MATE + ply
}

#[inline(always)]
pub const fn mate_step(v: i32, ply: i32) -> i32 {
    let abs_v = if v < 0 { -v } else { v };
    VALUE_MATE - ply - abs_v
}

#[inline(always)]
pub const fn search_value_to_stored(value: i32, ply: i32) -> i16 {
    let v = if value == VALUE_NONE {
        VALUE_NONE
    } else if value >= VALUE_MATE_IN_MAX_PLY {
        value + ply
    } else if value <= VALUE_MATED_IN_MAX_PLY {
        value - ply
    } else {
        value
    };
    v as i16
}

#[inline(always)]
pub const fn stored_value_to_search(stored: i16, ply: i32) -> i32 {
    let v = stored as i32;
    if v == VALUE_NONE {
        VALUE_NONE
    } else if v >= VALUE_MATE_IN_MAX_PLY {
        v - ply
    } else if v <= VALUE_MATED_IN_MAX_PLY {
        v + ply
    } else {
        v
    }
}

const ENTRIES_PER_BUCKET: usize = 5;

/// Atomic 12-byte TT slot (3 x `AtomicU32`) so concurrent Lazy SMP reads/writes
/// have zero data-race UB in Rust while preserving Rapfi's exact XOR lockless
/// integrity check (`key32 ^ data0 ^ data1 == lo32(hash_key)`).
#[repr(C)]
struct AtomicTTEntry {
    key32: AtomicU32,
    data0: AtomicU32, // low 16: value16 (i16), high 16: eval16 (i16)
    data1: AtomicU32, // low 16: pv_bound_best16, bits 16..24: depth8, bits 24..32: generation8
}

#[repr(C, align(64))]
struct TTBucket {
    entries: [AtomicTTEntry; ENTRIES_PER_BUCKET],
    _pad: u32,
}

const _: () = assert!(std::mem::size_of::<AtomicTTEntry>() == 12);
const _: () = assert!(std::mem::size_of::<TTBucket>() == 64);

#[derive(Clone, Copy, Debug)]
pub struct TTProbeResult {
    pub value: i32,
    pub eval: i32,
    pub is_pv: bool,
    pub bound: u8,
    pub best_move: u16,
    pub depth: i32,
}

/// Encode a `FastBoard` move (`0..225` or `NONE_MOVE`) into the 10-bit TT slot.
/// `0` = `NONE_MOVE`, `1..=225` = `pos + 1`.
#[inline(always)]
fn encode_tt_move(mv: u16) -> u16 {
    if mv < 225 {
        mv + 1
    } else {
        0
    }
}

#[inline(always)]
fn decode_tt_move(bits: u16) -> u16 {
    let raw = bits & 0x3FF;
    if raw == 0 {
        NONE_MOVE
    } else {
        raw - 1
    }
}

#[inline(always)]
fn pack_data0(value16: i16, eval16: i16) -> u32 {
    (value16 as u16 as u32) | ((eval16 as u16 as u32) << 16)
}

#[inline(always)]
fn unpack_data0(d0: u32) -> (i16, i16) {
    (d0 as u16 as i16, (d0 >> 16) as u16 as i16)
}

#[inline(always)]
fn pack_data1(is_pv: bool, bound: u8, best_move: u16, depth8: u8, gen8: u8) -> u32 {
    let pbb: u16 =
        ((is_pv as u16) << 15) | (((bound & 0x3) as u16) << 13) | encode_tt_move(best_move);
    (pbb as u32) | ((depth8 as u32) << 16) | ((gen8 as u32) << 24)
}

#[inline(always)]
fn mulhi64(a: u64, b: usize) -> usize {
    (((a as u128) * (b as u128)) >> 64) as usize
}

/// Shared 5-tier transposition table. Safe to share across Lazy SMP threads via `&FastTT`.
pub struct FastTT {
    ptr: *mut TTBucket,
    num_buckets: usize,
    generation: AtomicU8,
}

impl std::fmt::Debug for FastTT {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastTT")
            .field("hash_size_kb", &self.hash_size_kb())
            .field("num_entries", &self.num_entries())
            .finish()
    }
}

// SAFETY: Every bucket slot is accessed strictly through `AtomicU32` relaxed
// loads/stores protected by the XOR key checksum; `ptr` is immutable during search.
unsafe impl Send for FastTT {}
unsafe impl Sync for FastTT {}

impl Drop for FastTT {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.num_buckets > 0 {
            let layout =
                Layout::array::<TTBucket>(self.num_buckets).expect("valid TTBucket array layout");
            unsafe { dealloc(self.ptr as *mut u8, layout) };
        }
    }
}

impl FastTT {
    pub fn new_kb(hash_size_kb: usize) -> Self {
        let mut num_buckets = (hash_size_kb * 1024) / std::mem::size_of::<TTBucket>();
        if num_buckets == 0 {
            num_buckets = 1;
        }
        let layout = Layout::array::<TTBucket>(num_buckets).expect("valid TTBucket array layout");
        let ptr = unsafe { alloc_zeroed(layout) as *mut TTBucket };
        assert!(
            !ptr.is_null(),
            "failed to allocate FastTT ({hash_size_kb} KiB)"
        );
        Self {
            ptr,
            num_buckets,
            generation: AtomicU8::new(0),
        }
    }

    pub fn hash_size_kb(&self) -> usize {
        (self.num_buckets * std::mem::size_of::<TTBucket>()) / 1024
    }

    pub fn num_entries(&self) -> usize {
        self.num_buckets * ENTRIES_PER_BUCKET
    }

    pub fn resize_kb(&mut self, hash_size_kb: usize) {
        let new_buckets = ((hash_size_kb * 1024) / std::mem::size_of::<TTBucket>()).max(1);
        if new_buckets == self.num_buckets {
            return;
        }
        let old_layout = Layout::array::<TTBucket>(self.num_buckets).expect("old layout");
        unsafe { dealloc(self.ptr as *mut u8, old_layout) };
        let new_layout = Layout::array::<TTBucket>(new_buckets).expect("new layout");
        let ptr = unsafe { alloc_zeroed(new_layout) as *mut TTBucket };
        assert!(
            !ptr.is_null(),
            "failed to allocate FastTT ({hash_size_kb} KiB)"
        );
        self.ptr = ptr;
        self.num_buckets = new_buckets;
        self.generation.store(0, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        let buckets = self.buckets();
        for b in buckets {
            for e in &b.entries {
                e.key32.store(0, Ordering::Relaxed);
                e.data0.store(0, Ordering::Relaxed);
                e.data1.store(0, Ordering::Relaxed);
            }
        }
        self.generation.store(0, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn inc_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    #[inline(always)]
    fn buckets(&self) -> &[TTBucket] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.num_buckets) }
    }

    #[inline(always)]
    fn bucket_for(&self, hash_key: u64) -> &TTBucket {
        let idx = mulhi64(hash_key, self.num_buckets);
        unsafe { &*self.ptr.add(idx) }
    }

    #[inline(always)]
    pub fn prefetch(&self, hash_key: u64) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let idx = mulhi64(hash_key, self.num_buckets);
            let p = self.ptr.add(idx) as *const i8;
            std::arch::x86_64::_mm_prefetch(p, std::arch::x86_64::_MM_HINT_T0);
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = hash_key;
    }

    #[inline]
    pub fn probe(&self, hash_key: u64, ply: i32) -> Option<TTProbeResult> {
        let bucket = self.bucket_for(hash_key);
        let key32 = hash_key as u32;
        let cur_gen = self.generation.load(Ordering::Relaxed);

        for e in &bucket.entries {
            let d0 = e.data0.load(Ordering::Relaxed);
            let d1 = e.data1.load(Ordering::Relaxed);
            let k = e.key32.load(Ordering::Relaxed);
            if (k ^ d0 ^ d1) == key32 {
                let depth8 = ((d1 >> 16) & 0xFF) as u8;
                let old_gen = ((d1 >> 24) & 0xFF) as u8;
                // Refresh generation on hit (matching Rapfi hashtable.cpp:197-198).
                if old_gen != cur_gen {
                    let new_d1 = (d1 & 0x00FF_FFFF) | ((cur_gen as u32) << 24);
                    e.data1.store(new_d1, Ordering::Relaxed);
                    e.key32.store(key32 ^ d0 ^ new_d1, Ordering::Relaxed);
                }
                let (val16, eval16) = unpack_data0(d0);
                let pbb = (d1 & 0xFFFF) as u16;
                return Some(TTProbeResult {
                    value: stored_value_to_search(val16, ply),
                    eval: eval16 as i32,
                    is_pv: (pbb >> 15) != 0,
                    bound: ((pbb >> 13) & 0x3) as u8,
                    best_move: decode_tt_move(pbb),
                    depth: depth8 as i32 + DEPTH_LOWER_BOUND,
                });
            }
        }
        None
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &self,
        hash_key: u64,
        value: i32,
        eval: i32,
        is_pv: bool,
        bound: u8,
        mut best_move: u16,
        depth: i32,
        ply: i32,
    ) {
        let bucket = self.bucket_for(hash_key);
        let new_key32 = hash_key as u32;
        let cur_gen = self.generation.load(Ordering::Relaxed);

        let mut replace_idx = 0_usize;
        let mut replace_score = i32::MAX;
        let mut replace_old_key = 0_u32;
        let mut replace_old_d1 = 0_u32;

        for (i, e) in bucket.entries.iter().enumerate() {
            let d0 = e.data0.load(Ordering::Relaxed);
            let d1 = e.data1.load(Ordering::Relaxed);
            let k = e.key32.load(Ordering::Relaxed);
            let old_key = k ^ d0 ^ d1;
            if old_key == new_key32 {
                replace_idx = i;
                replace_old_key = old_key;
                replace_old_d1 = d1;
                break;
            }
            let depth8 = ((d1 >> 16) & 0xFF) as i32;
            let gen8 = ((d1 >> 24) & 0xFF) as u8;
            let rel_age = cur_gen.wrapping_sub(gen8) as i32;
            let score = depth8 - rel_age;
            if i == 0 || score < replace_score {
                replace_idx = i;
                replace_score = score;
                replace_old_key = old_key;
                replace_old_d1 = d1;
            }
        }

        let old_depth = ((replace_old_d1 >> 16) & 0xFF) as i32 + DEPTH_LOWER_BOUND;
        // Overwrite same position only on exact bound or depth nearly as deep (Rapfi hashtable.cpp:245).
        if bound != BOUND_EXACT && new_key32 == replace_old_key && depth + 2 < old_depth {
            return;
        }

        // Preserve previously stored best move if we have none this time.
        if best_move == NONE_MOVE && new_key32 == replace_old_key {
            best_move = decode_tt_move((replace_old_d1 & 0xFFFF) as u16);
        }

        let clamped_depth = depth.clamp(DEPTH_LOWER_BOUND + 1, DEPTH_UPPER_BOUND);
        let depth8 = (clamped_depth - DEPTH_LOWER_BOUND) as u8;
        let val16 = search_value_to_stored(value, ply);
        let eval16 = eval.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let d0 = pack_data0(val16, eval16);
        let d1 = pack_data1(is_pv, bound, best_move, depth8, cur_gen);
        let k = new_key32 ^ d0 ^ d1;

        let slot = &bucket.entries[replace_idx];
        slot.data0.store(d0, Ordering::Relaxed);
        slot.data1.store(d1, Ordering::Relaxed);
        slot.key32.store(k, Ordering::Relaxed);
    }

    pub fn hash_usage_permill(&self) -> usize {
        let cur_gen = self.generation.load(Ordering::Relaxed);
        let test_cnt = (self.num_buckets >> 10).max(1);
        let buckets = self.buckets();
        let mut cnt = 0_usize;
        for b in &buckets[..test_cnt] {
            for e in &b.entries {
                let d1 = e.data1.load(Ordering::Relaxed);
                let depth8 = ((d1 >> 16) & 0xFF) as u8;
                let gen8 = ((d1 >> 24) & 0xFF) as u8;
                if depth8 != 0 && gen8 == cur_gen {
                    cnt += 1;
                }
            }
        }
        (cnt * 1000) / (ENTRIES_PER_BUCKET * test_cnt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_tt_roundtrip_mate_and_normal_entries() {
        let tt = FastTT::new_kb(1024);
        assert_eq!(tt.hash_size_kb(), 1024);
        assert_eq!(tt.num_entries(), (1024 * 1024 / 64) * 5);

        let key = 0x1234_5678_9ABC_DEF1_u64;
        assert!(tt.probe(key, 4).is_none());

        // Normal entry
        tt.store(key, 345, 210, true, BOUND_EXACT, 112, 12, 4);
        let hit = tt.probe(key, 4).expect("normal hit");
        assert_eq!(hit.value, 345);
        assert_eq!(hit.eval, 210);
        assert!(hit.is_pv);
        assert_eq!(hit.bound, BOUND_EXACT);
        assert_eq!(hit.best_move, 112);
        assert_eq!(hit.depth, 12);

        // Mate entry adjusts across ply!
        let key2 = 0xFEDC_BA98_7654_3210_u64;
        tt.store(key2, mate_in(9), VALUE_NONE, false, BOUND_LOWER, 55, 8, 4);
        let hit_at_4 = tt.probe(key2, 4).expect("mate hit at ply 4");
        assert_eq!(hit_at_4.value, mate_in(9));
        let hit_at_6 = tt.probe(key2, 6).expect("mate hit at ply 6");
        assert_eq!(hit_at_6.value, mate_in(11)); // same distance-to-mate (5 plies from node)
        assert_eq!(hit_at_6.eval, VALUE_NONE);

        // Shallow non-exact store should not overwrite deep entry for same key.
        tt.store(key, -100, 0, false, BOUND_UPPER, NONE_MOVE, 6, 4);
        let still = tt.probe(key, 4).expect("still deep");
        assert_eq!(still.depth, 12);
        assert_eq!(still.best_move, 112);
    }
}
