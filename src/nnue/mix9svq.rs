//! Incremental `Mix9svq` NNUE weight loader and evaluator, bit-exact with
//! Rapfi's `Evaluation::mix9svq` (`mix9svqnnue.cpp` + `mixcommon.h`).

use std::io;
use std::path::Path;
use std::sync::Arc;

use crate::board::{move_to_xy, Board};
use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, EMPTY};
use crate::nnue::lz4::decompress_lz4_frame;
use crate::nnue::simd::{
    avg_u8, compute_signed_input_bias, crelu_i32_to_i16_scalar, crelu_i32_to_i8_scalar,
    dot2_u7i8_scalar, has_avx2, has_avxvnni, has_neon, has_neon_dotprod,
    linear_i16_chunk2_prerelu_scalar, linear_i8_chunk4_scalar, linear_i8_row4_scalar, mulhi_i16,
    preprocess_dynamic_weight_linear, preprocess_linear,
};
use crate::rules::RuleSet;
use crate::types::Side;

#[cfg(target_arch = "x86_64")]
use crate::nnue::simd::{
    avg_pool_2x2_u8_avx2, commit_conv_cell_avx2, commit_vsum_avx2, crelu_i32_to_i16_avx2,
    crelu_i32_to_i8_avx2, dot2_u7i8_avx2, linear_i16_chunk2_prerelu_avx2, linear_i8_chunk4_avx2,
    linear_i8_chunk4_avxvnni, linear_i8_chunk4_batch4_avxvnni, linear_i8_row4_avx2,
    linear_i8_row4_avxvnni, policy_dot16_avx2, policy_eval_batch4_avx2, update_point_avx2,
};

#[cfg(target_arch = "aarch64")]
use crate::nnue::simd::{
    avg_pool_2x2_u8_neon, commit_conv_cell_neon, commit_vsum_neon, crelu_i32_to_i16_neon,
    crelu_i32_to_i8_neon, dot2_u7i8_neon, linear_i16_chunk2_prerelu_neon, linear_i8_chunk4_neon,
    linear_i8_chunk4_neon_dotprod, linear_i8_row4_neon, linear_i8_row4_neon_dotprod,
    policy_dot16_neon, update_point_neon,
};

pub const SHAPE_NUM: usize = 442_503;
pub const CODEBOOK_SIZE: usize = 65_536;
pub const FEATURE_DIM: usize = 64;
pub const POLICY_DIM: usize = 32;
pub const VALUE_DIM: usize = 64;
pub const FEAT_DWCONV_DIM: usize = 32;
pub const POLICY_PWCONV_DIM: usize = 16;

pub const WEIGHT_MAGIC: u32 = 0xACD8_CC6A;
pub const MIX9SVQ_ARCH_HASH: u32 = 0x84A0_71FE
    ^ (((FEAT_DWCONV_DIM as u32 / 8) << 20)
        | ((VALUE_DIM as u32 / 8) << 14)
        | ((POLICY_DIM as u32 / 8) << 8)
        | (FEATURE_DIM as u32 / 8));

const MAX_INNER_CHANGES_15: usize = 7_646;
const MAX_OUTER_CHANGES_15: usize = 23_699;
const OUTER_BOARD_SIZE: usize = BOARD_SIZE + 2;
const OUTER_BOARD_AREA: usize = OUTER_BOARD_SIZE * OUTER_BOARD_SIZE;

const DX: [isize; 4] = [1, 0, 1, 1];
const DY: [isize; 4] = [0, 1, -1, 1];

const POWER3: [u32; 12] = [
    1, 3, 9, 27, 81, 243, 729, 2_187, 6_561, 19_683, 59_049, 177_147,
];

/// Precomputed mapping from inner board cell `y * 15 + x` to padded outer cell
/// `(y + 1) * 17 + (x + 1)`, eliminating integer div/mod per candidate in policy head.
const INNER_TO_OUTER: [u16; BOARD_AREA] = {
    let mut lut = [0_u16; BOARD_AREA];
    let mut y = 0;
    while y < BOARD_SIZE {
        let mut x = 0;
        while x < BOARD_SIZE {
            lut[y * BOARD_SIZE + x] = ((y + 1) * OUTER_BOARD_SIZE + (x + 1)) as u16;
            x += 1;
        }
        y += 1;
    }
    lut
};

/// Exact 13x13 bitmask of outer cells touched by the 3x3 depthwise convolution
/// around the 4 star rays of radius 5 (`-5..=5`). Cells outside this mask always
/// have zero `map_conv` delta and can safely reuse their previous version slot
/// (~35% fewer `map_conv` zero/commit ops per move, 100% bit-exact).
const STAR_CONV_MASK: [u16; 13] = {
    let mut mask = [0_u16; 13];
    let mut dir = 0;
    while dir < 4 {
        let mut dist = -5_isize;
        while dist <= 5 {
            let cx = 6 + dist * DX[dir];
            let cy = 6 + dist * DY[dir];
            let mut dy = -1_isize;
            while dy <= 1 {
                let mut dx = -1_isize;
                while dx <= 1 {
                    let rx = (cx + dx) as usize;
                    let ry = (cy + dy) as usize;
                    mask[ry] |= 1_u16 << rx;
                    dx += 1;
                }
                dy += 1;
            }
            dist += 1;
        }
        dir += 1;
    }
    mask
};

/// Rapfi's default value scaling factor (matches `Evaluation::ScalingFactor = 200.0f`).
pub const NNUE_SCALING_FACTOR: f32 = 200.0;
/// Safe non-mate clamp strictly below `crate::constants::WIN` (20_000).
pub const NNUE_VALUE_CLAMP: i32 = 15_000;

#[derive(Clone, Debug)]
pub struct StandardHeader {
    pub arch_hash: u32,
    pub rule_mask: u32,
    pub boardsize_mask: u32,
    pub description: String,
}

impl StandardHeader {
    pub fn supports_rule(&self, rule: RuleSet) -> bool {
        match rule {
            RuleSet::Freestyle => (self.rule_mask & 0x1) != 0,
            RuleSet::Renju => (self.rule_mask & 0x4) != 0,
        }
    }

    pub fn supports_board_size(&self, size: usize) -> bool {
        size >= 1 && size <= 32 && ((self.boardsize_mask >> (size - 1)) & 0x1) != 0
    }
}

#[derive(Clone, Debug)]
pub struct FcWeight<const OUT: usize, const IN: usize> {
    weight: Vec<i8>,
    bias: Vec<i32>,
    /// Precomputed `bias[o] - 128 * sum_j weight[o][j]` for AVX-VNNI signed-i8 inputs.
    signed_bias: Vec<i32>,
}

impl<const OUT: usize, const IN: usize> FcWeight<OUT, IN> {
    /// Construct a shape-validated fully-connected layer weight.
    /// Enforces `weight.len() == OUT * IN` and `bias.len() == OUT` at the
    /// construction boundary so SIMD kernels can rely on exact dimensions
    /// without per-evaluation bounds overhead.
    pub fn new(weight: Vec<i8>, bias: Vec<i32>) -> io::Result<Self> {
        if weight.len() != OUT * IN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FcWeight<{OUT}, {IN}> weight length mismatch: got {}, expected {}",
                    weight.len(),
                    OUT * IN
                ),
            ));
        }
        if bias.len() != OUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FcWeight<{OUT}, {IN}> bias length mismatch: got {}, expected {OUT}",
                    bias.len()
                ),
            ));
        }
        let signed_bias = compute_signed_input_bias::<OUT, IN>(&weight, &bias);
        Ok(Self {
            weight,
            bias,
            signed_bias,
        })
    }

    #[inline(always)]
    pub fn weight(&self) -> &[i8] {
        &self.weight
    }

    #[inline(always)]
    pub fn bias(&self) -> &[i32] {
        &self.bias
    }

    #[inline(always)]
    pub fn signed_bias(&self) -> &[i32] {
        &self.signed_bias
    }
}

#[derive(Clone, Debug)]
pub struct StarBlockWeight<const OUT: usize, const IN: usize> {
    up1: FcWeight<{ VALUE_DIM * 2 }, IN>,
    up2: FcWeight<{ VALUE_DIM * 2 }, IN>,
    down: FcWeight<OUT, OUT>,
}

#[derive(Clone, Debug)]
pub struct HeadBucket {
    policy_pwconv_l1: FcWeight<{ POLICY_DIM * 2 }, FEATURE_DIM>,
    policy_pwconv_l2:
        FcWeight<{ POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM }, { POLICY_DIM * 2 }>,
    value_corner: StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
    value_edge: StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
    value_center: StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
    value_quad: StarBlockWeight<VALUE_DIM, VALUE_DIM>,
    value_l1: FcWeight<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }>,
    value_l2: FcWeight<VALUE_DIM, VALUE_DIM>,
    value_l3: FcWeight<4, VALUE_DIM>,
    policy_output_weight: [f32; 16],
    policy_output_bias: f32,
}

/// Heap-allocated zero-initialized POD block backed by 2 MiB Transparent Huge
/// Pages on Linux (`madvise(MADV_HUGEPAGE)` before first touch), reducing L2
/// dTLB misses on the 16 MiB per-side `codebook` and `mapping_index` tables.
struct HugeBox<T> {
    ptr: std::ptr::NonNull<T>,
    layout: std::alloc::Layout,
}

// SAFETY: `HugeBox<T>` exclusively owns a `T` buffer just like `Box<T>`.
unsafe impl<T: Send> Send for HugeBox<T> {}
unsafe impl<T: Sync> Sync for HugeBox<T> {}

impl<T> HugeBox<T> {
    /// Allocate a zero-initialized `T` in a hugepage-backed buffer.
    ///
    /// # Safety
    /// All-zero bytes must be a valid bit pattern for `T`.
    unsafe fn new_zeroed() -> Self {
        let base = std::alloc::Layout::new::<T>();
        let layout = crate::fast_engine::tt::huge_page_layout(base);
        let raw = crate::fast_engine::tt::alloc_huge_zeroed(layout) as *mut T;
        let ptr = std::ptr::NonNull::new(raw).expect("failed to allocate HugeBox");
        Self { ptr, layout }
    }
}

impl<T> std::ops::Deref for HugeBox<T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> std::ops::DerefMut for HugeBox<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T> Drop for HugeBox<T> {
    fn drop(&mut self) {
        unsafe {
            std::ptr::drop_in_place(self.ptr.as_ptr());
            std::alloc::dealloc(self.ptr.as_ptr() as *mut u8, self.layout);
        }
    }
}

impl<T> std::fmt::Debug for HugeBox<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HugeBox({} bytes)", self.layout.size())
    }
}

/// Shared read-only `Mix9svq` weights (~18 MiB per side).
///
/// All tensor buffers are private and shape-validated at load/construction time
/// so safe evaluation entry points always pass exact-dimension slices to SIMD
/// kernels.
#[derive(Debug)]
pub struct Mix9svqWeight {
    pub header: StandardHeader,
    codebook: [HugeBox<[[i16; FEATURE_DIM]; CODEBOOK_SIZE]>; 2],
    mapping_index: HugeBox<[[u16; SHAPE_NUM]; 2]>,
    feature_dwconv_weight: [[i16; FEAT_DWCONV_DIM]; 9],
    feature_dwconv_bias: [i16; FEAT_DWCONV_DIM],
    bucket: HeadBucket,
}

