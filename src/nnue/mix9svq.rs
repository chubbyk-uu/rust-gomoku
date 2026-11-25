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
    dot2_u7i8_scalar, has_avx2, has_avxvnni, linear_i16_chunk2_prerelu_scalar,
    linear_i8_chunk4_scalar, linear_i8_row4_scalar, mulhi_i16, preprocess_dynamic_weight_linear,
    preprocess_linear,
};
use crate::rules::RuleSet;
use crate::types::Side;

#[cfg(target_arch = "x86_64")]
use crate::nnue::simd::{
    avg_pool_2x2_u8_avx2, commit_conv_cell_avx2, commit_vsum_avx2, crelu_i32_to_i16_avx2,
    crelu_i32_to_i8_avx2, dot2_u7i8_avx2, linear_i16_chunk2_prerelu_avx2, linear_i8_chunk4_avx2,
    linear_i8_chunk4_avxvnni, linear_i8_row4_avx2, linear_i8_row4_avxvnni, policy_dot16_avx2,
    update_point_avx2,
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

/// Shared read-only `Mix9svq` weights (~18 MiB per side).
///
/// All tensor buffers are private and shape-validated at load/construction time
/// so safe evaluation entry points always pass exact-dimension slices to SIMD
/// kernels.
#[derive(Debug)]
pub struct Mix9svqWeight {
    pub header: StandardHeader,
    codebook: [Box<[[i16; FEATURE_DIM]; CODEBOOK_SIZE]>; 2],
    mapping_index: [Box<[u16; SHAPE_NUM]>; 2],
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
        let mut codebook: [Box<[[i16; FEATURE_DIM]; CODEBOOK_SIZE]>; 2] = [
            vec![[0_i16; FEATURE_DIM]; CODEBOOK_SIZE]
                .into_boxed_slice()
                .try_into()
                .expect("exact codebook length"),
            vec![[0_i16; FEATURE_DIM]; CODEBOOK_SIZE]
                .into_boxed_slice()
                .try_into()
                .expect("exact codebook length"),
        ];
        for cb in &mut codebook {
            let mut stream = PackedSigned10Stream::new();
            for row in cb.iter_mut() {
                for v in row.iter_mut() {
                    *v = stream.next(&mut cur)?;
                }
            }
        }

        let mut mapping_index: [Box<[u16; SHAPE_NUM]>; 2] = [
            vec![0_u16; SHAPE_NUM]
                .into_boxed_slice()
                .try_into()
                .expect("exact mapping_index length"),
            vec![0_u16; SHAPE_NUM]
                .into_boxed_slice()
                .try_into()
                .expect("exact mapping_index length"),
        ];
        for mi in &mut mapping_index {
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

/// Version-stacked incremental accumulator for a single perspective (Black or White).
#[derive(Clone, Debug)]
pub struct Mix9svqAccumulator {
    value_sum_table: Vec<ValueSum>,
    version_change_num: Vec<ChangeNum>,
    version_inner_index: Vec<u16>,
    version_outer_index: Vec<u16>,
    index_table: Vec<[u32; 4]>,
    map_sum: Vec<[i16; FEATURE_DIM]>,
    map_conv: Vec<[i16; FEAT_DWCONV_DIM]>,
    current_version: i32,
    group_index: [usize; BOARD_SIZE],
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    use_avx2: bool,
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    use_avxvnni: bool,
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
            map_sum: vec![[0_i16; FEATURE_DIM]; MAX_INNER_CHANGES_15],
            map_conv: vec![[0_i16; FEAT_DWCONV_DIM]; MAX_OUTER_CHANGES_15],
            current_version: -1,
            group_index,
            use_avx2: has_avx2(),
            use_avxvnni: has_avxvnni(),
        }
    }

    pub fn clear(&mut self, w: &Mix9svqWeight) {
        if self.current_version == -1 {
            for i in 0..OUTER_BOARD_AREA {
                self.map_conv[i] = w.feature_dwconv_bias;
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
                    self.map_sum[inner_idx] = sum;

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
                self.map_conv[new_map_conv_idx] = [0; FEAT_DWCONV_DIM];
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
                // slices in `self.map_sum` never alias.
                unsafe {
                    let old_ptr = self.map_sum.as_ptr().add(c.old_map_idx);
                    let new_ptr = self.map_sum.as_mut_ptr().add(c.new_map_idx);
                    update_point_avx2(
                        &w.codebook[c.mapping_idx][c.old_cb_idx],
                        &w.codebook[c.mapping_idx][c.new_cb_idx],
                        &*old_ptr,
                        &mut *new_ptr,
                        &w.feature_dwconv_weight,
                        &mut self.map_conv,
                        &outer_indices,
                        &mut vsum_group[gi][gj],
                    );
                }
                continue;
            }

            let old_cb = &w.codebook[c.mapping_idx][c.old_cb_idx];
            let new_cb = &w.codebook[c.mapping_idx][c.new_cb_idx];
            let old_sum = self.map_sum[c.old_map_idx];
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
            self.map_sum[c.new_map_idx] = new_sum;

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
                        let old_ptr = self.map_conv.as_ptr().add(old_conv_idx);
                        let new_ptr = self.map_conv.as_mut_ptr().add(commit_idx);
                        commit_conv_cell_avx2(&*old_ptr, &mut *new_ptr, &mut vsum_group[gi][gj]);
                    }
                    commit_idx += 1;
                    continue;
                }

                let old_v = self.map_conv[old_conv_idx];
                let delta_v = self.map_conv[commit_idx];
                let mut new_v = [0_i16; FEAT_DWCONV_DIM];
                for ch in 0..FEAT_DWCONV_DIM {
                    let nv = old_v[ch].wrapping_add(delta_v[ch]);
                    new_v[ch] = nv;
                    vsum_group[gi][gj][ch] += (nv.max(0) as i32) - (old_v[ch].max(0) as i32);
                }
                self.map_conv[commit_idx] = new_v;
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
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM + VALUE_DIM * 4];
        let mut global_i8 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(&mut global_i8, &value_sum.global);
        layer0[..FEATURE_DIM].copy_from_slice(&global_i8);

        let mut group0 = [[[0_i8; FEATURE_DIM]; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_avx2::<FEATURE_DIM, 5, true>(
                    &mut group0[i][j],
                    &value_sum.group[i][j],
                );
            }
        }

        let mut group1 = [[[0_i8; VALUE_DIM]; 3]; 3];
        star_block_avx2(&mut group1[0][0], &group0[0][0], &b.value_corner);
        star_block_avx2(&mut group1[0][2], &group0[0][2], &b.value_corner);
        star_block_avx2(&mut group1[2][0], &group0[2][0], &b.value_corner);
        star_block_avx2(&mut group1[2][2], &group0[2][2], &b.value_corner);

        star_block_avx2(&mut group1[0][1], &group0[0][1], &b.value_edge);
        star_block_avx2(&mut group1[1][0], &group0[1][0], &b.value_edge);
        star_block_avx2(&mut group1[1][2], &group0[1][2], &b.value_edge);
        star_block_avx2(&mut group1[2][1], &group0[2][1], &b.value_edge);

        star_block_avx2(&mut group1[1][1], &group0[1][1], &b.value_center);

        let mut q00 = [0_i8; VALUE_DIM];
        let mut q01 = [0_i8; VALUE_DIM];
        let mut q10 = [0_i8; VALUE_DIM];
        let mut q11 = [0_i8; VALUE_DIM];
        avg_pool_2x2_u8_avx2(&mut q00, &mut q01, &mut q10, &mut q11, &group1);

        let mut quad_out = [[0_i8; VALUE_DIM]; 4];
        star_block_avx2(&mut quad_out[0], &q00, &b.value_quad);
        star_block_avx2(&mut quad_out[1], &q01, &b.value_quad);
        star_block_avx2(&mut quad_out[2], &q10, &b.value_quad);
        star_block_avx2(&mut quad_out[3], &q11, &b.value_quad);
        for q in 0..4 {
            let off = FEATURE_DIM + q * VALUE_DIM;
            layer0[off..off + VALUE_DIM].copy_from_slice(&quad_out[q]);
        }

        let mut l1_i32 = [0_i32; VALUE_DIM];
        let mut l1 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_avx2::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, false>(
            &mut l1_i32,
            &layer0,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut l1, &l1_i32);

        let mut l2_i32 = [0_i32; VALUE_DIM];
        let mut l2 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_avx2::<VALUE_DIM, VALUE_DIM, false>(
            &mut l2_i32,
            &l1,
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut l2, &l2_i32);

        let mut l3_i32 = [0_i32; 4];
        let l3_bias: [i32; 4] = [
            b.value_l3.bias[0],
            b.value_l3.bias[1],
            b.value_l3.bias[2],
            b.value_l3.bias[3],
        ];
        linear_i8_row4_avx2::<VALUE_DIM>(&mut l3_i32, &l2, &b.value_l3.weight, &l3_bias);
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
    #[target_feature(enable = "avx2,avxvnni")]
    unsafe fn evaluate_value_logits_avxvnni(
        &self,
        w: &Mix9svqWeight,
    ) -> ([i32; 4], (f32, f32, f32)) {
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM + VALUE_DIM * 4];
        let mut global_i8 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(&mut global_i8, &value_sum.global);
        layer0[..FEATURE_DIM].copy_from_slice(&global_i8);

        let mut group0 = [[[0_i8; FEATURE_DIM]; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                crelu_i32_to_i8_avx2::<FEATURE_DIM, 5, true>(
                    &mut group0[i][j],
                    &value_sum.group[i][j],
                );
            }
        }

        let mut group1 = [[[0_i8; VALUE_DIM]; 3]; 3];
        star_block_avxvnni(&mut group1[0][0], &group0[0][0], &b.value_corner);
        star_block_avxvnni(&mut group1[0][2], &group0[0][2], &b.value_corner);
        star_block_avxvnni(&mut group1[2][0], &group0[2][0], &b.value_corner);
        star_block_avxvnni(&mut group1[2][2], &group0[2][2], &b.value_corner);

        star_block_avxvnni(&mut group1[0][1], &group0[0][1], &b.value_edge);
        star_block_avxvnni(&mut group1[1][0], &group0[1][0], &b.value_edge);
        star_block_avxvnni(&mut group1[1][2], &group0[1][2], &b.value_edge);
        star_block_avxvnni(&mut group1[2][1], &group0[2][1], &b.value_edge);

        star_block_avxvnni(&mut group1[1][1], &group0[1][1], &b.value_center);

        let mut q00 = [0_i8; VALUE_DIM];
        let mut q01 = [0_i8; VALUE_DIM];
        let mut q10 = [0_i8; VALUE_DIM];
        let mut q11 = [0_i8; VALUE_DIM];
        avg_pool_2x2_u8_avx2(&mut q00, &mut q01, &mut q10, &mut q11, &group1);

        let mut quad_out = [[0_i8; VALUE_DIM]; 4];
        star_block_avxvnni(&mut quad_out[0], &q00, &b.value_quad);
        star_block_avxvnni(&mut quad_out[1], &q01, &b.value_quad);
        star_block_avxvnni(&mut quad_out[2], &q10, &b.value_quad);
        star_block_avxvnni(&mut quad_out[3], &q11, &b.value_quad);
        for q in 0..4 {
            let off = FEATURE_DIM + q * VALUE_DIM;
            layer0[off..off + VALUE_DIM].copy_from_slice(&quad_out[q]);
        }

        let mut l1_i32 = [0_i32; VALUE_DIM];
        let mut l1 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_avxvnni::<VALUE_DIM, { FEATURE_DIM + VALUE_DIM * 4 }, false>(
            &mut l1_i32,
            &layer0,
            &b.value_l1.weight,
            &b.value_l1.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut l1, &l1_i32);

        let mut l2_i32 = [0_i32; VALUE_DIM];
        let mut l2 = [0_i8; VALUE_DIM];
        linear_i8_chunk4_avxvnni::<VALUE_DIM, VALUE_DIM, false>(
            &mut l2_i32,
            &l1,
            &b.value_l2.weight,
            &b.value_l2.bias,
        );
        crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(&mut l2, &l2_i32);

        let mut l3_i32 = [0_i32; 4];
        let l3_bias: [i32; 4] = [
            b.value_l3.bias[0],
            b.value_l3.bias[1],
            b.value_l3.bias[2],
            b.value_l3.bias[3],
        ];
        linear_i8_row4_avxvnni::<VALUE_DIM>(&mut l3_i32, &l2, &b.value_l3.weight, &l3_bias);
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
                &self.map_conv[mc_idx],
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
        let value_sum = &self.value_sum_table[self.current_version as usize];
        let b = &w.bucket;

        let mut layer0 = [0_i8; FEATURE_DIM];
        crelu_i32_to_i8_avx2::<FEATURE_DIM, 8, true>(&mut layer0, &value_sum.global);

        let mut l1_i32 = [0_i32; POLICY_DIM * 2];
        let mut l1 = [0_i8; POLICY_DIM * 2];
        linear_i8_chunk4_avxvnni::<{ POLICY_DIM * 2 }, FEATURE_DIM, false>(
            &mut l1_i32,
            &layer0,
            &b.policy_pwconv_l1.weight,
            &b.policy_pwconv_l1.bias,
        );
        crelu_i32_to_i8_avx2::<{ POLICY_DIM * 2 }, 7, false>(&mut l1, &l1_i32);

        const L2_OUT: usize = POLICY_PWCONV_DIM * POLICY_DIM + POLICY_PWCONV_DIM;
        let mut l2_i32 = [0_i32; L2_OUT];
        linear_i8_chunk4_avxvnni::<L2_OUT, { POLICY_DIM * 2 }, false>(
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
        for (dst, &m) in out_scores.iter_mut().zip(moves.iter()) {
            let idx = m as usize;
            let y = idx / BOARD_SIZE;
            let x = idx % BOARD_SIZE;
            let outer_idx = (y + 1) * OUTER_BOARD_SIZE + (x + 1);
            let mc_idx = self.version_outer_index[outer_ver_base + outer_idx] as usize;
            linear_i16_chunk2_prerelu_avx2::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut p_i32,
                &self.map_conv[mc_idx],
                &pw_w,
                &pw_b,
            );
            *dst = policy_dot16_avx2(&p_i32, &b.policy_output_weight, b.policy_output_bias);
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
        for (dst, &m) in out_scores.iter_mut().zip(moves.iter()) {
            let idx = m as usize;
            let y = idx / BOARD_SIZE;
            let x = idx % BOARD_SIZE;
            let outer_idx = (y + 1) * OUTER_BOARD_SIZE + (x + 1);
            let mc_idx = self.version_outer_index[outer_ver_base + outer_idx] as usize;
            linear_i16_chunk2_prerelu_avx2::<POLICY_PWCONV_DIM, POLICY_DIM>(
                &mut p_i32,
                &self.map_conv[mc_idx],
                &pw_w,
                &pw_b,
            );
            *dst = policy_dot16_avx2(&p_i32, &b.policy_output_weight, b.policy_output_bias);
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
                    &self.map_conv[mc_idx],
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
                    &self.map_conv[mc_idx],
                    &pw_w,
                    &pw_b,
                );
                out_policy[inner_idx] =
                    policy_dot16_avx2(&p_i32, &b.policy_output_weight, b.policy_output_bias);
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
    let mut up_i32 = [0_i32; VALUE_DIM * 2];
    let mut up1 = [0_i8; VALUE_DIM * 2];
    let mut up2 = [0_i8; VALUE_DIM * 2];

    linear_i8_chunk4_avx2::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, false>(&mut up1, &up_i32);

    linear_i8_chunk4_avx2::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, true>(&mut up2, &up_i32);

    let mut dotsum = [0_i8; VALUE_DIM];
    dot2_u7i8_avx2::<VALUE_DIM, 7>(&mut dotsum, &up1, &up2);

    let mut out_i32 = [0_i32; VALUE_DIM];
    linear_i8_chunk4_avx2::<VALUE_DIM, VALUE_DIM, true>(
        &mut out_i32,
        &dotsum,
        &w.down.weight,
        &w.down.bias,
    );
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(output, &out_i32);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,avxvnni")]
unsafe fn star_block_avxvnni(
    output: &mut [i8; VALUE_DIM],
    input: &[i8; FEATURE_DIM],
    w: &StarBlockWeight<VALUE_DIM, FEATURE_DIM>,
) {
    let mut up_i32 = [0_i32; VALUE_DIM * 2];
    let mut up1 = [0_i8; VALUE_DIM * 2];
    let mut up2 = [0_i8; VALUE_DIM * 2];

    linear_i8_chunk4_avxvnni::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up1.weight,
        &w.up1.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, false>(&mut up1, &up_i32);

    linear_i8_chunk4_avxvnni::<{ VALUE_DIM * 2 }, FEATURE_DIM, false>(
        &mut up_i32,
        input,
        &w.up2.weight,
        &w.up2.bias,
    );
    crelu_i32_to_i8_avx2::<{ VALUE_DIM * 2 }, 7, true>(&mut up2, &up_i32);

    let mut dotsum = [0_i8; VALUE_DIM];
    dot2_u7i8_avx2::<VALUE_DIM, 7>(&mut dotsum, &up1, &up2);

    let mut out_i32 = [0_i32; VALUE_DIM];
    linear_i8_chunk4_avxvnni::<VALUE_DIM, VALUE_DIM, true>(
        &mut out_i32,
        &dotsum,
        &w.down.weight,
        &w.down.signed_bias,
    );
    crelu_i32_to_i8_avx2::<VALUE_DIM, 7, false>(output, &out_i32);
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
        if cache.is_empty() {
            return;
        }
        let acc = &mut self.accumulator[slot];
        let w = &self.weight[slot];
        for entry in cache.drain(..) {
            if entry.old_color == COLOR_EMPTY {
                let piece = if slot == 1 {
                    1 - entry.new_color
                } else {
                    entry.new_color
                };
                acc.play_move(w, piece, entry.x as usize, entry.y as usize);
            } else {
                acc.undo();
            }
        }
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

    /// Verify scalar vs AVX2 vs AVX-VNNI bit-exactness (for tests).
    #[cfg(test)]
    pub(crate) fn set_simd_mode(&mut self, avx2: bool, avxvnni: bool) {
        for acc in &mut self.accumulator {
            acc.use_avx2 = avx2;
            acc.use_avxvnni = avxvnni;
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
        ev_avx.set_simd_mode(has_avx2(), false);
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