struct ByteCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("unexpected EOF reading {n} bytes at {}", self.pos),
            ));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&mut self) -> io::Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_i32_vec(&mut self, n: usize) -> io::Result<Vec<i32>> {
        let b = self.take(n * 4)?;
        let mut out = Vec::with_capacity(n);
        for chunk in b.chunks_exact(4) {
            out.push(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(out)
    }

    fn read_i8_vec(&mut self, n: usize) -> io::Result<Vec<i8>> {
        let b = self.take(n)?;
        Ok(b.iter().map(|&v| v as i8).collect())
    }

    fn read_f32(&mut self) -> io::Result<f32> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_fc<const OUT: usize, const IN: usize>(&mut self) -> io::Result<FcWeight<OUT, IN>> {
        let weight = self.read_i8_vec(OUT * IN)?;
        let bias = self.read_i32_vec(OUT)?;
        FcWeight::new(weight, bias)
    }

    fn read_star_block<const OUT: usize, const IN: usize>(
        &mut self,
    ) -> io::Result<StarBlockWeight<OUT, IN>> {
        let up1 = self.read_fc::<{ VALUE_DIM * 2 }, IN>()?;
        let up2 = self.read_fc::<{ VALUE_DIM * 2 }, IN>()?;
        let down = self.read_fc::<OUT, OUT>()?;
        Ok(StarBlockWeight { up1, up2, down })
    }
}

struct PackedSigned10Stream {
    buffer: u64,
    bits_left: u32,
}

impl PackedSigned10Stream {
    fn new() -> Self {
        Self {
            buffer: 0,
            bits_left: 0,
        }
    }

    #[inline(always)]
    fn next(&mut self, cur: &mut ByteCursor<'_>) -> io::Result<i16> {
        const BITS: u32 = 10;
        const MASK: u64 = (1 << BITS) - 1;
        const SIGN_BIT: u16 = 1 << (BITS - 1);
        const SIGN_EXT: u16 = !(MASK as u16);

        let masked = if self.bits_left < BITS {
            let next64 = cur.read_u64()?;
            self.buffer |= next64 << self.bits_left;
            let m = (self.buffer & MASK) as u16;
            self.buffer = next64 >> (BITS - self.bits_left);
            self.bits_left = 64 - (BITS - self.bits_left);
            m
        } else {
            let m = (self.buffer & MASK) as u16;
            self.buffer >>= BITS;
            self.bits_left -= BITS;
            m
        };
        let ext = if (masked & SIGN_BIT) != 0 {
            SIGN_EXT
        } else {
            0
        };
        Ok((ext | masked) as i16)
    }
}

impl Mix9svqWeight {
    pub fn load_from_lz4_file(path: &Path, expected_rule: RuleSet) -> io::Result<Self> {
        let compressed = std::fs::read(path)?;
        let raw = decompress_lz4_frame(&compressed)?;
        Self::from_raw_bytes(&raw, expected_rule)
    }

    pub fn from_raw_bytes(raw: &[u8], expected_rule: RuleSet) -> io::Result<Self> {
        let mut cur = ByteCursor::new(raw);
        let magic = cur.read_u32()?;
        if magic != WEIGHT_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bad nnue weight magic: 0x{magic:08x}"),
            ));
        }
        let arch_hash = cur.read_u32()?;
        if arch_hash != MIX9SVQ_ARCH_HASH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "incompatible arch_hash: got 0x{arch_hash:08x}, expected 0x{MIX9SVQ_ARCH_HASH:08x}"
                ),
            ));
        }
        let rule_mask = cur.read_u32()?;
        let boardsize_mask = cur.read_u32()?;
        let desc_len = cur.read_u32()? as usize;
        if desc_len > 1_048_576 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "nnue description length exceeds 1 MiB",
            ));
        }
        let desc_bytes = cur.take(desc_len)?;
        let description = String::from_utf8_lossy(desc_bytes).into_owned();
        let header = StandardHeader {
            arch_hash,
            rule_mask,
            boardsize_mask,
            description,
        };
        if !header.supports_rule(expected_rule) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("weight does not support rule {expected_rule:?} (mask=0x{rule_mask:x})"),
            ));
        }
        if !header.supports_board_size(BOARD_SIZE) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("weight does not support board size {BOARD_SIZE}"),
            ));
        }

        // 1. Read compressed codebooks (each codebook uses a fresh 10-bit stream)
        // SAFETY: integer arrays `[i16; ...]` and `[u16; ...]` are valid when zero-initialized.
        let mut codebook: [HugeBox<[[i16; FEATURE_DIM]; CODEBOOK_SIZE]>; 2] = unsafe {
            [
                HugeBox::<[[i16; FEATURE_DIM]; CODEBOOK_SIZE]>::new_zeroed(),
                HugeBox::<[[i16; FEATURE_DIM]; CODEBOOK_SIZE]>::new_zeroed(),
            ]
        };
        for cb in &mut codebook {
            let mut stream = PackedSigned10Stream::new();
            for row in cb.iter_mut() {
                for v in row.iter_mut() {
                    *v = stream.next(&mut cur)?;
                }
            }
        }

        let mut mapping_index: HugeBox<[[u16; SHAPE_NUM]; 2]> =
            unsafe { HugeBox::<[[u16; SHAPE_NUM]; 2]>::new_zeroed() };
        for mi in mapping_index.iter_mut() {
            let bytes = cur.take(SHAPE_NUM * 2)?;
            for (dst, chunk) in mi.iter_mut().zip(bytes.chunks_exact(2)) {
                *dst = u16::from_le_bytes([chunk[0], chunk[1]]);
            }
        }

        // 2. Depthwise 3x3 conv
        let mut feature_dwconv_weight = [[0_i16; FEAT_DWCONV_DIM]; 9];
        for row in &mut feature_dwconv_weight {
            let bytes = cur.take(FEAT_DWCONV_DIM * 2)?;
            for (dst, chunk) in row.iter_mut().zip(bytes.chunks_exact(2)) {
                *dst = i16::from_le_bytes([chunk[0], chunk[1]]);
            }
        }
        let mut feature_dwconv_bias = [0_i16; FEAT_DWCONV_DIM];
        let bias_bytes = cur.take(FEAT_DWCONV_DIM * 2)?;
        for (dst, chunk) in feature_dwconv_bias
            .iter_mut()
            .zip(bias_bytes.chunks_exact(2))
        {
            *dst = i16::from_le_bytes([chunk[0], chunk[1]]);
        }

        // 3-6. HeadBucket
        let mut policy_pwconv_l1 = cur.read_fc::<{ POLICY_DIM * 2 }, FEATURE_DIM>()?;
        let mut policy_pwconv_l2 = cur
            .read_fc::<{ POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM }, { POLICY_DIM * 2 }>(
            )?;
        let mut value_corner = cur.read_star_block::<VALUE_DIM, FEATURE_DIM>()?;
        let mut value_edge = cur.read_star_block::<VALUE_DIM, FEATURE_DIM>()?;
        let mut value_center = cur.read_star_block::<VALUE_DIM, FEATURE_DIM>()?;
        let mut value_quad = cur.read_star_block::<VALUE_DIM, VALUE_DIM>()?;
        let mut value_l1 = cur.read_fc::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }>()?;
        let mut value_l2 = cur.read_fc::<VALUE_DIM, VALUE_DIM>()?;
        let value_l3 = cur.read_fc::<4, VALUE_DIM>()?;
        let mut policy_output_weight = [0.0_f32; 16];
        for v in &mut policy_output_weight {
            *v = cur.read_f32()?;
        }
        let policy_output_bias = cur.read_f32()?;
        let _pad = cur.take(44)?;

        if cur.pos != raw.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "trailing bytes after mix9svq weight: consumed {} of {}",
                    cur.pos,
                    raw.len()
                ),
            ));
        }

        // Preprocess weights into chunked layout matching Rapfi's `preprocess()`.
        preprocess_linear::<i8, { POLICY_DIM * 2 }, FEATURE_DIM, 4>(&mut policy_pwconv_l1.weight);
        preprocess_dynamic_weight_linear::<POLICY_PWCONV_DIM, POLICY_DIM, { POLICY_DIM * 2 }, 2>(
            &mut policy_pwconv_l2.weight,
            &mut policy_pwconv_l2.bias,
        );
        preprocess_linear::<
            i8,
            { POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM },
            { POLICY_DIM * 2 },
            4,
        >(&mut policy_pwconv_l2.weight);

        let preprocess_star = |b: &mut StarBlockWeight<VALUE_DIM, FEATURE_DIM>| {
            b.down.signed_bias =
                compute_signed_input_bias::<VALUE_DIM, VALUE_DIM>(&b.down.weight, &b.down.bias);
            preprocess_linear::<i8, { VALUE_DIM * 2 }, FEATURE_DIM, 4>(&mut b.up1.weight);
            preprocess_linear::<i8, { VALUE_DIM * 2 }, FEATURE_DIM, 4>(&mut b.up2.weight);
            preprocess_linear::<i8, VALUE_DIM, VALUE_DIM, 4>(&mut b.down.weight);
        };
        preprocess_star(&mut value_corner);
        preprocess_star(&mut value_edge);
        preprocess_star(&mut value_center);
        preprocess_star(&mut value_quad);

        preprocess_linear::<i8, VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, 4>(
            &mut value_l1.weight,
        );
        preprocess_linear::<i8, VALUE_DIM, VALUE_DIM, 4>(&mut value_l2.weight);
        // value_l3 has OutSize = 4, kept row-major (matches Rapfi Affine<4, 64>).

        Ok(Self {
            header,
            codebook,
            mapping_index,
            feature_dwconv_weight,
            feature_dwconv_bias,
            bucket: HeadBucket {
                policy_pwconv_l1,
                policy_pwconv_l2,
                value_corner,
                value_edge,
                value_center,
                value_quad,
                value_l1,
                value_l2,
                value_l3,
                policy_output_weight,
                policy_output_bias,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ChangeNum {
    inner: u16,
    outer: u16,
}

#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
struct ValueSum {
    global: [i32; FEATURE_DIM],
    group: [[[i32; FEATURE_DIM]; 3]; 3],
}

impl Default for ValueSum {
    fn default() -> Self {
        Self {
            global: [0; FEATURE_DIM],
            group: [[[0; FEATURE_DIM]; 3]; 3],
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
struct AlignedRow<const N: usize>([i16; N]);

impl<const N: usize> AlignedRow<N> {
    #[inline(always)]
    const fn zeroed() -> Self {
        Self([0; N])
    }
}

impl<const N: usize> std::ops::Deref for AlignedRow<N> {
    type Target = [i16; N];
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> std::ops::DerefMut for AlignedRow<N> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Version-stacked incremental accumulator for a single perspective (Black or White).
#[derive(Clone, Debug)]
pub struct Mix9svqAccumulator {
    value_sum_table: Vec<ValueSum>,
    version_change_num: Vec<ChangeNum>,
    version_inner_index: Vec<u16>,
    version_outer_index: Vec<u16>,
    index_table: Vec<[u32; 4]>,
    map_sum: Vec<AlignedRow<FEATURE_DIM>>,
    map_conv: Vec<AlignedRow<FEAT_DWCONV_DIM>>,
    current_version: i32,
    group_index: [usize; BOARD_SIZE],
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    use_avx2: bool,
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    use_avxvnni: bool,
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    use_neon: bool,
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    use_neon_dotprod: bool,
}

fn border_encoding(left: usize, right: usize) -> u32 {
    const LENGTH: usize = 11;
    const HALF: usize = 5;
    if left == HALF && right == HALF {
        0
    } else if right == HALF {
        let mut code = 2 * POWER3[LENGTH];
        let left_dist = HALF - left;
        for i in 1..left_dist {
            code += POWER3[LENGTH - i];
        }
        code
    } else {
        let mut code = POWER3[LENGTH];
        let left_dist = HALF - left;
        let right_dist = HALF - right;
        let right_twos = left_dist.min(right_dist);
        let left_twos = left_dist.min(right_dist.saturating_sub(1));
        for i in 0..right_twos {
            code += 2 * POWER3[i];
        }
        for i in 0..left_twos {
            code += 2 * POWER3[LENGTH - 1 - i];
        }
        for i in right_twos..right_dist.saturating_sub(1) {
            code += POWER3[i];
        }
        for i in left_twos..left_dist.saturating_sub(1) {
            code += POWER3[LENGTH - 1 - i];
        }
        code
    }
}

impl Mix9svqAccumulator {
    pub fn new() -> Self {
        let mut group_index = [0_usize; BOARD_SIZE];
        let size1 = (BOARD_SIZE / 3) + usize::from(BOARD_SIZE % 3 == 2);
        let size2 = (BOARD_SIZE / 3) * 2 + usize::from(BOARD_SIZE % 3 > 0);
        for (i, g) in group_index.iter_mut().enumerate() {
            *g = usize::from(i >= size1) + usize::from(i >= size2);
        }

        let mut version_change_num = vec![ChangeNum::default(); BOARD_AREA + 1];
        version_change_num[0] = ChangeNum {
            inner: BOARD_AREA as u16,
            outer: OUTER_BOARD_AREA as u16,
        };

        let mut version_inner_index = vec![0_u16; (BOARD_AREA + 1) * BOARD_AREA];
        for i in 0..BOARD_AREA {
            version_inner_index[i] = i as u16;
        }
        let mut version_outer_index = vec![0_u16; (BOARD_AREA + 2) * OUTER_BOARD_AREA];
        for i in 0..OUTER_BOARD_AREA {
            version_outer_index[i] = i as u16;
        }

        let mut index_table = vec![[0_u32; 4]; MAX_INNER_CHANGES_15];
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                let distx0 = x.min(5);
                let distx1 = (BOARD_SIZE - 1 - x).min(5);
                let disty0 = y.min(5);
                let disty1 = (BOARD_SIZE - 1 - y).min(5);
                index_table[y * BOARD_SIZE + x] = [
                    border_encoding(distx0, distx1),
                    border_encoding(disty0, disty1),
                    border_encoding(distx0.min(disty1), distx1.min(disty0)),
                    border_encoding(distx0.min(disty0), distx1.min(disty1)),
                ];
            }
        }

        Self {
            value_sum_table: vec![ValueSum::default(); BOARD_AREA + 1],
            version_change_num,
            version_inner_index,
            version_outer_index,
            index_table,
            map_sum: vec![AlignedRow::zeroed(); MAX_INNER_CHANGES_15],
            map_conv: vec![AlignedRow::zeroed(); MAX_OUTER_CHANGES_15],
            current_version: -1,
            group_index,
            use_avx2: has_avx2(),
            use_avxvnni: has_avxvnni(),
            use_neon: has_neon(),
            use_neon_dotprod: has_neon_dotprod(),
        }
    }

    pub fn clear(&mut self, w: &Mix9svqWeight) {
        if self.current_version == -1 {
            for i in 0..OUTER_BOARD_AREA {
                self.map_conv[i].0 = w.feature_dwconv_bias;
            }
            let value_sum = &mut self.value_sum_table[0];
            *value_sum = ValueSum::default();

            for y in 0..BOARD_SIZE {
                for x in 0..BOARD_SIZE {
                    let inner_idx = y * BOARD_SIZE + x;
                    let mut sum = [0_i16; FEATURE_DIM];
                    for dir in 0..4 {
                        let mapping_idx = dir / 2;
                        let shape_idx = self.index_table[inner_idx][dir] as usize;
                        let cb_idx = w.mapping_index[mapping_idx][shape_idx] as usize;
                        let cb = &w.codebook[mapping_idx][cb_idx];
                        for c in 0..FEATURE_DIM {
                            sum[c] = sum[c].wrapping_add(cb[c]);
                        }
                    }
                    self.map_sum[inner_idx].0 = sum;

                    // Depthwise 3x3 conv contribution for channels 0..32
                    let mut conv_in = [0_i16; FEAT_DWCONV_DIM];
                    for c in 0..FEAT_DWCONV_DIM {
                        conv_in[c] = sum[c].max(0).wrapping_shl(2);
                    }
                    for dy in 0..=2 {
                        let yi = y + dy;
                        for dx in 0..=2 {
                            let xi = x + dx;
                            let outer_idx = xi + yi * OUTER_BOARD_SIZE;
                            let cw = &w.feature_dwconv_weight[8 - dy * 3 - dx];
                            let dst = &mut self.map_conv[outer_idx];
                            for c in 0..FEAT_DWCONV_DIM {
                                dst[c] = dst[c].wrapping_add(mulhi_i16(cw[c], conv_in[c]));
                            }
                        }
                    }

                    // Channels 32..64 directly into value_sum
                    let gi = self.group_index[y];
                    let gj = self.group_index[x];
                    for c in FEAT_DWCONV_DIM..FEATURE_DIM {
                        let v = sum[c].max(0) as i32;
                        value_sum.global[c] += v;
                        value_sum.group[gi][gj][c] += v;
                    }
                }
            }

            // Add all dwconv channels 0..32 into value_sum
            for y in 0..BOARD_SIZE {
                let gi = self.group_index[y];
                for x in 0..BOARD_SIZE {
                    let gj = self.group_index[x];
                    let outer_idx = (y + 1) * OUTER_BOARD_SIZE + (x + 1);
                    let mc = &self.map_conv[outer_idx];
                    for c in 0..FEAT_DWCONV_DIM {
                        let v = mc[c].max(0) as i32;
                        value_sum.global[c] += v;
                        value_sum.group[gi][gj][c] += v;
                    }
                }
            }
        }
        self.current_version = 0;
    }

    #[inline(always)]
    pub fn undo(&mut self) {
        debug_assert!(self.current_version > 0);
        self.current_version -= 1;
    }

    /// Incrementally apply a stone of `piece_color` (`0 = self/black`, `1 = opp/white`)
    /// at `(x, y)`.
    pub fn play_move(&mut self, w: &Mix9svqWeight, piece_color: u8, x: usize, y: usize) {
        debug_assert!(piece_color <= 1);
        #[cfg(target_arch = "x86_64")]
        if self.use_avx2 {
            // SAFETY: `self.use_avx2` verified AVX2 support at construction.
            unsafe { self.play_move_avx2(w, piece_color, x, y) };
            return;
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon {
            // SAFETY: `self.use_neon` verified NEON support at construction.
            unsafe { self.play_move_neon(w, piece_color, x, y) };
            return;
        }
        self.play_move_scalar(w, piece_color, x, y);
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn play_move_avx2(&mut self, w: &Mix9svqWeight, piece_color: u8, x: usize, y: usize) {
        use std::arch::x86_64::*;

        #[derive(Clone, Copy)]
        struct OnePointChange {
            x: u8,
            y: u8,
            mapping_idx: u8,
            old_map_idx: u16,
            new_map_idx: u16,
            old_cb_idx: u16,
            new_cb_idx: u16,
        }

        #[repr(align(32))]
        struct AlignedVSum([[[i32; FEATURE_DIM]; 3]; 3]);

        let ver = self.current_version as usize;
        let inner_prev_base = ver * BOARD_AREA;
        let inner_next_base = inner_prev_base + BOARD_AREA;
        let outer_prev_base = ver * OUTER_BOARD_AREA;
        let outer_next_base = outer_prev_base + OUTER_BOARD_AREA;
        let change_num = *self.version_change_num.get_unchecked(ver);

        let vi_ptr = self.version_inner_index.as_mut_ptr();
        std::ptr::copy_nonoverlapping(
            vi_ptr.add(inner_prev_base),
            vi_ptr.add(inner_next_base),
            BOARD_AREA,
        );
        let vo_ptr = self.version_outer_index.as_mut_ptr();
        std::ptr::copy_nonoverlapping(
            vo_ptr.add(outer_prev_base),
            vo_ptr.add(outer_next_base),
            OUTER_BOARD_AREA,
        );

        let zero = _mm256_setzero_si256();
        let x0 = (x + 1).saturating_sub(6).max(1);
        let y0 = (y + 1).saturating_sub(6).max(1);
        let x1 = (x + 7).min(BOARD_SIZE);
        let y1 = (y + 7).min(BOARD_SIZE);

        let map_conv_ptr = self.map_conv.as_mut_ptr();
        let mut new_map_conv_idx = change_num.outer as usize;
        let x_plus_1 = x + 1;
        let y_plus_1 = y + 1;
        let x_lo_bit = x0 + 6 - x_plus_1;
        let x_hi_bit = x1 + 6 - x_plus_1;
        let clip_mask = ((1_u32 << (x_hi_bit - x_lo_bit + 1)) - 1) << x_lo_bit;
        for yi in y0..=y1 {
            let mut bits = (*STAR_CONV_MASK.get_unchecked(yi + 6 - y_plus_1) as u32) & clip_mask;
            let outer_row = outer_next_base + yi * OUTER_BOARD_SIZE;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let xi = x_plus_1 + bit - 6;
                *vo_ptr.add(outer_row + xi) = new_map_conv_idx as u16;
                let dst = map_conv_ptr.add(new_map_conv_idx) as *mut __m256i;
                _mm256_store_si256(dst, zero);
                _mm256_store_si256(dst.add(1), zero);
                new_map_conv_idx += 1;
            }
        }

        #[derive(Clone, Copy)]
        struct PendingShape {
            x: u8,
            y: u8,
            mapping_idx: u8,
            old_map_idx: u16,
            new_map_idx: u16,
            old_shape: u32,
            new_shape: u32,
        }

        let mut pending = [std::mem::MaybeUninit::<PendingShape>::uninit(); 40];
        let mut pending_count = 0_usize;
        let mut changes = [std::mem::MaybeUninit::<OnePointChange>::uninit(); 40];
        let mut change_count = 0_usize;
        let d_pow3 = (piece_color as u32) + 1;
        let idx_tbl_ptr = self.index_table.as_mut_ptr();
        let map_idx_ptrs = [w.mapping_index[0].as_ptr(), w.mapping_index[1].as_ptr()];
        let mut new_map_idx = change_num.inner as usize;
        const BS_SUB1: isize = (BOARD_SIZE - 1) as isize;
        let x_i = x as isize;
        let y_i = y as isize;

        let map_sum_ptr = self.map_sum.as_mut_ptr();
        let cb_ptrs = [w.codebook[0].as_ptr(), w.codebook[1].as_ptr()];
        let dw_weight_ptr = w.feature_dwconv_weight.as_ptr();

        // Fuse the 4 `dist == 0` updates at the newly placed cell `(x, y)` into
        // a single telescoping update (41 changes instead of 44, 100% bit-exact).
        let center_slot = vi_ptr.add(inner_next_base + y * BOARD_SIZE + x);
        let center_old_map_idx = *center_slot as usize;
        let center_new_map_idx = new_map_idx;
        let mut center_shapes = *idx_tbl_ptr.add(center_old_map_idx);
        let mut center_old_shapes = [0_u32; 4];
        let center_delta = d_pow3 * POWER3[5];
        for dir in 0..4 {
            let map_ptr = map_idx_ptrs[dir >> 1];
            let old_shape = center_shapes[dir];
            let new_shape = old_shape + center_delta;
            debug_assert!((new_shape as usize) < SHAPE_NUM);
            center_old_shapes[dir] = old_shape;
            center_shapes[dir] = new_shape;
            _mm_prefetch(map_ptr.add(old_shape as usize) as *const i8, _MM_HINT_T0);
            _mm_prefetch(map_ptr.add(new_shape as usize) as *const i8, _MM_HINT_T0);
        }
        *idx_tbl_ptr.add(center_new_map_idx) = center_shapes;
        *center_slot = center_new_map_idx as u16;
        new_map_idx += 1;

        // Pass 1: update index_table for all ray cells and issue parallel prefetches
        // into the 1.77 MiB `mapping_index` table.
        for dir in 0..4 {
            let mapping_idx = dir >> 1;
            let map_ptr = map_idx_ptrs[mapping_idx];
            let dx_dir = DX[dir];
            let dy_dir = DY[dir];
            for dist in [-5_isize, -4, -3, -2, -1, 1, 2, 3, 4, 5] {
                let xi = x_i + dist * dx_dir;
                let yi = y_i + dist * dy_dir;
                if (xi | (BS_SUB1 - xi) | yi | (BS_SUB1 - yi)) < 0 {
                    continue;
                }
                let xu = xi as usize;
                let yu = yi as usize;
                let inner_slot = vi_ptr.add(inner_next_base + yu * BOARD_SIZE + xu);
                let old_map_idx = *inner_slot as usize;
                let mut next_shapes = *idx_tbl_ptr.add(old_map_idx);
                let old_shape = next_shapes[dir];
                let new_shape = old_shape + d_pow3 * POWER3[(dist + 5) as usize];
                debug_assert!((new_shape as usize) < SHAPE_NUM);
                next_shapes[dir] = new_shape;
                *idx_tbl_ptr.add(new_map_idx) = next_shapes;
                *inner_slot = new_map_idx as u16;

                _mm_prefetch(map_ptr.add(old_shape as usize) as *const i8, _MM_HINT_T0);
                _mm_prefetch(map_ptr.add(new_shape as usize) as *const i8, _MM_HINT_T0);

                pending.get_unchecked_mut(pending_count).write(PendingShape {
                    x: xu as u8,
                    y: yu as u8,
                    mapping_idx: mapping_idx as u8,
                    old_map_idx: old_map_idx as u16,
                    new_map_idx: new_map_idx as u16,
                    old_shape,
                    new_shape,
                });
                pending_count += 1;
                new_map_idx += 1;
            }
        }

        // Resolve center cell codebook indices and prefetch its codebook lines.
        let mut center_old_cb = [0_u16; 4];
        let mut center_new_cb = [0_u16; 4];
        for dir in 0..4 {
            let map_ptr = map_idx_ptrs[dir >> 1];
            let ocb = *map_ptr.add(center_old_shapes[dir] as usize);
            let ncb = *map_ptr.add(center_shapes[dir] as usize);
            center_old_cb[dir] = ocb;
            center_new_cb[dir] = ncb;
            if ocb != ncb {
                let cb_base = cb_ptrs[dir >> 1];
                let p_old = cb_base.add(ocb as usize) as *const i8;
                let p_new = cb_base.add(ncb as usize) as *const i8;
                _mm_prefetch(p_old, _MM_HINT_T0);
                _mm_prefetch(p_old.add(64), _MM_HINT_T0);
                _mm_prefetch(p_new, _MM_HINT_T0);
                _mm_prefetch(p_new.add(64), _MM_HINT_T0);
            }
        }

        // Pass 2: read prefetched `mapping_index`, skip identical VQ clusters, and
        // prime codebook prefetches for the first 6 non-zero changes.
        for i in 0..pending_count {
            let p = pending.get_unchecked(i).assume_init();
            let map_ptr = map_idx_ptrs[p.mapping_idx as usize];
            let old_cb_idx = *map_ptr.add(p.old_shape as usize);
            let new_cb_idx = *map_ptr.add(p.new_shape as usize);
            if old_cb_idx == new_cb_idx {
                let old_s = map_sum_ptr.add(p.old_map_idx as usize) as *const __m256i;
                let new_s = map_sum_ptr.add(p.new_map_idx as usize) as *mut __m256i;
                _mm256_store_si256(new_s, _mm256_load_si256(old_s));
                _mm256_store_si256(new_s.add(1), _mm256_load_si256(old_s.add(1)));
                _mm256_store_si256(new_s.add(2), _mm256_load_si256(old_s.add(2)));
                _mm256_store_si256(new_s.add(3), _mm256_load_si256(old_s.add(3)));
                continue;
            }
            if change_count < 6 {
                let cb_base = cb_ptrs[p.mapping_idx as usize];
                let p_old = cb_base.add(old_cb_idx as usize) as *const i8;
                let p_new = cb_base.add(new_cb_idx as usize) as *const i8;
                _mm_prefetch(p_old, _MM_HINT_T0);
                _mm_prefetch(p_old.add(64), _MM_HINT_T0);
                _mm_prefetch(p_new, _MM_HINT_T0);
                _mm_prefetch(p_new.add(64), _MM_HINT_T0);
            }
            changes
                .get_unchecked_mut(change_count)
                .write(OnePointChange {
                    x: p.x,
                    y: p.y,
                    mapping_idx: p.mapping_idx,
                    old_map_idx: p.old_map_idx,
                    new_map_idx: p.new_map_idx,
                    old_cb_idx,
                    new_cb_idx,
                });
            change_count += 1;
        }

        // Zero aligned vsum_group via inline AVX2 stores (no glibc memset).
        let mut vsum_aligned = std::mem::MaybeUninit::<AlignedVSum>::uninit();
        let vsum_raw = vsum_aligned.as_mut_ptr() as *mut __m256i;
        for i in 0..(9 * FEATURE_DIM / 8) {
            _mm256_store_si256(vsum_raw.add(i), zero);
        }
        let vsum_group = &mut (*vsum_aligned.as_mut_ptr()).0;

        // Apply fused 4-direction update for center cell `(x, y)`.
        {
            let old_s_ptr = map_sum_ptr.add(center_old_map_idx) as *const __m256i;
            let new_s_ptr = map_sum_ptr.add(center_new_map_idx) as *mut __m256i;
            let old_s0 = _mm256_load_si256(old_s_ptr);
            let old_s1 = _mm256_load_si256(old_s_ptr.add(1));
            let old_s2 = _mm256_load_si256(old_s_ptr.add(2));
            let old_s3 = _mm256_load_si256(old_s_ptr.add(3));
            let (mut new_s0, mut new_s1, mut new_s2, mut new_s3) = (old_s0, old_s1, old_s2, old_s3);
            for dir in 0..4 {
                if center_old_cb[dir] == center_new_cb[dir] {
                    continue;
                }
                let cb_base = cb_ptrs[dir >> 1];
                let old_cb_ptr = cb_base.add(center_old_cb[dir] as usize) as *const __m256i;
                let new_cb_ptr = cb_base.add(center_new_cb[dir] as usize) as *const __m256i;
                new_s0 = _mm256_add_epi16(
                    _mm256_sub_epi16(new_s0, _mm256_load_si256(old_cb_ptr)),
                    _mm256_load_si256(new_cb_ptr),
                );
                new_s1 = _mm256_add_epi16(
                    _mm256_sub_epi16(new_s1, _mm256_load_si256(old_cb_ptr.add(1))),
                    _mm256_load_si256(new_cb_ptr.add(1)),
                );
                new_s2 = _mm256_add_epi16(
                    _mm256_sub_epi16(new_s2, _mm256_load_si256(old_cb_ptr.add(2))),
                    _mm256_load_si256(new_cb_ptr.add(2)),
                );
                new_s3 = _mm256_add_epi16(
                    _mm256_sub_epi16(new_s3, _mm256_load_si256(old_cb_ptr.add(3))),
                    _mm256_load_si256(new_cb_ptr.add(3)),
                );
            }
            _mm256_store_si256(new_s_ptr, new_s0);
            _mm256_store_si256(new_s_ptr.add(1), new_s1);
            _mm256_store_si256(new_s_ptr.add(2), new_s2);
            _mm256_store_si256(new_s_ptr.add(3), new_s3);

            let old_c0 = _mm256_slli_epi16(_mm256_max_epi16(old_s0, zero), 2);
            let old_c1 = _mm256_slli_epi16(_mm256_max_epi16(old_s1, zero), 2);
            let new_c0 = _mm256_slli_epi16(_mm256_max_epi16(new_s0, zero), 2);
            let new_c1 = _mm256_slli_epi16(_mm256_max_epi16(new_s1, zero), 2);

            let mut outer_row = outer_next_base + y * OUTER_BOARD_SIZE + x;
            let mut k = 0;
            for _dy in 0..=2 {
                for dx in 0..=2 {
                    let mc_idx = *vo_ptr.add(outer_row + dx) as usize;
                    let dst = map_conv_ptr.add(mc_idx) as *mut __m256i;
                    let w_row = (*dw_weight_ptr.add(8 - k)).as_ptr() as *const __m256i;
                    let dw0 = _mm256_loadu_si256(w_row);
                    let dw1 = _mm256_loadu_si256(w_row.add(1));
                    let d0 = _mm256_sub_epi16(
                        _mm256_mulhi_epi16(dw0, new_c0),
                        _mm256_mulhi_epi16(dw0, old_c0),
                    );
                    let d1 = _mm256_sub_epi16(
                        _mm256_mulhi_epi16(dw1, new_c1),
                        _mm256_mulhi_epi16(dw1, old_c1),
                    );
                    _mm256_store_si256(dst, _mm256_add_epi16(_mm256_load_si256(dst), d0));
                    _mm256_store_si256(
                        dst.add(1),
                        _mm256_add_epi16(_mm256_load_si256(dst.add(1)), d1),
                    );
                    k += 1;
                }
                outer_row += OUTER_BOARD_SIZE;
            }

            let gi = *self.group_index.get_unchecked(y);
            let gj = *self.group_index.get_unchecked(x);
            let vg_ptr = vsum_group
                .get_unchecked_mut(gi)
                .get_unchecked_mut(gj)
                .as_mut_ptr() as *mut __m256i;
            let diff2 = _mm256_sub_epi16(
                _mm256_max_epi16(new_s2, zero),
                _mm256_max_epi16(old_s2, zero),
            );
            let diff3 = _mm256_sub_epi16(
                _mm256_max_epi16(new_s3, zero),
                _mm256_max_epi16(old_s3, zero),
            );
            let lo2 = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff2));
            let hi2 = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff2, 1));
            let lo3 = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff3));
            let hi3 = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff3, 1));
            _mm256_store_si256(
                vg_ptr.add(4),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(4)), lo2),
            );
            _mm256_store_si256(
                vg_ptr.add(5),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(5)), hi2),
            );
            _mm256_store_si256(
                vg_ptr.add(6),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(6)), lo3),
            );
            _mm256_store_si256(
                vg_ptr.add(7),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(7)), hi3),
            );
        }

        for i in 0..change_count {
            let c = changes.get_unchecked(i).assume_init();
            if i + 6 < change_count {
                let cnext = changes.get_unchecked(i + 6).assume_init();
                let cb_next = cb_ptrs[cnext.mapping_idx as usize];
                let p_old = cb_next.add(cnext.old_cb_idx as usize) as *const i8;
                let p_new = cb_next.add(cnext.new_cb_idx as usize) as *const i8;
                _mm_prefetch(p_old, _MM_HINT_T0);
                _mm_prefetch(p_old.add(64), _MM_HINT_T0);
                _mm_prefetch(p_new, _MM_HINT_T0);
                _mm_prefetch(p_new.add(64), _MM_HINT_T0);
            }

            let cb_base = cb_ptrs[c.mapping_idx as usize];
            let old_cb_ptr = cb_base.add(c.old_cb_idx as usize) as *const __m256i;
            let new_cb_ptr = cb_base.add(c.new_cb_idx as usize) as *const __m256i;
            let old_s_ptr = map_sum_ptr.add(c.old_map_idx as usize) as *const __m256i;
            let new_s_ptr = map_sum_ptr.add(c.new_map_idx as usize) as *mut __m256i;

            let old_s0 = _mm256_load_si256(old_s_ptr);
            let old_s1 = _mm256_load_si256(old_s_ptr.add(1));
            let old_s2 = _mm256_load_si256(old_s_ptr.add(2));
            let old_s3 = _mm256_load_si256(old_s_ptr.add(3));

            let new_s0 = _mm256_add_epi16(
                _mm256_sub_epi16(old_s0, _mm256_load_si256(old_cb_ptr)),
                _mm256_load_si256(new_cb_ptr),
            );
            let new_s1 = _mm256_add_epi16(
                _mm256_sub_epi16(old_s1, _mm256_load_si256(old_cb_ptr.add(1))),
                _mm256_load_si256(new_cb_ptr.add(1)),
            );
            let new_s2 = _mm256_add_epi16(
                _mm256_sub_epi16(old_s2, _mm256_load_si256(old_cb_ptr.add(2))),
                _mm256_load_si256(new_cb_ptr.add(2)),
            );
            let new_s3 = _mm256_add_epi16(
                _mm256_sub_epi16(old_s3, _mm256_load_si256(old_cb_ptr.add(3))),
                _mm256_load_si256(new_cb_ptr.add(3)),
            );

            _mm256_store_si256(new_s_ptr, new_s0);
            _mm256_store_si256(new_s_ptr.add(1), new_s1);
            _mm256_store_si256(new_s_ptr.add(2), new_s2);
            _mm256_store_si256(new_s_ptr.add(3), new_s3);

            let old_c0 = _mm256_slli_epi16(_mm256_max_epi16(old_s0, zero), 2);
            let old_c1 = _mm256_slli_epi16(_mm256_max_epi16(old_s1, zero), 2);
            let new_c0 = _mm256_slli_epi16(_mm256_max_epi16(new_s0, zero), 2);
            let new_c1 = _mm256_slli_epi16(_mm256_max_epi16(new_s1, zero), 2);

            let cy = c.y as usize;
            let cx = c.x as usize;
            let mut outer_row = outer_next_base + cy * OUTER_BOARD_SIZE + cx;
            let mut k = 0;
            for _dy in 0..=2 {
                for dx in 0..=2 {
                    let mc_idx = *vo_ptr.add(outer_row + dx) as usize;
                    let dst = map_conv_ptr.add(mc_idx) as *mut __m256i;
                    let w_row = (*dw_weight_ptr.add(8 - k)).as_ptr() as *const __m256i;
                    let dw0 = _mm256_loadu_si256(w_row);
                    let dw1 = _mm256_loadu_si256(w_row.add(1));
                    let d0 = _mm256_sub_epi16(
                        _mm256_mulhi_epi16(dw0, new_c0),
                        _mm256_mulhi_epi16(dw0, old_c0),
                    );
                    let d1 = _mm256_sub_epi16(
                        _mm256_mulhi_epi16(dw1, new_c1),
                        _mm256_mulhi_epi16(dw1, old_c1),
                    );
                    _mm256_store_si256(dst, _mm256_add_epi16(_mm256_load_si256(dst), d0));
                    _mm256_store_si256(
                        dst.add(1),
                        _mm256_add_epi16(_mm256_load_si256(dst.add(1)), d1),
                    );
                    k += 1;
                }
                outer_row += OUTER_BOARD_SIZE;
            }

            let gi = *self.group_index.get_unchecked(cy);
            let gj = *self.group_index.get_unchecked(cx);
            let vg_ptr = vsum_group
                .get_unchecked_mut(gi)
                .get_unchecked_mut(gj)
                .as_mut_ptr() as *mut __m256i;
            let diff2 = _mm256_sub_epi16(
                _mm256_max_epi16(new_s2, zero),
                _mm256_max_epi16(old_s2, zero),
            );
            let diff3 = _mm256_sub_epi16(
                _mm256_max_epi16(new_s3, zero),
                _mm256_max_epi16(old_s3, zero),
            );
            let lo2 = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff2));
            let hi2 = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff2, 1));
            let lo3 = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff3));
            let hi3 = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff3, 1));
            _mm256_store_si256(
                vg_ptr.add(4),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(4)), lo2),
            );
            _mm256_store_si256(
                vg_ptr.add(5),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(5)), hi2),
            );
            _mm256_store_si256(
                vg_ptr.add(6),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(6)), lo3),
            );
            _mm256_store_si256(
                vg_ptr.add(7),
                _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(7)), hi3),
            );
        }

        // Commit dwconv deltas into new map_conv cells & accumulate channels 0..32
        let mut commit_idx = change_num.outer as usize;
        for yi in y0..=y1 {
            let mut bits = (*STAR_CONV_MASK.get_unchecked(yi + 6 - y_plus_1) as u32) & clip_mask;
            let gi = *self.group_index.get_unchecked(yi - 1);
            let prev_row = outer_prev_base + yi * OUTER_BOARD_SIZE;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let xi = x_plus_1 + bit - 6;
                let gj = *self.group_index.get_unchecked(xi - 1);
                let old_conv_idx = *vo_ptr.add(prev_row + xi) as usize;
                let old_ptr = map_conv_ptr.add(old_conv_idx) as *const __m256i;
                let new_ptr = map_conv_ptr.add(commit_idx) as *mut __m256i;
                let vg_ptr = vsum_group
                    .get_unchecked_mut(gi)
                    .get_unchecked_mut(gj)
                    .as_mut_ptr() as *mut __m256i;

                let old_v0 = _mm256_load_si256(old_ptr);
                let new_v0 = _mm256_add_epi16(old_v0, _mm256_load_si256(new_ptr));
                _mm256_store_si256(new_ptr, new_v0);
                let diff0 = _mm256_sub_epi16(
                    _mm256_max_epi16(new_v0, zero),
                    _mm256_max_epi16(old_v0, zero),
                );
                let lo0 = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff0));
                let hi0 = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff0, 1));
                _mm256_store_si256(vg_ptr, _mm256_add_epi32(_mm256_load_si256(vg_ptr), lo0));
                _mm256_store_si256(
                    vg_ptr.add(1),
                    _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(1)), hi0),
                );

                let old_v1 = _mm256_load_si256(old_ptr.add(1));
                let new_v1 = _mm256_add_epi16(old_v1, _mm256_load_si256(new_ptr.add(1)));
                _mm256_store_si256(new_ptr.add(1), new_v1);
                let diff1 = _mm256_sub_epi16(
                    _mm256_max_epi16(new_v1, zero),
                    _mm256_max_epi16(old_v1, zero),
                );
                let lo1 = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff1));
                let hi1 = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff1, 1));
                _mm256_store_si256(
                    vg_ptr.add(2),
                    _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(2)), lo1),
                );
                _mm256_store_si256(
                    vg_ptr.add(3),
                    _mm256_add_epi32(_mm256_load_si256(vg_ptr.add(3)), hi1),
                );

                commit_idx += 1;
            }
        }

        self.current_version += 1;
        let next_ver = self.current_version as usize;
        *self.version_change_num.get_unchecked_mut(next_ver) = ChangeNum {
            inner: new_map_idx as u16,
            outer: commit_idx as u16,
        };

        let vs_ptr = self.value_sum_table.as_mut_ptr();
        let prev_vs = &*vs_ptr.add(ver);
        let next_vs = &mut *vs_ptr.add(next_ver);
        for b in 0..8 {
            let off = b * 8;
            let mut g_acc = zero;
            for i in 0..3 {
                for j in 0..3 {
                    let old_v =
                        _mm256_loadu_si256(prev_vs.group[i][j].as_ptr().add(off) as *const __m256i);
                    let del_v =
                        _mm256_load_si256(vsum_group[i][j].as_ptr().add(off) as *const __m256i);
                    let new_v = _mm256_add_epi32(old_v, del_v);
                    _mm256_storeu_si256(
                        next_vs.group[i][j].as_mut_ptr().add(off) as *mut __m256i,
                        new_v,
                    );
                    g_acc = _mm256_add_epi32(g_acc, new_v);
                }
            }
            _mm256_storeu_si256(next_vs.global.as_mut_ptr().add(off) as *mut __m256i, g_acc);
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn play_move_neon(&mut self, w: &Mix9svqWeight, piece_color: u8, x: usize, y: usize) {
        self.play_move_scalar_or_neon(w, piece_color, x, y, true);
    }

    fn play_move_scalar(&mut self, w: &Mix9svqWeight, piece_color: u8, x: usize, y: usize) {
        self.play_move_scalar_or_neon(w, piece_color, x, y, false);
    }

    fn play_move_scalar_or_neon(
        &mut self,
        w: &Mix9svqWeight,
        piece_color: u8,
        x: usize,
        y: usize,
        _use_neon: bool,
    ) {
        let ver = self.current_version as usize;
        let inner_prev_base = ver * BOARD_AREA;
        let inner_next_base = inner_prev_base + BOARD_AREA;
        let outer_prev_base = ver * OUTER_BOARD_AREA;
        let outer_next_base = outer_prev_base + OUTER_BOARD_AREA;
        let change_num = self.version_change_num[ver];

        self.version_inner_index.copy_within(
            inner_prev_base..inner_prev_base + BOARD_AREA,
            inner_next_base,
        );
        self.version_outer_index.copy_within(
            outer_prev_base..outer_prev_base + OUTER_BOARD_AREA,
            outer_next_base,
        );

        // Allocate fresh delta rows in map_conv for the 13x13 affected window (1..=15)
        let x0 = (x + 1).saturating_sub(6).max(1);
        let y0 = (y + 1).saturating_sub(6).max(1);
        let x1 = (x + 7).min(BOARD_SIZE);
        let y1 = (y + 7).min(BOARD_SIZE);
        let mut new_map_conv_idx = change_num.outer as usize;
        for yi in y0..=y1 {
            let outer_row = yi * OUTER_BOARD_SIZE;
            for xi in x0..=x1 {
                let outer_idx = outer_row + xi;
                self.version_outer_index[outer_next_base + outer_idx] = new_map_conv_idx as u16;
                self.map_conv[new_map_conv_idx] = AlignedRow::zeroed();
                new_map_conv_idx += 1;
            }
        }

        #[derive(Clone, Copy, Default)]
        struct OnePointChange {
            x: usize,
            y: usize,
            mapping_idx: usize,
            old_map_idx: usize,
            new_map_idx: usize,
            old_cb_idx: usize,
            new_cb_idx: usize,
        }
        let mut changes = [OnePointChange::default(); 44];
        let mut change_count = 0_usize;
        let d_pow3 = (piece_color as u32) + 1;

        let mut new_map_idx = change_num.inner as usize;
        for dir in 0..4 {
            let mapping_idx = dir / 2;
            for dist in -5_isize..=5_isize {
                let xi = x as isize + dist * DX[dir];
                let yi = y as isize + dist * DY[dir];
                if xi < 0 || xi >= BOARD_SIZE as isize || yi < 0 || yi >= BOARD_SIZE as isize {
                    continue;
                }
                let xu = xi as usize;
                let yu = yi as usize;
                let inner_idx = yu * BOARD_SIZE + xu;
                let old_map_idx = self.version_inner_index[inner_next_base + inner_idx] as usize;
                let old_shape = self.index_table[old_map_idx][dir];
                let mut next_shapes = self.index_table[old_map_idx];
                let new_shape = old_shape + d_pow3 * POWER3[(dist + 5) as usize];
                debug_assert!((new_shape as usize) < SHAPE_NUM);
                next_shapes[dir] = new_shape;
                self.index_table[new_map_idx] = next_shapes;

                let old_cb_idx = w.mapping_index[mapping_idx][old_shape as usize] as usize;
                let new_cb_idx = w.mapping_index[mapping_idx][new_shape as usize] as usize;
                self.version_inner_index[inner_next_base + inner_idx] = new_map_idx as u16;

                changes[change_count] = OnePointChange {
                    x: xu,
                    y: yu,
                    mapping_idx,
                    old_map_idx,
                    new_map_idx,
                    old_cb_idx,
                    new_cb_idx,
                };
                change_count += 1;
                new_map_idx += 1;
            }
        }

        let mut vsum_group = [[[0_i32; FEATURE_DIM]; 3]; 3];

        for i in 0..change_count {
            let c = changes[i];
            #[cfg(target_arch = "x86_64")]
            if self.use_avx2 && i + 1 < change_count {
                let cnext = changes[i + 1];
                unsafe {
                    let p_old =
                        w.codebook[cnext.mapping_idx][cnext.old_cb_idx].as_ptr() as *const i8;
                    let p_new =
                        w.codebook[cnext.mapping_idx][cnext.new_cb_idx].as_ptr() as *const i8;
                    std::arch::x86_64::_mm_prefetch(p_old, std::arch::x86_64::_MM_HINT_T0);
                    std::arch::x86_64::_mm_prefetch(p_old.add(64), std::arch::x86_64::_MM_HINT_T0);
                    std::arch::x86_64::_mm_prefetch(p_new, std::arch::x86_64::_MM_HINT_T0);
                    std::arch::x86_64::_mm_prefetch(p_new.add(64), std::arch::x86_64::_MM_HINT_T0);
                }
            }
            let gi = self.group_index[c.y];
            let gj = self.group_index[c.x];

            let mut outer_indices = [0_u16; 9];
            let mut k = 0;
            for dy in 0..=2 {
                let row_base = (c.y + dy) * OUTER_BOARD_SIZE + c.x;
                for dx in 0..=2 {
                    outer_indices[k] = self.version_outer_index[outer_next_base + row_base + dx];
                    k += 1;
                }
            }

            #[cfg(target_arch = "x86_64")]
            if self.use_avx2 {
                // SAFETY: `c.old_map_idx < c.new_map_idx` always holds since
                // `new_map_idx` is strictly monotonically appended, so the two
                // slices in `self.map_sum` never alias; `AlignedRow<32>` has exact
                // size 64 bytes matching `[i16; 32]`.
                unsafe {
                    let old_ptr = &(*self.map_sum.as_ptr().add(c.old_map_idx)).0;
                    let new_ptr = &mut (*self.map_sum.as_mut_ptr().add(c.new_map_idx)).0;
                    let map_conv_raw = std::slice::from_raw_parts_mut(
                        self.map_conv.as_mut_ptr() as *mut [i16; FEAT_DWCONV_DIM],
                        self.map_conv.len(),
                    );
                    update_point_avx2(
                        &w.codebook[c.mapping_idx][c.old_cb_idx],
                        &w.codebook[c.mapping_idx][c.new_cb_idx],
                        old_ptr,
                        new_ptr,
                        &w.feature_dwconv_weight,
                        map_conv_raw,
                        &outer_indices,
                        &mut vsum_group[gi][gj],
                    );
                }
                continue;
            }
            #[cfg(target_arch = "aarch64")]
            if self.use_neon {
                unsafe {
                    let old_ptr = &(*self.map_sum.as_ptr().add(c.old_map_idx)).0;
                    let new_ptr = &mut (*self.map_sum.as_mut_ptr().add(c.new_map_idx)).0;
                    let map_conv_raw = std::slice::from_raw_parts_mut(
                        self.map_conv.as_mut_ptr() as *mut [i16; FEAT_DWCONV_DIM],
                        self.map_conv.len(),
                    );
                    update_point_neon(
                        &w.codebook[c.mapping_idx][c.old_cb_idx],
                        &w.codebook[c.mapping_idx][c.new_cb_idx],
                        old_ptr,
                        new_ptr,
                        &w.feature_dwconv_weight,
                        map_conv_raw,
                        &outer_indices,
                        &mut vsum_group[gi][gj],
                    );
                }
                continue;
            }

            let old_cb = &w.codebook[c.mapping_idx][c.old_cb_idx];
            let new_cb = &w.codebook[c.mapping_idx][c.new_cb_idx];
            let old_sum = self.map_sum[c.old_map_idx].0;
            let mut new_sum = [0_i16; FEATURE_DIM];
            let mut old_f = [0_i16; FEATURE_DIM];
            let mut new_f = [0_i16; FEATURE_DIM];
            for ch in 0..FEATURE_DIM {
                let ns = old_sum[ch]
                    .wrapping_sub(old_cb[ch])
                    .wrapping_add(new_cb[ch]);
                new_sum[ch] = ns;
                old_f[ch] = old_sum[ch].max(0);
                new_f[ch] = ns.max(0);
            }
            self.map_sum[c.new_map_idx].0 = new_sum;

            let mut old_c = [0_i16; FEAT_DWCONV_DIM];
            let mut new_c = [0_i16; FEAT_DWCONV_DIM];
            for ch in 0..FEAT_DWCONV_DIM {
                old_c[ch] = old_f[ch].wrapping_shl(2);
                new_c[ch] = new_f[ch].wrapping_shl(2);
            }
            for k in 0..9 {
                let cw = &w.feature_dwconv_weight[8 - k];
                let dst = &mut self.map_conv[outer_indices[k] as usize];
                for ch in 0..FEAT_DWCONV_DIM {
                    let delta =
                        mulhi_i16(cw[ch], new_c[ch]).wrapping_sub(mulhi_i16(cw[ch], old_c[ch]));
                    dst[ch] = dst[ch].wrapping_add(delta);
                }
            }
            for ch in FEAT_DWCONV_DIM..FEATURE_DIM {
                vsum_group[gi][gj][ch] += (new_f[ch] as i32) - (old_f[ch] as i32);
            }
        }

        // Commit dwconv deltas into new map_conv cells & accumulate channels 0..32
        let mut commit_idx = change_num.outer as usize;
        for yi in y0..=y1 {
            let gi = self.group_index[yi - 1];
            let outer_row = yi * OUTER_BOARD_SIZE;
            for xi in x0..=x1 {
                let gj = self.group_index[xi - 1];
                let old_conv_idx =
                    self.version_outer_index[outer_prev_base + outer_row + xi] as usize;

                #[cfg(target_arch = "x86_64")]
                if self.use_avx2 {
                    unsafe {
                        let old_ptr = &(*self.map_conv.as_ptr().add(old_conv_idx)).0;
                        let new_ptr = &mut (*self.map_conv.as_mut_ptr().add(commit_idx)).0;
                        commit_conv_cell_avx2(old_ptr, new_ptr, &mut vsum_group[gi][gj]);
                    }
                    commit_idx += 1;
                    continue;
                }
                #[cfg(target_arch = "aarch64")]
                if self.use_neon {
                    unsafe {
                        let old_ptr = &(*self.map_conv.as_ptr().add(old_conv_idx)).0;
                        let new_ptr = &mut (*self.map_conv.as_mut_ptr().add(commit_idx)).0;
                        commit_conv_cell_neon(old_ptr, new_ptr, &mut vsum_group[gi][gj]);
                    }
                    commit_idx += 1;
                    continue;
                }

                let old_v = self.map_conv[old_conv_idx].0;
                let delta_v = self.map_conv[commit_idx].0;
                let mut new_v = [0_i16; FEAT_DWCONV_DIM];
                for ch in 0..FEAT_DWCONV_DIM {
                    let nv = old_v[ch].wrapping_add(delta_v[ch]);
                    new_v[ch] = nv;
                    vsum_group[gi][gj][ch] += (nv.max(0) as i32) - (old_v[ch].max(0) as i32);
                }
                self.map_conv[commit_idx].0 = new_v;
                commit_idx += 1;
            }
        }

        self.current_version += 1;
        let next_ver = self.current_version as usize;
        self.version_change_num[next_ver] = ChangeNum {
            inner: new_map_idx as u16,
            outer: commit_idx as u16,
        };

        #[cfg(target_arch = "x86_64")]
        if self.use_avx2 {
            // SAFETY: `ver < next_ver`, so the two entries in `value_sum_table` never alias.
            unsafe {
                let prev_ptr = self.value_sum_table.as_ptr().add(ver);
                let next_ptr = self.value_sum_table.as_mut_ptr().add(next_ver);
                commit_vsum_avx2(
                    &(*prev_ptr).group,
                    &vsum_group,
                    &mut (*next_ptr).group,
                    &mut (*next_ptr).global,
                );
            }
            return;
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon {
            // SAFETY: `ver < next_ver`, so the two entries in `value_sum_table` never alias.
            unsafe {
                let prev_ptr = self.value_sum_table.as_ptr().add(ver);
                let next_ptr = self.value_sum_table.as_mut_ptr().add(next_ver);
                commit_vsum_neon(
                    &(*prev_ptr).group,
                    &vsum_group,
                    &mut (*next_ptr).group,
                    &mut (*next_ptr).global,
                );
            }
            return;
        }

        let prev_sum = self.value_sum_table[ver];
        let next_sum = &mut self.value_sum_table[next_ver];
        next_sum.global = [0; FEATURE_DIM];
        for i in 0..3 {
            for j in 0..3 {
                for ch in 0..FEATURE_DIM {
                    let v = prev_sum.group[i][j][ch] + vsum_group[i][j][ch];
                    next_sum.group[i][j][ch] = v;
                    next_sum.global[ch] += v;
                }
            }
        }
    }

    /// Compute raw integer `layer3i32[0..3]` logits and `(win, loss, draw)` float logits.
    pub fn evaluate_value_logits(&self, w: &Mix9svqWeight) -> ([i32; 4], (f32, f32, f32)) {
        #[cfg(target_arch = "x86_64")]
        if self.use_avxvnni {
            // SAFETY: checked via runtime `is_x86_feature_detected!("avxvnni")`.
            return unsafe { self.evaluate_value_logits_avxvnni(w) };
        }
        #[cfg(target_arch = "x86_64")]
        if self.use_avx2 {
            // SAFETY: checked via runtime `is_x86_feature_detected!("avx2")`.
            return unsafe { self.evaluate_value_logits_avx2(w) };
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon_dotprod {
            // SAFETY: checked via runtime `is_aarch64_feature_detected!("dotprod")`.
            return unsafe { self.evaluate_value_logits_neon_dotprod(w) };
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon {
            // SAFETY: NEON is mandatory on aarch64.
            return unsafe { self.evaluate_value_logits_neon(w) };
        }
        self.evaluate_value_logits_scalar(w)
    }

    fn evaluate_value_logits_scalar(&self, w: &Mix9svqWeight) -> ([i32; 4], (f32, f32, f32)) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM + VALUE_DIM * 4];
        let mut global_i8 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_scalar::<FEATURE_DIM, 8, true>(&mut global_i8, &value_sum.global);
        layer0[..FEATURE_DIM].copy_from_slice(&global_i8);

        let mut group0 = [[[0_i8; FEATURE_DIM]; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_scalar::<FEATURE_DIM, 5, true>(
                    &mut group0[i][j],
                    &value_sum.group[i][j],
                );
            }
        }

        let mut group1 = [[[0_i8; VALUE_DIM]; 3]; 3];
        star_block_scalar(&mut group1[0][0], &group0[0][0], &b.value_corner);
        star_block_scalar(&mut group1[0][2], &group0[0][2], &b.value_corner);
        star_block_scalar(&mut group1[2][0], &group0[2][0], &b.value_corner);
        star_block_scalar(&mut group1[2][2], &group0[2][2], &b.value_corner);

        star_block_scalar(&mut group1[0][1], &group0[0][1], &b.value_edge);
        star_block_scalar(&mut group1[1][0], &group0[1][0], &b.value_edge);
        star_block_scalar(&mut group1[1][2], &group0[1][2], &b.value_edge);
        star_block_scalar(&mut group1[2][1], &group0[2][1], &b.value_edge);

        star_block_scalar(&mut group1[1][1], &group0[1][1], &b.value_center);

        let mut q00 = [0_i8; VALUE_DIM];
        let mut q01 = [0_i8; VALUE_DIM];
        let mut q10 = [0_i8; VALUE_DIM];
        let mut q11 = [0_i8; VALUE_DIM];
        for ch in 0..VALUE_DIM {
            q00[ch] = avg_u8(
                avg_u8(group1[0][0][ch], group1[0][1][ch]),
                avg_u8(group1[1][0][ch], group1[1][1][ch]),
            );
            q01[ch] = avg_u8(
                avg_u8(group1[0][1][ch], group1[0][2][ch]),
                avg_u8(group1[1][1][ch], group1[1][2][ch]),
            );
            q10[ch] = avg_u8(
                avg_u8(group1[1][0][ch], group1[1][1][ch]),
                avg_u8(group1[2][0][ch], group1[2][1][ch]),
            );
            q11[ch] = avg_u8(
                avg_u8(group1[1][1][ch], group1[1][2][ch]),
                avg_u8(group1[2][1][ch], group1[2][2][ch]),
            );
        }

        let mut quad_out = [[0_i8; VALUE_DIM]; 4];
        star_block_scalar(&mut quad_out[0], &q00, &b.value_quad);
        star_block_scalar(&mut quad_out[1], &q01, &b.value_quad);
        star_block_scalar(&mut quad_out[2], &q10, &b.value_quad);
        star_block_scalar(&mut quad_out[3], &q11, &b.value_quad);
        for q in 0..4 {
            let off = FEATURE_DIM + q * VALUE_DIM;
            layer0[off..off + VALUE_DIM].copy_from_slice(&quad_out[q]);
        }

        let mut l1_i32 = [0_i32; VALUE_DIM];
        let mut l1 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_scalar::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, false>(
            &mut l1_i32,
            &layer0,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_scalar::<VALUE_DIM, 7, false>(&mut l1, &l1_i32);

        let mut l2_i32 = [0_i32; VALUE_DIM];
        let mut l2 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_scalar::<VALUE_DIM, VALUE_DIM, false>(
            &mut l2_i32,
            &l1,
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_scalar::<VALUE_DIM, 7, false>(&mut l2, &l2_i32);

        let mut l3_i32 = [0_i32; 4];
        linear_i8_row4_scalar::<VALUE_DIM>(
            &mut l3_i32,
            &l2,
            &b.value_l3.weight,
            &b.value_l3.bias.as_slice().try_into().unwrap(),
        );
        let scale = 1.0_f32 / (128.0 * 128.0);
        (
            l3_i32,
            (
                l3_i32[0] as f32 * scale,
                l3_i32[1] as f32 * scale,
                l3_i32[2] as f32 * scale,
            ),
        )
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn evaluate_value_logits_avx2(&self, w: &Mix9svqWeight) -> ([i32; 4], (f32, f32, f32)) {
        use std::mem::MaybeUninit;
        let value_sum = self
            .value_sum_table
            .get_unchecked(self.current_version as usize);
        let b = &w.bucket;

        let mut layer0 = MaybeUninit::<[i8; FEATURE_DIM + VALUE_DIM * 4]>::uninit();
        let layer0_ptr = layer0.as_mut_ptr() as *mut i8;
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(
            &mut *(layer0_ptr as *mut [i8; FEATURE_DIM]),
            &value_sum.global,
        );

        let mut group0 = MaybeUninit::<[[[i8; FEATURE_DIM]; 3]; 3]>::uninit();
        let group0_mut = &mut *group0.as_mut_ptr();
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_avx2::<FEATURE_DIM, 5, true>(
                    &mut group0_mut[i][j],
                    &value_sum.group[i][j],
                );
            }
        }
        let group0_ref = &*group0.as_ptr();

        let mut group1 = MaybeUninit::<[[[i8; VALUE_DIM]; 3]; 3]>::uninit();
        let group1_mut = &mut *group1.as_mut_ptr();
        star_block_avx2(&mut group1_mut[0][0], &group0_ref[0][0], &b.value_corner);
        star_block_avx2(&mut group1_mut[0][2], &group0_ref[0][2], &b.value_corner);
        star_block_avx2(&mut group1_mut[2][0], &group0_ref[2][0], &b.value_corner);
        star_block_avx2(&mut group1_mut[2][2], &group0_ref[2][2], &b.value_corner);

        star_block_avx2(&mut group1_mut[0][1], &group0_ref[0][1], &b.value_edge);
        star_block_avx2(&mut group1_mut[1][0], &group0_ref[1][0], &b.value_edge);
        star_block_avx2(&mut group1_mut[1][2], &group0_ref[1][2], &b.value_edge);
        star_block_avx2(&mut group1_mut[2][1], &group0_ref[2][1], &b.value_edge);

        star_block_avx2(&mut group1_mut[1][1], &group0_ref[1][1], &b.value_center);
        let group1_ref = &*group1.as_ptr();

        let mut q00 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        let mut q01 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        let mut q10 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        let mut q11 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        avg_pool_2x2_u8_avx2(
            &mut *q00.as_mut_ptr(),
            &mut *q01.as_mut_ptr(),
            &mut *q10.as_mut_ptr(),
            &mut *q11.as_mut_ptr(),
            group1_ref,
        );

        star_block_avx2(
            &mut *(layer0_ptr.add(FEATURE_DIM) as *mut [i8; VALUE_DIM]),
            &*q00.as_ptr(),
            &b.value_quad,
        );
        star_block_avx2(
            &mut *(layer0_ptr.add(FEATURE_DIM + VALUE_DIM) as *mut [i8; VALUE_DIM]),
            &*q01.as_ptr(),
            &b.value_quad,
        );
        star_block_avx2(
            &mut *(layer0_ptr.add(FEATURE_DIM + 2 * VALUE_DIM) as *mut [i8; VALUE_DIM]),
            &*q10.as_ptr(),
            &b.value_quad,
        );
        star_block_avx2(
            &mut *(layer0_ptr.add(FEATURE_DIM + 3 * VALUE_DIM) as *mut [i8; VALUE_DIM]),
            &*q11.as_ptr(),
            &b.value_quad,
        );
        let layer0_ref = &*layer0.as_ptr();

        let mut l1_i32 = MaybeUninit::<[i32; VALUE_DIM]>::uninit();
        let mut l1 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        linear_i8_chunk4_avx2::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, false>(
            &mut *l1_i32.as_mut_ptr(),
            layer0_ref,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut *l1.as_mut_ptr(), &*l1_i32.as_ptr());

        let mut l2_i32 = MaybeUninit::<[i32; VALUE_DIM]>::uninit();
        let mut l2 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        linear_i8_chunk4_avx2::<VALUE_DIM, VALUE_DIM, false>(
            &mut *l2_i32.as_mut_ptr(),
            &*l1.as_ptr(),
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut *l2.as_mut_ptr(), &*l2_i32.as_ptr());

        let mut l3_i32 = MaybeUninit::<[i32; 4]>::uninit();
        let l3_bias: [i32; 4] = [
            b.value_l3.bias[0],
            b.value_l3.bias[1],
            b.value_l3.bias[2],
            b.value_l3.bias[3],
        ];
        linear_i8_row4_avx2::<VALUE_DIM>(
            &mut *l3_i32.as_mut_ptr(),
            &*l2.as_ptr(),
            &b.value_l3.weight,
            &l3_bias,
        );
        let l3 = l3_i32.assume_init();
        let scale = 1.0_f32 / (128.0 * 128.0);
        (
            l3,
            (
                l3[0] as f32 * scale,
                l3[1] as f32 * scale,
                l3[2] as f32 * scale,
            ),
        )
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2,avxvnni")]
    unsafe fn evaluate_value_logits_avxvnni(
        &self,
        w: &Mix9svqWeight,
    ) -> ([i32; 4], (f32, f32, f32)) {
        use std::mem::MaybeUninit;
        let value_sum = self
            .value_sum_table
            .get_unchecked(self.current_version as usize);
        let b = &w.bucket;

        let mut layer0 = MaybeUninit::<[i8; FEATURE_DIM + VALUE_DIM * 4]>::uninit();
        let layer0_ptr = layer0.as_mut_ptr() as *mut i8;
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(
            &mut *(layer0_ptr as *mut [i8; FEATURE_DIM]),
            &value_sum.global,
        );

        let mut group0 = MaybeUninit::<[[[i8; FEATURE_DIM]; 3]; 3]>::uninit();
        let group0_mut = &mut *group0.as_mut_ptr();
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_avx2::<FEATURE_DIM, 5, true>(
                    &mut group0_mut[i][j],
                    &value_sum.group[i][j],
                );
            }
        }
        let group0_ref = &*group0.as_ptr();

        let mut group1 = MaybeUninit::<[[[i8; VALUE_DIM]; 3]; 3]>::uninit();
        let g1_ptr = group1.as_mut_ptr() as *mut [i8; VALUE_DIM];
        // Batch-4 shared-weight star_blocks for the 4 corners and 4 edges (4x lower weight traffic).
        star_block_batch4_avxvnni(
            &mut *g1_ptr.add(0),
            &mut *g1_ptr.add(2),
            &mut *g1_ptr.add(6),
            &mut *g1_ptr.add(8),
            &group0_ref[0][0],
            &group0_ref[0][2],
            &group0_ref[2][0],
            &group0_ref[2][2],
            &b.value_corner,
        );
        star_block_batch4_avxvnni(
            &mut *g1_ptr.add(1),
            &mut *g1_ptr.add(3),
            &mut *g1_ptr.add(5),
            &mut *g1_ptr.add(7),
            &group0_ref[0][1],
            &group0_ref[1][0],
            &group0_ref[1][2],
            &group0_ref[2][1],
            &b.value_edge,
        );
        star_block_avxvnni(&mut *g1_ptr.add(4), &group0_ref[1][1], &b.value_center);
        let group1_ref = &*group1.as_ptr();

        let mut q00 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        let mut q01 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        let mut q10 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        let mut q11 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        avg_pool_2x2_u8_avx2(
            &mut *q00.as_mut_ptr(),
            &mut *q01.as_mut_ptr(),
            &mut *q10.as_mut_ptr(),
            &mut *q11.as_mut_ptr(),
            group1_ref,
        );

        star_block_batch4_avxvnni(
            &mut *(layer0_ptr.add(FEATURE_DIM) as *mut [i8; VALUE_DIM]),
            &mut *(layer0_ptr.add(FEATURE_DIM + VALUE_DIM) as *mut [i8; VALUE_DIM]),
            &mut *(layer0_ptr.add(FEATURE_DIM + 2 * VALUE_DIM) as *mut [i8; VALUE_DIM]),
            &mut *(layer0_ptr.add(FEATURE_DIM + 3 * VALUE_DIM) as *mut [i8; VALUE_DIM]),
            &*q00.as_ptr(),
            &*q01.as_ptr(),
            &*q10.as_ptr(),
            &*q11.as_ptr(),
            &b.value_quad,
        );
        let layer0_ref = &*layer0.as_ptr();

        let mut l1_i32 = MaybeUninit::<[i32; VALUE_DIM]>::uninit();
        let mut l1 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        linear_i8_chunk4_avxvnni::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, false>(
            &mut *l1_i32.as_mut_ptr(),
            layer0_ref,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut *l1.as_mut_ptr(), &*l1_i32.as_ptr());

        let mut l2_i32 = MaybeUninit::<[i32; VALUE_DIM]>::uninit();
        let mut l2 = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
        linear_i8_chunk4_avxvnni::<VALUE_DIM, VALUE_DIM, false>(
            &mut *l2_i32.as_mut_ptr(),
            &*l1.as_ptr(),
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut *l2.as_mut_ptr(), &*l2_i32.as_ptr());

        let mut l3_i32 = MaybeUninit::<[i32; 4]>::uninit();
        let l3_bias: [i32; 4] = [
            b.value_l3.bias[0],
            b.value_l3.bias[1],
            b.value_l3.bias[2],
            b.value_l3.bias[3],
        ];
        linear_i8_row4_avxvnni::<VALUE_DIM>(
            &mut *l3_i32.as_mut_ptr(),
            &*l2.as_ptr(),
            &b.value_l3.weight,
            &l3_bias,
        );
        let l3 = l3_i32.assume_init();
        let scale = 1.0_f32 / (128.0 * 128.0);
        (
            l3,
            (
                l3[0] as f32 * scale,
                l3[1] as f32 * scale,
                l3[2] as f32 * scale,
            ),
        )
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn evaluate_value_logits_neon(&self, w: &Mix9svqWeight) -> ([i32; 4], (f32, f32, f32)) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM + VALUE_DIM * 4];
        let mut global_i8 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_neon::<FEATURE_DIM, 8, true>(&mut global_i8, &value_sum.global);
        layer0[..FEATURE_DIM].copy_from_slice(&global_i8);

        let mut group0 = [[[0_i8; FEATURE_DIM]; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_neon::<FEATURE_DIM, 5, true>(
                    &mut group0[i][j],
                    &value_sum.group[i][j],
                );
            }
        }

        let mut group1 = [[[0_i8; VALUE_DIM]; 3]; 3];
        star_block_neon(&mut group1[0][0], &group0[0][0], &b.value_corner);
        star_block_neon(&mut group1[0][2], &group0[0][2], &b.value_corner);
        star_block_neon(&mut group1[2][0], &group0[2][0], &b.value_corner);
        star_block_neon(&mut group1[2][2], &group0[2][2], &b.value_corner);

        star_block_neon(&mut group1[0][1], &group0[0][1], &b.value_edge);
        star_block_neon(&mut group1[1][0], &group0[1][0], &b.value_edge);
        star_block_neon(&mut group1[1][2], &group0[1][2], &b.value_edge);
        star_block_neon(&mut group1[2][1], &group0[2][1], &b.value_edge);

        star_block_neon(&mut group1[1][1], &group0[1][1], &b.value_center);

        let mut q00 = [0_i8; VALUE_DIM];
        let mut q01 = [0_i8; VALUE_DIM];
        let mut q10 = [0_i8; VALUE_DIM];
        let mut q11 = [0_i8; VALUE_DIM];
        avg_pool_2x2_u8_neon(&mut q00, &mut q01, &mut q10, &mut q11, &group1);

        let mut quad_out = [[0_i8; VALUE_DIM]; 4];
        star_block_neon(&mut quad_out[0], &q00, &b.value_quad);
        star_block_neon(&mut quad_out[1], &q01, &b.value_quad);
        star_block_neon(&mut quad_out[2], &q10, &b.value_quad);
        star_block_neon(&mut quad_out[3], &q11, &b.value_quad);
        for q in 0..4 {
            let off = FEATURE_DIM + q * VALUE_DIM;
            layer0[off..off + VALUE_DIM].copy_from_slice(&quad_out[q]);
        }

        let mut l1_i32 = [0_i32; VALUE_DIM];
        let mut l1 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_neon::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, false>(
            &mut l1_i32,
            &layer0,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_neon::<VALUE_DIM, 7, false>(&mut l1, &l1_i32);

        let mut l2_i32 = [0_i32; VALUE_DIM];
        let mut l2 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_neon::<VALUE_DIM, VALUE_DIM, false>(
            &mut l2_i32,
            &l1,
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_neon::<VALUE_DIM, 7, false>(&mut l2, &l2_i32);

        let mut l3_i32 = [0_i32; 4];
        let l3_bias: [i32; 4] = [
            b.value_l3.bias[0],
            b.value_l3.bias[1],
            b.value_l3.bias[2],
            b.value_l3.bias[3],
        ];
        linear_i8_row4_neon::<VALUE_DIM>(&mut l3_i32, &l2, &b.value_l3.weight, &l3_bias);
        let scale = 1.0_f32 / (128.0 * 128.0);
        (
            l3_i32,
            (
                l3_i32[0] as f32 * scale,
                l3_i32[1] as f32 * scale,
                l3_i32[2] as f32 * scale,
            ),
        )
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn evaluate_value_logits_neon_dotprod(
        &self,
        w: &Mix9svqWeight,
    ) -> ([i32; 4], (f32, f32, f32)) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM + VALUE_DIM * 4];
        let mut global_i8 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_neon::<FEATURE_DIM, 8, true>(&mut global_i8, &value_sum.global);
        layer0[..FEATURE_DIM].copy_from_slice(&global_i8);

        let mut group0 = [[[0_i8; FEATURE_DIM]; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_neon::<FEATURE_DIM, 5, true>(
                    &mut group0[i][j],
                    &value_sum.group[i][j],
                );
            }
        }

        let mut group1 = [[[0_i8; VALUE_DIM]; 3]; 3];
        star_block_neon_dotprod(&mut group1[0][0], &group0[0][0], &b.value_corner);
        star_block_neon_dotprod(&mut group1[0][2], &group0[0][2], &b.value_corner);
        star_block_neon_dotprod(&mut group1[2][0], &group0[2][0], &b.value_corner);
        star_block_neon_dotprod(&mut group1[2][2], &group0[2][2], &b.value_corner);

        star_block_neon_dotprod(&mut group1[0][1], &group0[0][1], &b.value_edge);
        star_block_neon_dotprod(&mut group1[1][0], &group0[1][0], &b.value_edge);
        star_block_neon_dotprod(&mut group1[1][2], &group0[1][2], &b.value_edge);
        star_block_neon_dotprod(&mut group1[2][1], &group0[2][1], &b.value_edge);

        star_block_neon_dotprod(&mut group1[1][1], &group0[1][1], &b.value_center);

        let mut q00 = [0_i8; VALUE_DIM];
        let mut q01 = [0_i8; VALUE_DIM];
        let mut q10 = [0_i8; VALUE_DIM];
        let mut q11 = [0_i8; VALUE_DIM];
        avg_pool_2x2_u8_neon(&mut q00, &mut q01, &mut q10, &mut q11, &group1);

        let mut quad_out = [[0_i8; VALUE_DIM]; 4];
        star_block_neon_dotprod(&mut quad_out[0], &q00, &b.value_quad);
        star_block_neon_dotprod(&mut quad_out[1], &q01, &b.value_quad);
        star_block_neon_dotprod(&mut quad_out[2], &q10, &b.value_quad);
        star_block_neon_dotprod(&mut quad_out[3], &q11, &b.value_quad);
        for q in 0..4 {
            let off = FEATURE_DIM + q * VALUE_DIM;
            layer0[off..off + VALUE_DIM].copy_from_slice(&quad_out[q]);
        }

        let mut l1_i32 = [0_i32; VALUE_DIM];
        let mut l1 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_neon_dotprod::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }>(
            &mut l1_i32,
            &layer0,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_neon::<VALUE_DIM, 7, false>(&mut l1, &l1_i32);

        let mut l2_i32 = [0_i32; VALUE_DIM];
        let mut l2 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_neon_dotprod::<VALUE_DIM, VALUE_DIM>(
            &mut l2_i32,
            &l1,
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_neon::<VALUE_DIM, 7, false>(&mut l2, &l2_i32);

        let mut l3_i32 = [0_i32; 4];
        let l3_bias: [i32; 4] = [
            b.value_l3.bias[0],
            b.value_l3.bias[1],
            b.value_l3.bias[2],
            b.value_l3.bias[3],
        ];
        linear_i8_row4_neon_dotprod::<VALUE_DIM>(&mut l3_i32, &l2, &b.value_l3.weight, &l3_bias);
        let scale = 1.0_f32 / (128.0 * 128.0);
        (
            l3_i32,
            (
                l3_i32[0] as f32 * scale,
                l3_i32[1] as f32 * scale,
                l3_i32[2] as f32 * scale,
            ),
        )
    }

    /// Fast policy evaluation for an explicit slice of `Move`s (e.g. `<= 40`
    /// candidates), avoiding scanning all 225 cells.
    pub fn evaluate_policy_moves(
        &self,
        w: &Mix9svqWeight,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        debug_assert_eq!(moves.len(), out_scores.len());
        #[cfg(target_arch = "x86_64")]
        if self.use_avxvnni {
            unsafe {
                self.evaluate_policy_moves_avxvnni(w, moves, out_scores);
            }
            return;
        }
        #[cfg(target_arch = "x86_64")]
        if self.use_avx2 {
            unsafe {
                self.evaluate_policy_moves_avx2(w, moves, out_scores);
            }
            return;
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon_dotprod {
            unsafe {
                self.evaluate_policy_moves_neon_dotprod(w, moves, out_scores);
            }
            return;
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon {
            unsafe {
                self.evaluate_policy_moves_neon(w, moves, out_scores);
            }
            return;
        }
        self.evaluate_policy_moves_scalar(w, moves, out_scores);
    }

    fn evaluate_policy_moves_scalar(
        &self,
        w: &Mix9svqWeight,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_scalar::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_scalar::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_scalar::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_scalar::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_scalar::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for (dst, &m) in out_scores.iter_mut().zip(moves.iter()) {
            let idx = m as usize;
            let y = idx / BOARD_SIZE;
            let x = idx % BOARD_SIZE;
            let outer_idx = (y + 1) * OUTER_BOARD_SIZE + (x + 1);
            let mc_idx = self.version_outer_index[outer_ver_base + outer_idx] as usize;
            linear_i16_chunk2_prerelu_scalar::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut p_i32,
                &self.map_conv[mc_idx].0,
                &pw_w,
                &pw_b,
            );
            let mut acc = 0.0_f32;
            for c in 0..POLICY_PWCONV_DIM {
                acc += (p_i32[c].max(0) as f32) * b.policy_output_weight[c];
            }
            *dst = acc + b.policy_output_bias;
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2,avxvnni")]
    unsafe fn evaluate_policy_moves_avxvnni(
        &self,
        w: &Mix9svqWeight,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        use std::mem::MaybeUninit;
        let value_sum = self
            .value_sum_table
            .get_unchecked(self.current_version as usize);
        let b = &w.bucket;

        let mut layer0 = MaybeUninit::<[i8; FEATURE_DIM]>::uninit();
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(&mut *layer0.as_mut_ptr(), &value_sum.global);

        let mut l1_i32 = MaybeUninit::<[i32; POLICY_DIM * 2]>::uninit();
        let mut l1 = MaybeUninit::<[i8; POLICY_DIM * 2]>::uninit();
        linear_i8_chunk4_avxvnni::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut *l1_i32.as_mut_ptr(),
            &*layer0.as_ptr(),
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_avx2::<{ POLICY_DIM * 2 }, 7, false>(
            &mut *l1.as_mut_ptr(),
            &*l1_i32.as_ptr(),
        );

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = MaybeUninit::<[i32; L2_OUT]>::uninit();
        linear_i8_chunk4_avxvnni::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut *l2_i32.as_mut_ptr(),
            &*l1.as_ptr(),
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let l2_ptr = l2_i32.as_ptr() as *const i32;
        let mut pw_w = MaybeUninit::<[i16; POLICY_PWCONV_DIM * POLICY_DIM]>::uninit();
        crelu_i32_to_i16_avx2::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(
            &mut *pw_w.as_mut_ptr(),
            &*(l2_ptr as *const [i32; POLICY_PWCONV_DIM * POLICY_DIM]),
        );
        let pw_w_ref = &*pw_w.as_ptr();
        let mut pw_b = MaybeUninit::<[i32; POLICY_PWCONV_DIM]>::uninit();
        let pw_b_ptr = pw_b.as_mut_ptr() as *mut i32;
        for i in 0..POLICY_PWCONV_DIM {
            *pw_b_ptr.add(i) = *l2_ptr.add(POLICY_PWCONV_DIM * POLICY_DIM + i) << 7;
        }
        let pw_b_ref = &*pw_b.as_ptr();

        let outer_ver_ptr = self
            .version_outer_index
            .as_ptr()
            .add((self.current_version as usize) * OUTER_BOARD_AREA);
        let map_conv_ptr = self.map_conv.as_ptr();
        let mut p_i32 = MaybeUninit::<[i32; POLICY_PWCONV_DIM]>::uninit();
        let n = moves.len().min(out_scores.len());
        let mut i = 0;
        while i + 4 <= n {
            let mc0 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i) as usize) as usize)
                as usize;
            let mc1 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i + 1) as usize) as usize)
                as usize;
            let mc2 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i + 2) as usize) as usize)
                as usize;
            let mc3 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i + 3) as usize) as usize)
                as usize;
            let s4 = policy_eval_batch4_avx2(
                &*map_conv_ptr.add(mc0),
                &*map_conv_ptr.add(mc1),
                &*map_conv_ptr.add(mc2),
                &*map_conv_ptr.add(mc3),
                pw_w_ref,
                pw_b_ref,
                &b.policy_output_weight,
                b.policy_output_bias,
            );
            *out_scores.get_unchecked_mut(i) = s4[0];
            *out_scores.get_unchecked_mut(i + 1) = s4[1];
            *out_scores.get_unchecked_mut(i + 2) = s4[2];
            *out_scores.get_unchecked_mut(i + 3) = s4[3];
            i += 4;
        }
        while i < n {
            let idx = *moves.get_unchecked(i) as usize;
            let outer_idx = *INNER_TO_OUTER.get_unchecked(idx) as usize;
            let mc_idx = *outer_ver_ptr.add(outer_idx) as usize;
            linear_i16_chunk2_prerelu_avx2::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut *p_i32.as_mut_ptr(),
                &(*map_conv_ptr.add(mc_idx)).0,
                pw_w_ref,
                pw_b_ref,
            );
            *out_scores.get_unchecked_mut(i) = policy_dot16_avx2(
                &*p_i32.as_ptr(),
                &b.policy_output_weight,
                b.policy_output_bias,
            );
            i += 1;
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn evaluate_policy_moves_avx2(
        &self,
        w: &Mix9svqWeight,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        use std::mem::MaybeUninit;
        let value_sum = self
            .value_sum_table
            .get_unchecked(self.current_version as usize);
        let b = &w.bucket;

        let mut layer0 = MaybeUninit::<[i8; FEATURE_DIM]>::uninit();
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(&mut *layer0.as_mut_ptr(), &value_sum.global);

        let mut l1_i32 = MaybeUninit::<[i32; POLICY_DIM * 2]>::uninit();
        let mut l1 = MaybeUninit::<[i8; POLICY_DIM * 2]>::uninit();
        linear_i8_chunk4_avx2::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut *l1_i32.as_mut_ptr(),
            &*layer0.as_ptr(),
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_avx2::<{ POLICY_DIM * 2 }, 7, false>(
            &mut *l1.as_mut_ptr(),
            &*l1_i32.as_ptr(),
        );

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = MaybeUninit::<[i32; L2_OUT]>::uninit();
        linear_i8_chunk4_avx2::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut *l2_i32.as_mut_ptr(),
            &*l1.as_ptr(),
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let l2_ptr = l2_i32.as_ptr() as *const i32;
        let mut pw_w = MaybeUninit::<[i16; POLICY_PWCONV_DIM * POLICY_DIM]>::uninit();
        crelu_i32_to_i16_avx2::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(
            &mut *pw_w.as_mut_ptr(),
            &*(l2_ptr as *const [i32; POLICY_PWCONV_DIM * POLICY_DIM]),
        );
        let pw_w_ref = &*pw_w.as_ptr();
        let mut pw_b = MaybeUninit::<[i32; POLICY_PWCONV_DIM]>::uninit();
        let pw_b_ptr = pw_b.as_mut_ptr() as *mut i32;
        for i in 0..POLICY_PWCONV_DIM {
            *pw_b_ptr.add(i) = *l2_ptr.add(POLICY_PWCONV_DIM * POLICY_DIM + i) << 7;
        }
        let pw_b_ref = &*pw_b.as_ptr();

        let outer_ver_ptr = self
            .version_outer_index
            .as_ptr()
            .add((self.current_version as usize) * OUTER_BOARD_AREA);
        let map_conv_ptr = self.map_conv.as_ptr();
        let mut p_i32 = MaybeUninit::<[i32; POLICY_PWCONV_DIM]>::uninit();
        let n = moves.len().min(out_scores.len());
        let mut i = 0;
        while i + 4 <= n {
            let mc0 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i) as usize) as usize)
                as usize;
            let mc1 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i + 1) as usize) as usize)
                as usize;
            let mc2 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i + 2) as usize) as usize)
                as usize;
            let mc3 = *outer_ver_ptr
                .add(*INNER_TO_OUTER.get_unchecked(*moves.get_unchecked(i + 3) as usize) as usize)
                as usize;
            let s4 = policy_eval_batch4_avx2(
                &*map_conv_ptr.add(mc0),
                &*map_conv_ptr.add(mc1),
                &*map_conv_ptr.add(mc2),
                &*map_conv_ptr.add(mc3),
                pw_w_ref,
                pw_b_ref,
                &b.policy_output_weight,
                b.policy_output_bias,
            );
            *out_scores.get_unchecked_mut(i) = s4[0];
            *out_scores.get_unchecked_mut(i + 1) = s4[1];
            *out_scores.get_unchecked_mut(i + 2) = s4[2];
            *out_scores.get_unchecked_mut(i + 3) = s4[3];
            i += 4;
        }
        while i < n {
            let idx = *moves.get_unchecked(i) as usize;
            let outer_idx = *INNER_TO_OUTER.get_unchecked(idx) as usize;
            let mc_idx = *outer_ver_ptr.add(outer_idx) as usize;
            linear_i16_chunk2_prerelu_avx2::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut *p_i32.as_mut_ptr(),
                &(*map_conv_ptr.add(mc_idx)).0,
                pw_w_ref,
                pw_b_ref,
            );
            *out_scores.get_unchecked_mut(i) = policy_dot16_avx2(
                &*p_i32.as_ptr(),
                &b.policy_output_weight,
                b.policy_output_bias,
            );
            i += 1;
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn evaluate_policy_moves_neon(
        &self,
        w: &Mix9svqWeight,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_neon::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_neon::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_neon::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_neon::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_neon::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for (dst, &m) in out_scores.iter_mut().zip(moves.iter()) {
            let idx = m as usize;
            let y = idx / BOARD_SIZE;
            let x = idx % BOARD_SIZE;
            let outer_idx = (y + 1) * OUTER_BOARD_SIZE + (x + 1);
            let mc_idx = self.version_outer_index[outer_ver_base + outer_idx] as usize;
            linear_i16_chunk2_prerelu_neon::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut p_i32,
                &self.map_conv[mc_idx].0,
                &pw_w,
                &pw_b,
            );
            *dst = policy_dot16_neon(&p_i32, &b.policy_output_weight, b.policy_output_bias);
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn evaluate_policy_moves_neon_dotprod(
        &self,
        w: &Mix9svqWeight,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_neon::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_neon_dotprod::<{ POLICY_DIM * 2 }, FEATURE_DIM>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_neon::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_neon_dotprod::<L2_OUT, { POLICY_DIM * 2 }>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_neon::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for (dst, &m) in out_scores.iter_mut().zip(moves.iter()) {
            let idx = m as usize;
            let y = idx / BOARD_SIZE;
            let x = idx % BOARD_SIZE;
            let outer_idx = (y + 1) * OUTER_BOARD_SIZE + (x + 1);
            let mc_idx = self.version_outer_index[outer_ver_base + outer_idx] as usize;
            linear_i16_chunk2_prerelu_neon::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut p_i32,
                &self.map_conv[mc_idx].0,
                &pw_w,
                &pw_b,
            );
            *dst = policy_dot16_neon(&p_i32, &b.policy_output_weight, b.policy_output_bias);
        }
    }

    /// Compute policy logits for every cell where `compute_mask[y * 15 + x]` is `true`.
    pub fn evaluate_policy(
        &self,
        w: &Mix9svqWeight,
        compute_mask: &[bool; BOARD_AREA],
        out_policy: &mut [f32; BOARD_AREA],
    ) {
        #[cfg(target_arch = "x86_64")]
        if self.use_avx2 {
            unsafe {
                self.evaluate_policy_avx2(w, compute_mask, out_policy);
            }
            return;
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon_dotprod {
            unsafe {
                self.evaluate_policy_neon_dotprod(w, compute_mask, out_policy);
            }
            return;
        }
        #[cfg(target_arch = "aarch64")]
        if self.use_neon {
            unsafe {
                self.evaluate_policy_neon(w, compute_mask, out_policy);
            }
            return;
        }
        self.evaluate_policy_scalar(w, compute_mask, out_policy);
    }

    fn evaluate_policy_scalar(
        &self,
        w: &Mix9svqWeight,
        compute_mask: &[bool; BOARD_AREA],
        out_policy: &mut [f32; BOARD_AREA],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_scalar::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_scalar::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_scalar::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_scalar::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_scalar::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for y in 0..BOARD_SIZE {
            let inner_row = y * BOARD_SIZE;
            let outer_row = (y + 1) * OUTER_BOARD_SIZE + 1;
            for x in 0..BOARD_SIZE {
                let inner_idx = inner_row + x;
                if !compute_mask[inner_idx] {
                    continue;
                }
                let mc_idx = self.version_outer_index[outer_ver_base + outer_row + x] as usize;
                linear_i16_chunk2_prerelu_scalar::<POLICY_PWCONV_DIM, POLICY_DIM>(
                    &mut p_i32,
                    &self.map_conv[mc_idx].0,
                    &pw_w,
                    &pw_b,
                );
                let mut acc = 0.0_f32;
                for c in 0..POLICY_PWCONV_DIM {
                    acc += (p_i32[c].max(0) as f32) * b.policy_output_weight[c];
                }
                out_policy[inner_idx] = acc + b.policy_output_bias;
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn evaluate_policy_avx2(
        &self,
        w: &Mix9svqWeight,
        compute_mask: &[bool; BOARD_AREA],
        out_policy: &mut [f32; BOARD_AREA],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_avx2::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_avx2::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_avx2::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_avx2::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for y in 0..BOARD_SIZE {
            let inner_row = y * BOARD_SIZE;
            let outer_row = (y + 1) * OUTER_BOARD_SIZE + 1;
            for x in 0..BOARD_SIZE {
                let inner_idx = inner_row + x;
                if !compute_mask[inner_idx] {
                    continue;
                }
                let mc_idx = self.version_outer_index[outer_ver_base + outer_row + x] as usize;
                linear_i16_chunk2_prerelu_avx2::<POLICY_PWCONV_DIM, POLICY_DIM>(
                    &mut p_i32,
                    &self.map_conv[mc_idx].0,
                    &pw_w,
                    &pw_b,
                );
                out_policy[inner_idx] =
                    policy_dot16_avx2(&p_i32, &b.policy_output_weight, b.policy_output_bias);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn evaluate_policy_neon(
        &self,
        w: &Mix9svqWeight,
        compute_mask: &[bool; BOARD_AREA],
        out_policy: &mut [f32; BOARD_AREA],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_neon::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_neon::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_neon::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_neon::<L2_OUT, { POLICY_DIM * 2 }, false>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_neon::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for y in 0..BOARD_SIZE {
            let inner_row = y * BOARD_SIZE;
            let outer_row = (y + 1) * OUTER_BOARD_SIZE + 1;
            for x in 0..BOARD_SIZE {
                let inner_idx = inner_row + x;
                if !compute_mask[inner_idx] {
                    continue;
                }
                let mc_idx = self.version_outer_index[outer_ver_base + outer_row + x] as usize;
                linear_i16_chunk2_prerelu_neon::<POLICY_PWCONV_DIM, POLICY_DIM>(
                    &mut p_i32,
                    &self.map_conv[mc_idx].0,
                    &pw_w,
                    &pw_b,
                );
                out_policy[inner_idx] =
                    policy_dot16_neon(&p_i32, &b.policy_output_weight, b.policy_output_bias);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon,dotprod")]
    unsafe fn evaluate_policy_neon_dotprod(
        &self,
        w: &Mix9svqWeight,
        compute_mask: &[bool; BOARD_AREA],
        out_policy: &mut [f32; BOARD_AREA],
    ) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_neon::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_neon_dotprod::<{ POLICY_DIM * 2 }, FEATURE_DIM>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_neon::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_neon_dotprod::<L2_OUT, { POLICY_DIM * 2 }>(
            &mut l2_i32,
            &l1,
            &b.policy_pwconv_l2.weight,
            &b.policy_pwconv_l2.bias,
        );
        let mut pw_w = [0_i16; POLICY_PWCONV_DIM * POLICY_DIM];
        let l2_head: &[i32; POLICY_PWCONV_DIM * POLICY_DIM] =
            l2_i32[..POLICY_PWCONV_DIM * POLICY_DIM].try_into().unwrap();
        crelu_i32_to_i16_neon::<{ POLICY_PWCONV_DIM * POLICY_DIM }, 0, true>(&mut pw_w, l2_head);
        let mut pw_b = [0_i32; POLICY_PWCONV_DIM];
        for i in 0..POLICY_PWCONV_DIM {
            pw_b[i] = l2_i32[POLICY_PWCONV_DIM * POLICY_DIM + i] << 7;
        }

        let outer_ver_base = (self.current_version as usize) * OUTER_BOARD_AREA;
        let mut p_i32 = [0_i32; POLICY_PWCONV_DIM];
        for y in 0..BOARD_SIZE {
            let inner_row = y * BOARD_SIZE;
            let outer_row = (y + 1) * OUTER_BOARD_SIZE + 1;
            for x in 0..BOARD_SIZE {
                let inner_idx = inner_row + x;
                if !compute_mask[inner_idx] {
                    continue;
                }
                let mc_idx = self.version_outer_index[outer_ver_base + outer_row + x] as usize;
                linear_i16_chunk2_prerelu_neon::<POLICY_PWCONV_DIM, POLICY_DIM>(
                    &mut p_i32,
                    &self.map_conv[mc_idx].0,
                    &pw_w,
                    &pw_b,
                );
                out_policy[inner_idx] =
                    policy_dot16_neon(&p_i32, &b.policy_output_weight, b.policy_output_bias);
            }
        }
    }
}

fn star_block_scalar(
    output: &mut [i8; VALUE_DIM],
    input: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    let mut up_i32 = [0_i32; VALUE_DIM * 2];
    let mut up1 = [0_i8; VALUE_DIM * 2];
    let mut up2 = [0_i8; VALUE_DIM * 2];

    linear_i8_chunk4_scalar::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_scalar::<{ VALUE_DIM * 2 }, 7, false>(&mut up1, &up_i32);

    linear_i8_chunk4_scalar::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_scalar::<{ VALUE_DIM * 2 }, 7, true>(&mut up2, &up_i32);

    let mut dotsum = [0_i8; VALUE_DIM];
    dot2_u7i8_scalar::<VALUE_DIM, 7>(&mut dotsum, &up1, &up2);

    let mut out_i32 = [0_i32; VALUE_DIM];
    linear_i8_chunk4_scalar::<VALUE_DIM, VALUE_DIM, true>(
        &mut out_i32,
        &dotsum,
        &w.down.weight,
        &w.down.bias,
    );
    crelu_i32_to_i8_scalar::<VALUE_DIM, 7, false>(output, &out_i32);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn star_block_avx2(
    output: &mut [i8; VALUE_DIM],
    input: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    use std::mem::MaybeUninit;
    let mut up_i32 = MaybeUninit::<[i32; VALUE_DIM * 2]>::uninit();
    let mut up1 = MaybeUninit::<[i8; VALUE_DIM * 2]>::uninit();
    let mut up2 = MaybeUninit::<[i8; VALUE_DIM * 2]>::uninit();

    linear_i8_chunk4_avx2::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut *up_i32.as_mut_ptr(),
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, false>(&mut *up1.as_mut_ptr(), &*up_i32.as_ptr());

    linear_i8_chunk4_avx2::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut *up_i32.as_mut_ptr(),
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, true>(&mut *up2.as_mut_ptr(), &*up_i32.as_ptr());

    let mut dotsum = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
    dot2_u7i8_avx2::<VALUE_DIM, 7>(&mut *dotsum.as_mut_ptr(), &*up1.as_ptr(), &*up2.as_ptr());

    let mut out_i32 = MaybeUninit::<[i32; VALUE_DIM]>::uninit();
    linear_i8_chunk4_avx2::<VALUE_DIM, VALUE_DIM, true>(
        &mut *out_i32.as_mut_ptr(),
        &*dotsum.as_ptr(),
        &w.down.weight,
        &w.down.bias,
    );
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(output, &*out_i32.as_ptr());
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,avxvnni")]
unsafe fn star_block_avxvnni(
    output: &mut [i8; VALUE_DIM],
    input: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    use std::mem::MaybeUninit;
    let mut up_i32 = MaybeUninit::<[i32; VALUE_DIM * 2]>::uninit();
    let mut up1 = MaybeUninit::<[i8; VALUE_DIM * 2]>::uninit();
    let mut up2 = MaybeUninit::<[i8; VALUE_DIM * 2]>::uninit();

    linear_i8_chunk4_avxvnni::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut *up_i32.as_mut_ptr(),
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, false>(&mut *up1.as_mut_ptr(), &*up_i32.as_ptr());

    linear_i8_chunk4_avxvnni::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut *up_i32.as_mut_ptr(),
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, true>(&mut *up2.as_mut_ptr(), &*up_i32.as_ptr());

    let mut dotsum = MaybeUninit::<[i8; VALUE_DIM]>::uninit();
    dot2_u7i8_avx2::<VALUE_DIM, 7>(&mut *dotsum.as_mut_ptr(), &*up1.as_ptr(), &*up2.as_ptr());

    let mut out_i32 = MaybeUninit::<[i32; VALUE_DIM]>::uninit();
    linear_i8_chunk4_avxvnni::<VALUE_DIM, VALUE_DIM, true>(
        &mut *out_i32.as_mut_ptr(),
        &*dotsum.as_ptr(),
        &w.down.weight,
        &w.down.signed_bias,
    );
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(output, &*out_i32.as_ptr());
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,avxvnni")]
#[allow(clippy::too_many_arguments)]
unsafe fn star_block_batch4_avxvnni(
    out0: &mut [i8; VALUE_DIM],
    out1: &mut [i8; VALUE_DIM],
    out2: &mut [i8; VALUE_DIM],
    out3: &mut [i8; VALUE_DIM],
    in0: &[i8; FEATURE_DIM],
    in1: &[i8; FEATURE_DIM],
    in2: &[i8; FEATURE_DIM],
    in3: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    use std::mem::MaybeUninit;
    let mut up_i32 = MaybeUninit::<[[i32; VALUE_DIM * 2]; 4]>::uninit();
    let up_i32_ptr = up_i32.as_mut_ptr() as *mut [i32; VALUE_DIM * 2];
    let mut up1 = MaybeUninit::<[[i8; VALUE_DIM * 2]; 4]>::uninit();
    let up1_ptr = up1.as_mut_ptr() as *mut [i8; VALUE_DIM * 2];
    let mut up2 = MaybeUninit::<[[i8; VALUE_DIM * 2]; 4]>::uninit();
    let up2_ptr = up2.as_mut_ptr() as *mut [i8; VALUE_DIM * 2];

    linear_i8_chunk4_batch4_avxvnni::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut *up_i32_ptr.add(0),
        &mut *up_i32_ptr.add(1),
        &mut *up_i32_ptr.add(2),
        &mut *up_i32_ptr.add(3),
        in0,
        in1,
        in2,
        in3,
        &w.up1.weight,
        &w.up1.bias,
    );
    for k in 0..4 {
        crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, false>(
            &mut *up1_ptr.add(k),
            &*up_i32_ptr.add(k),
        );
    }

    linear_i8_chunk4_batch4_avxvnni::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut *up_i32_ptr.add(0),
        &mut *up_i32_ptr.add(1),
        &mut *up_i32_ptr.add(2),
        &mut *up_i32_ptr.add(3),
        in0,
        in1,
        in2,
        in3,
        &w.up2.weight,
        &w.up2.bias,
    );
    for k in 0..4 {
        crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, true>(
            &mut *up2_ptr.add(k),
            &*up_i32_ptr.add(k),
        );
    }

    let mut dotsum = MaybeUninit::<[[i8; VALUE_DIM]; 4]>::uninit();
    let ds_ptr = dotsum.as_mut_ptr() as *mut [i8; VALUE_DIM];
    for k in 0..4 {
        dot2_u7i8_avx2::<VALUE_DIM, 7>(&mut *ds_ptr.add(k), &*up1_ptr.add(k), &*up2_ptr.add(k));
    }

    let mut out_i32 = MaybeUninit::<[[i32; VALUE_DIM]; 4]>::uninit();
    let oi_ptr = out_i32.as_mut_ptr() as *mut [i32; VALUE_DIM];
    linear_i8_chunk4_batch4_avxvnni::<VALUE_DIM, VALUE_DIM, true>(
        &mut *oi_ptr.add(0),
        &mut *oi_ptr.add(1),
        &mut *oi_ptr.add(2),
        &mut *oi_ptr.add(3),
        &*ds_ptr.add(0),
        &*ds_ptr.add(1),
        &*ds_ptr.add(2),
        &*ds_ptr.add(3),
        &w.down.weight,
        &w.down.signed_bias,
    );
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(out0, &*oi_ptr.add(0));
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(out1, &*oi_ptr.add(1));
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(out2, &*oi_ptr.add(2));
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(out3, &*oi_ptr.add(3));
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn star_block_neon(
    output: &mut [i8; VALUE_DIM],
    input: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    let mut up_i32 = [0_i32; VALUE_DIM * 2];
    let mut up1 = [0_i8; VALUE_DIM * 2];
    let mut up2 = [0_i8; VALUE_DIM * 2];

    linear_i8_chunk4_neon::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_neon::<{ VALUE_DIM * 2 }, 7, false>(&mut up1, &up_i32);

    linear_i8_chunk4_neon::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_neon::<{ VALUE_DIM * 2 }, 7, true>(&mut up2, &up_i32);

    let mut dotsum = [0_i8; VALUE_DIM];
    dot2_u7i8_neon::<VALUE_DIM, 7>(&mut dotsum, &up1, &up2);

    let mut out_i32 = [0_i32; VALUE_DIM];
    linear_i8_chunk4_neon::<VALUE_DIM, VALUE_DIM, true>(
        &mut out_i32,
        &dotsum,
        &w.down.weight,
        &w.down.bias,
    );
    crelu_i32_to_i8_neon::<VALUE_DIM, 7, false>(output, &out_i32);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon,dotprod")]
unsafe fn star_block_neon_dotprod(
    output: &mut [i8; VALUE_DIM],
    input: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    let mut up_i32 = [0_i32; VALUE_DIM * 2];
    let mut up1 = [0_i8; VALUE_DIM * 2];
    let mut up2 = [0_i8; VALUE_DIM * 2];

    linear_i8_chunk4_neon_dotprod::<{ VALUE_DIM * 2 }, FEATURE_DIM>(
        &mut up_i32,
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_neon::<{ VALUE_DIM * 2 }, 7, false>(&mut up1, &up_i32);

    linear_i8_chunk4_neon_dotprod::<{ VALUE_DIM * 2 }, FEATURE_DIM>(
        &mut up_i32,
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_neon::<{ VALUE_DIM * 2 }, 7, true>(&mut up2, &up_i32);

    let mut dotsum = [0_i8; VALUE_DIM];
    dot2_u7i8_neon::<VALUE_DIM, 7>(&mut dotsum, &up1, &up2);

    let mut out_i32 = [0_i32; VALUE_DIM];
    linear_i8_chunk4_neon_dotprod::<VALUE_DIM, VALUE_DIM>(
        &mut out_i32,
        &dotsum,
        &w.down.weight,
        &w.down.bias,
    );
    crelu_i32_to_i8_neon::<VALUE_DIM, 7, false>(output, &out_i32);
}

/// Convert `(win_logit, loss_logit, draw_logit)` via softmax and Rapfi's
/// `winRateToValue` sigmoid into a signed evaluation in `[-NNUE_VALUE_CLAMP, NNUE_VALUE_CLAMP]`.
#[inline]
pub fn logits_to_value(win_logit: f32, loss_logit: f32, draw_logit: f32) -> i32 {
    let max_v = win_logit.max(loss_logit).max(draw_logit);
    let w = (win_logit - max_v).exp();
    let l = (loss_logit - max_v).exp();
    let d = (draw_logit - max_v).exp();
    let inv = 1.0_f32 / (w + l + d);
    let win_prob = w * inv;
    let loss_prob = l * inv;
    let win_rate = ((win_prob - loss_prob) + 1.0_f32) * 0.5_f32;
    let win_rate = win_rate.clamp(1e-7_f32, 1.0_f32 - 1e-7_f32);
    let val = NNUE_SCALING_FACTOR * (win_rate / (1.0_f32 - win_rate)).ln();
    (val.clamp(-NNUE_VALUE_CLAMP as f32, NNUE_VALUE_CLAMP as f32)) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MoveCacheEntry {
    /// 0 = black, 1 = white, 2 = empty
    old_color: u8,
    new_color: u8,
    x: u8,
    y: u8,
}

impl MoveCacheEntry {
    #[inline(always)]
    fn is_contrary(self, other: Self) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.old_color == other.new_color
            && self.new_color == other.old_color
    }
}

const COLOR_BLACK: u8 = 0;
const COLOR_WHITE: u8 = 1;
const COLOR_EMPTY: u8 = 2;

#[inline(always)]
fn side_to_color(side: Side) -> u8 {
    if side == BLACK {
        COLOR_BLACK
    } else {
        COLOR_WHITE
    }
}

/// Complete two-sided `Mix9svq` evaluator with lazy `MoveCache` and shared weights.
#[derive(Clone, Debug)]
pub struct Mix9svqEvaluator {
    weight: [Arc<Mix9svqWeight>; 2],
    accumulator: [Box<Mix9svqAccumulator>; 2],
    move_cache: [Vec<MoveCacheEntry>; 2],
    /// Root move sequence currently reflected by the evaluator (when synced via
    /// `sync_with_board` on a board with `move_history()`).
    synced_history: Option<Vec<(Side, u8, u8)>>,
    /// Net `before_move` minus `after_undo` calls since the last `sync_with_board`.
    /// When zero, `sync_with_board` can safely apply only the LCP diff instead of
    /// replaying the entire board from empty.
    pending_balance: isize,
}

#[cfg(test)]
pub(crate) mod test_counters {
    use std::sync::atomic::{AtomicU64, Ordering};
    pub static PLAY_MOVE_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static VALUE_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static POLICY_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static POLICY_CANDS: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        PLAY_MOVE_CALLS.store(0, Ordering::Relaxed);
        VALUE_CALLS.store(0, Ordering::Relaxed);
        POLICY_CALLS.store(0, Ordering::Relaxed);
        POLICY_CANDS.store(0, Ordering::Relaxed);
    }
}

impl Mix9svqEvaluator {
    pub fn new(black_weight: Arc<Mix9svqWeight>, white_weight: Arc<Mix9svqWeight>) -> Self {
        let mut eval = Self {
            weight: [black_weight, white_weight],
            accumulator: [
                Box::new(Mix9svqAccumulator::new()),
                Box::new(Mix9svqAccumulator::new()),
            ],
            move_cache: [
                Vec::with_capacity(BOARD_AREA),
                Vec::with_capacity(BOARD_AREA),
            ],
            synced_history: None,
            pending_balance: 0,
        };
        eval.init_empty_board();
        eval
    }

    pub fn init_empty_board(&mut self) {
        self.move_cache[0].clear();
        self.move_cache[1].clear();
        self.accumulator[0].clear(&self.weight[0]);
        self.accumulator[1].clear(&self.weight[1]);
        self.synced_history = Some(Vec::new());
        self.pending_balance = 0;
    }

    /// Stable identity hash of the currently loaded black/white weight pair,
    /// used by `AlphaBetaSearcher` to isolate TT keys across weight switches.
    pub fn weight_identity_hash(&self) -> u64 {
        fn mix64(mut x: u64) -> u64 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x ^= x >> 27;
            x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        }
        let mut h = 0x243F_6A88_85A3_08D3_u64;
        for w in &self.weight {
            let ptr = Arc::as_ptr(w) as usize as u64;
            h = mix64(h ^ ptr);
            h = mix64(h ^ u64::from(w.header.arch_hash));
            h = mix64(h ^ u64::from(w.header.rule_mask));
            h = mix64(h ^ u64::from(w.header.boardsize_mask));
        }
        h
    }

    pub fn sync_with_board(&mut self, board: &Board) {
        let history = board.move_history();
        if !history.is_empty() || board.move_count() == 0 {
            let mut target = Vec::with_capacity(history.len());
            for played in history {
                if let Ok((x, y)) = move_to_xy(played.move_) {
                    target.push((played.side, x as u8, y as u8));
                }
            }
            if self.pending_balance == 0 {
                if let Some(prev) = self.synced_history.take() {
                    let lcp = prev
                        .iter()
                        .zip(target.iter())
                        .take_while(|(a, b)| a == b)
                        .count();
                    if lcp == prev.len() && lcp == target.len() {
                        self.synced_history = Some(prev);
                        return;
                    }
                    for &(side, x, y) in prev[lcp..].iter().rev() {
                        self.queue_cache(side_to_color(side), x as usize, y as usize, true);
                    }
                    for &(side, x, y) in &target[lcp..] {
                        self.queue_cache(side_to_color(side), x as usize, y as usize, false);
                    }
                    self.synced_history = Some(target);
                    return;
                }
            }
            self.init_empty_board();
            for &(side, x, y) in &target {
                self.queue_cache(side_to_color(side), x as usize, y as usize, false);
            }
            self.synced_history = Some(target);
            self.pending_balance = 0;
            return;
        }

        // Fallback for boards populated directly via `grid_rows_mut()` without move_history.
        self.init_empty_board();
        self.synced_history = None;
        let grid = board.grid_rows();
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                let cell = grid[y][x];
                if cell != EMPTY {
                    self.queue_cache(side_to_color(cell), x, y, false);
                }
            }
        }
        self.pending_balance = 0;
    }

    #[inline(always)]
    pub fn before_move(&mut self, side: Side, x: usize, y: usize) {
        self.pending_balance += 1;
        self.queue_cache(side_to_color(side), x, y, false);
    }

    #[inline(always)]
    pub fn after_undo(&mut self, side: Side, x: usize, y: usize) {
        self.pending_balance -= 1;
        self.queue_cache(side_to_color(side), x, y, true);
    }

    #[inline(always)]
    fn queue_cache(&mut self, color: u8, x: usize, y: usize, is_undo: bool) {
        let entry = if is_undo {
            MoveCacheEntry {
                old_color: color,
                new_color: COLOR_EMPTY,
                x: x as u8,
                y: y as u8,
            }
        } else {
            MoveCacheEntry {
                old_color: COLOR_EMPTY,
                new_color: color,
                x: x as u8,
                y: y as u8,
            }
        };
        for c in 0..2 {
            let cache = &mut self.move_cache[c];
            if cache.last().is_some_and(|&last| entry.is_contrary(last)) {
                cache.pop();
            } else {
                cache.push(entry);
            }
        }
    }

    fn flush_move_cache(&mut self, slot: usize) {
        let cache = &mut self.move_cache[slot];
        let len = cache.len();
        if len == 0 {
            return;
        }
        let acc = &mut self.accumulator[slot];
        let w = &self.weight[slot];
        for i in 0..len {
            // SAFETY: `i < len == cache.len()`, and `cache` is not mutated in the loop.
            let entry = unsafe { *cache.get_unchecked(i) };
            if entry.old_color == COLOR_EMPTY {
                let piece = if slot == 1 {
                    1 - entry.new_color
                } else {
                    entry.new_color
                };
                #[cfg(test)]
                test_counters::PLAY_MOVE_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                acc.play_move(w, piece, entry.x as usize, entry.y as usize);
            } else {
                acc.undo();
            }
        }
        cache.clear();
    }

    /// Evaluate the current position from `side_to_move`'s perspective, returning
    /// `(raw_layer3i32, (win_logit, loss_logit, draw_logit), value_cp)`.
    pub fn evaluate_value_detailed(
        &mut self,
        side_to_move: Side,
    ) -> ([i32; 4], (f32, f32, f32), i32) {
        let slot = side_to_color(side_to_move) as usize;
        self.flush_move_cache(slot);
        let (raw, logits) = self.accumulator[slot].evaluate_value_logits(&self.weight[slot]);
        let val = logits_to_value(logits.0, logits.1, logits.2);
        (raw, logits, val)
    }

    /// Evaluate the current position from `side_to_move`'s perspective in centipawns.
    #[inline]
    pub fn evaluate_value(&mut self, side_to_move: Side) -> i32 {
        #[cfg(test)]
        test_counters::VALUE_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let slot = side_to_color(side_to_move) as usize;
        self.flush_move_cache(slot);
        let (_, (w, l, d)) = self.accumulator[slot].evaluate_value_logits(&self.weight[slot]);
        logits_to_value(w, l, d)
    }

    /// Evaluate policy logits for `side_to_move` on an explicit candidate slice.
    pub fn evaluate_policy_moves(
        &mut self,
        side_to_move: Side,
        moves: &[crate::types::Move],
        out_scores: &mut [f32],
    ) {
        #[cfg(test)]
        {
            test_counters::POLICY_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            test_counters::POLICY_CANDS
                .fetch_add(moves.len() as u64, std::sync::atomic::Ordering::Relaxed);
        }
        let slot = side_to_color(side_to_move) as usize;
        self.flush_move_cache(slot);
        self.accumulator[slot].evaluate_policy_moves(&self.weight[slot], moves, out_scores);
    }

    /// Evaluate policy logits for `side_to_move` on every cell flagged in `compute_mask`.
    pub fn evaluate_policy(
        &mut self,
        side_to_move: Side,
        compute_mask: &[bool; BOARD_AREA],
        out_policy: &mut [f32; BOARD_AREA],
    ) {
        let slot = side_to_color(side_to_move) as usize;
        self.flush_move_cache(slot);
        self.accumulator[slot].evaluate_policy(&self.weight[slot], compute_mask, out_policy);
    }

    /// Verify scalar vs SIMD (AVX2/AVX-VNNI on x86_64, NEON/DOTPROD on aarch64) bit-exactness.
    #[cfg(test)]
    pub(crate) fn set_simd_mode(&mut self, base_simd: bool, dotprod_simd: bool) {
        for acc in &mut self.accumulator {
            acc.use_avx2 = base_simd;
            acc.use_avxvnni = dotprod_simd;
            acc.use_neon = base_simd;
            acc.use_neon_dotprod = dotprod_simd;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::constants::WHITE;
    use std::path::PathBuf;

    fn local_mix9svq_dir() -> Option<PathBuf> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("opponent/rapfi-networks/mix9svq");
        dir.is_dir().then_some(dir)
    }

    #[test]
    fn mix9svq_avx2_matches_scalar_bit_exact_on_renju_and_freestyle() {
        let Some(dir) = local_mix9svq_dir() else {
            return;
        };
        let fs_w = Arc::new(
            Mix9svqWeight::load_from_lz4_file(
                &dir.join("mix9svqfreestyle_bsmix.bin.lz4"),
                RuleSet::Freestyle,
            )
            .expect("freestyle weight loads"),
        );
        let mut ev_vnni = Mix9svqEvaluator::new(fs_w.clone(), fs_w.clone());
        let mut ev_avx = Mix9svqEvaluator::new(fs_w.clone(), fs_w.clone());
        ev_avx.set_simd_mode(has_avx2() || has_neon(), false);
        let mut ev_sca = Mix9svqEvaluator::new(fs_w.clone(), fs_w);
        ev_sca.set_simd_mode(false, false);

        let moves = [
            (7, 7, BLACK),
            (8, 7, WHITE),
            (7, 8, BLACK),
            (6, 6, WHITE),
            (8, 8, BLACK),
            (9, 9, WHITE),
            (6, 8, BLACK),
            (5, 8, WHITE),
        ];
        let mut board = Board::new();
        for (x, y, s) in moves {
            board.play(xy_to_move(x, y).unwrap(), Some(s)).unwrap();
            ev_vnni.before_move(s, x, y);
            ev_avx.before_move(s, x, y);
            ev_sca.before_move(s, x, y);
        }

        let (raw_v, log_v, val_v) = ev_vnni.evaluate_value_detailed(BLACK);
        let (raw_a, log_a, val_a) = ev_avx.evaluate_value_detailed(BLACK);
        let (raw_s, log_s, val_s) = ev_sca.evaluate_value_detailed(BLACK);
        assert_eq!(
            raw_a, raw_s,
            "AVX2 and scalar layer3i32 must be bit-identical"
        );
        assert_eq!(
            raw_v, raw_s,
            "AVX-VNNI and scalar layer3i32 must be bit-identical"
        );
        assert_eq!(val_a, val_s);
        assert_eq!(val_v, val_s);
        assert!((log_a.0 - log_s.0).abs() < 1e-6);
        assert!((log_v.0 - log_s.0).abs() < 1e-6);

        let mut mask = [false; BOARD_AREA];
        let mut cand_moves = Vec::new();
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                if board.grid_rows()[y][x] == EMPTY {
                    let idx = y * BOARD_SIZE + x;
                    mask[idx] = true;
                    cand_moves.push(idx as crate::types::Move);
                }
            }
        }
        let mut pol_a = [0.0_f32; BOARD_AREA];
        let mut pol_s = [0.0_f32; BOARD_AREA];
        ev_avx.evaluate_policy(BLACK, &mask, &mut pol_a);
        ev_sca.evaluate_policy(BLACK, &mask, &mut pol_s);
        let mut pm_v = vec![0.0_f32; cand_moves.len()];
        let mut pm_a = vec![0.0_f32; cand_moves.len()];
        let mut pm_s = vec![0.0_f32; cand_moves.len()];
        ev_vnni.evaluate_policy_moves(BLACK, &cand_moves, &mut pm_v);
        ev_avx.evaluate_policy_moves(BLACK, &cand_moves, &mut pm_a);
        ev_sca.evaluate_policy_moves(BLACK, &cand_moves, &mut pm_s);
        for (i, &m) in cand_moves.iter().enumerate() {
            let idx = m as usize;
            assert!(
                (pol_a[idx] - pol_s[idx]).abs() < 1e-4,
                "policy mismatch at idx {idx}: avx={} scalar={}",
                pol_a[idx],
                pol_s[idx]
            );
            assert!(
                (pm_v[i] - pm_s[i]).abs() < 1e-4,
                "policy_moves vnni mismatch at idx {idx}: vnni={} scalar={}",
                pm_v[i],
                pm_s[i]
            );
            assert!(
                (pm_a[i] - pm_s[i]).abs() < 1e-4,
                "policy_moves avx2 mismatch at idx {idx}: avx2={} scalar={}",
                pm_a[i],
                pm_s[i]
            );
        }

        // Verify LCP incremental sync_with_board across: same board, +2 moves,
        // undo 3 + divergent 2 moves, and manual before_move imbalance.
        let mut ev_sync =
            Mix9svqEvaluator::new(ev_vnni.weight[0].clone(), ev_vnni.weight[1].clone());
        ev_sync.sync_with_board(&board);
        assert_eq!(ev_sync.evaluate_value_detailed(BLACK).0, raw_s);
        // Same board again (no-op fast path)
        ev_sync.sync_with_board(&board);
        assert_eq!(ev_sync.evaluate_value_detailed(BLACK).0, raw_s);
        // +2 moves
        board.play(xy_to_move(7, 9).unwrap(), Some(BLACK)).unwrap();
        board.play(xy_to_move(6, 9).unwrap(), Some(WHITE)).unwrap();
        ev_sync.sync_with_board(&board);
        let mut ev_ref =
            Mix9svqEvaluator::new(ev_vnni.weight[0].clone(), ev_vnni.weight[1].clone());
        ev_ref.sync_with_board(&board);
        assert_eq!(
            ev_sync.evaluate_value_detailed(BLACK).0,
            ev_ref.evaluate_value_detailed(BLACK).0
        );
        // Undo 3 moves and play 2 divergent moves
        for _ in 0..3 {
            board.undo().unwrap();
        }
        board.play(xy_to_move(9, 7).unwrap(), Some(WHITE)).unwrap();
        board.play(xy_to_move(10, 7).unwrap(), Some(BLACK)).unwrap();
        ev_sync.sync_with_board(&board);
        let mut ev_ref2 =
            Mix9svqEvaluator::new(ev_vnni.weight[0].clone(), ev_vnni.weight[1].clone());
        ev_ref2.sync_with_board(&board);
        assert_eq!(
            ev_sync.evaluate_value_detailed(WHITE).0,
            ev_ref2.evaluate_value_detailed(WHITE).0
        );
        // Manual before_move imbalance forces full resync
        ev_sync.before_move(WHITE, 4, 4);
        ev_sync.sync_with_board(&board);
        assert_eq!(
            ev_sync.evaluate_value_detailed(WHITE).0,
            ev_ref2.evaluate_value_detailed(WHITE).0
        );

        // Microbenchmark AVX2 vs AVX-VNNI
        let t0 = std::time::Instant::now();
        let mut sink = 0_i64;
        for _ in 0..2000 {
            ev_avx.before_move(BLACK, 5, 7);
            sink += ev_avx.evaluate_value(WHITE) as i64;
            ev_avx.after_undo(BLACK, 5, 7);
        }
        let us_avx2_me = t0.elapsed().as_secs_f64() * 1e6 / 2000.0;
        let t1 = std::time::Instant::now();
        for _ in 0..2000 {
            sink += ev_avx.evaluate_value(BLACK) as i64;
        }
        let us_avx2_e = t1.elapsed().as_secs_f64() * 1e6 / 2000.0;
        let t2 = std::time::Instant::now();
        for _ in 0..2000 {
            ev_vnni.before_move(BLACK, 5, 7);
            sink += ev_vnni.evaluate_value(WHITE) as i64;
            ev_vnni.after_undo(BLACK, 5, 7);
        }
        let us_vnni_me = t2.elapsed().as_secs_f64() * 1e6 / 2000.0;
        let t3 = std::time::Instant::now();
        for _ in 0..2000 {
            sink += ev_vnni.evaluate_value(BLACK) as i64;
        }
        let us_vnni_e = t3.elapsed().as_secs_f64() * 1e6 / 2000.0;
        eprintln!(
            "mix9svq bench: AVX2 eval={us_avx2_e:.2}us move+eval={us_avx2_me:.2}us | AVX-VNNI eval={us_vnni_e:.2}us move+eval={us_vnni_me:.2}us (has_avxvnni={}) sink={sink}",
            has_avxvnni()
        );
        // Pin bit-exact match against Rapfi C++ oracle (`opponent/rapfi/Rapfi/eval/mix9svqnnue.cpp`)
        assert_eq!(
            val_a, 994,
            "Freestyle 8-ply BLACK must match Rapfi C++ oracle (994)"
        );
        let mut top: Vec<(f32, usize, usize)> = (0..BOARD_AREA)
            .filter(|&i| mask[i])
            .map(|i| (pol_a[i], i % BOARD_SIZE, i / BOARD_SIZE))
            .collect();
        top.sort_by(|a, b| b.0.total_cmp(&a.0));
        assert_eq!((top[0].1, top[0].2), (7, 9));
        assert!((top[0].0 - 12.929_859).abs() < 1e-4);

        // Also pin Renju 7-ply WHITE (val=80) and 8-ply BLACK (val=225) vs Rapfi C++ oracle
        let mut ev_renju = crate::nnue::load_mix9svq_evaluator_for_rule(&dir, RuleSet::Renju)
            .expect("renju weights load");
        let renju_moves = [
            (7, 7, BLACK),
            (7, 6, WHITE),
            (8, 6, BLACK),
            (6, 8, WHITE),
            (6, 6, BLACK),
            (8, 8, WHITE),
            (9, 7, BLACK),
        ];
        for (x, y, s) in renju_moves {
            ev_renju.before_move(s, x, y);
        }
        assert_eq!(
            ev_renju.evaluate_value(WHITE),
            80,
            "Renju 7-ply WHITE must match Rapfi C++ oracle (80)"
        );
        ev_renju.before_move(WHITE, 5, 7);
        assert_eq!(
            ev_renju.evaluate_value(BLACK),
            225,
            "Renju 8-ply BLACK must match Rapfi C++ oracle (225)"
        );
    }
}
