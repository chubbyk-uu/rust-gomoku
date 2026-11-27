//! Bit-exact SIMD (AVX2 on x86_64) and scalar kernels matching Rapfi's
//! `Evaluation::simd` primitives for `Mix9svq`.
//!
//! Every kernel matches Rapfi's exact integer arithmetic:
//! - `crelu_i32_to_i8`: saturate i32->i16, arithmetic shift right by log2(Divisor),
//!   saturate i16->i8, optional ReLU (`max(_, 0)`).
//! - `dot2_u7i8`: pairwise `u7 * i8 + u7 * i8` (exact in i16), arithmetic shift
//!   right by log2(Divisor), saturate i16->i8.
//! - `avg_pool_2x2_u8`: unsigned rounding-up average `((a + b + 1) >> 1)` matching
//!   `_mm256_avg_epu8` / ARM `vrhaddq_u8`.
//! - `linear_i8_chunk4`: scrambled `[InSize/4][OutSize][4]` i8 affine into i32.
//! - `linear_i8_row4`: row-major `[4][InSize]` i8 affine into i32 (for `value_l3`).
//! - `linear_i16_chunk2_prerelu`: scrambled `[InSize/2][OutSize][2]` i16 affine
//!   with ReLU applied to input before dot product (for policy dynamic pwconv).

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

/// Runtime check for AVX2 support on x86_64.
#[inline(always)]
pub fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Runtime check for AVX-VNNI support on x86_64 (implies AVX2).
#[inline(always)]
pub fn has_avxvnni() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("avxvnni")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Compile-time check for baseline ARMv8 NEON support on aarch64.
#[inline(always)]
pub fn has_neon() -> bool {
    cfg!(target_arch = "aarch64")
}

/// Runtime check for ARMv8.2+ NEON `dotprod` (`sdot`) support on aarch64.
#[inline(always)]
pub fn has_neon_dotprod() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        std::arch::is_aarch64_feature_detected!("dotprod")
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        false
    }
}

/// Scramble a row-major `[OutSize][InSize]` weight matrix into
/// `[InSize / CHUNK][OutSize][CHUNK]` for chunked SIMD affine kernels.
pub fn preprocess_linear<
    T: Copy + Default,
    const OUT: usize,
    const IN: usize,
    const CHUNK: usize,
>(
    weight: &mut [T],
) {
    assert_eq!(weight.len(), OUT * IN);
    assert_eq!(IN % CHUNK, 0);
    let mut scrambled = vec![T::default(); OUT * IN];
    let in_chunks = IN / CHUNK;
    for i in 0..(OUT * IN) {
        let offset = i % CHUNK;
        let idx_chunk = i / CHUNK;
        let col_chunk = idx_chunk % in_chunks;
        let row_chunk = i / IN;
        let transposed_idx_chunk = col_chunk * OUT + row_chunk;
        scrambled[transposed_idx_chunk * CHUNK + offset] = weight[i];
    }
    weight.copy_from_slice(&scrambled);
}

/// Scramble the leading `DYN_OUT * DYN_IN` rows of a hyper-network linear layer
/// (`weight: [TotalOut][HYPER_IN]`, `bias: [TotalOut]`) so its output directly
/// lands in `[DYN_IN / CHUNK][DYN_OUT][CHUNK]` layout.
pub fn preprocess_dynamic_weight_linear<
    const DYN_OUT: usize,
    const DYN_IN: usize,
    const HYPER_IN: usize,
    const CHUNK: usize,
>(
    weight: &mut [i8],
    bias: &mut [i32],
) {
    let n = DYN_OUT * DYN_IN;
    assert!(weight.len() >= n * HYPER_IN);
    assert!(bias.len() >= n);
    let in_chunks = DYN_IN / CHUNK;

    let mut w_scrambled = vec![0_i8; n * HYPER_IN];
    let mut b_scrambled = vec![0_i32; n];
    for i in 0..n {
        let offset = i % CHUNK;
        let idx_chunk = i / CHUNK;
        let col_chunk = idx_chunk % in_chunks;
        let row_chunk = i / DYN_IN;
        let dst_row = (col_chunk * DYN_OUT + row_chunk) * CHUNK + offset;
        w_scrambled[dst_row * HYPER_IN..(dst_row + 1) * HYPER_IN]
            .copy_from_slice(&weight[i * HYPER_IN..(i + 1) * HYPER_IN]);
        b_scrambled[dst_row] = bias[i];
    }
    weight[..n * HYPER_IN].copy_from_slice(&w_scrambled);
    bias[..n].copy_from_slice(&b_scrambled);
}

// ============================================================================
// Scalar reference implementations (exact bit-for-bit match to AVX2)
// ============================================================================

#[inline(always)]
fn sat_i32_to_i16(x: i32) -> i16 {
    x.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[inline(always)]
fn sat_i16_to_i8(x: i16) -> i8 {
    x.clamp(i8::MIN as i16, i8::MAX as i16) as i8
}

#[inline(always)]
pub fn mulhi_i16(a: i16, b: i16) -> i16 {
    (((a as i32) * (b as i32)) >> 16) as i16
}

#[inline(always)]
pub fn avg_u8(a: i8, b: i8) -> i8 {
    (((a as u8 as u16) + (b as u8 as u16) + 1) >> 1) as u8 as i8
}

#[inline(always)]
pub fn crelu_i32_to_i8_scalar<const SIZE: usize, const SHIFT: u32, const NO_RELU: bool>(
    output: &mut [i8; SIZE],
    input: &[i32; SIZE],
) {
    for i in 0..SIZE {
        let v16 = sat_i32_to_i16(input[i]) >> SHIFT;
        let mut v8 = sat_i16_to_i8(v16);
        if !NO_RELU && v8 < 0 {
            v8 = 0;
        }
        output[i] = v8;
    }
}

#[inline(always)]
pub fn crelu_i32_to_i16_scalar<const SIZE: usize, const SHIFT: u32, const NO_RELU: bool>(
    output: &mut [i16; SIZE],
    input: &[i32; SIZE],
) {
    for i in 0..SIZE {
        let shifted = if SHIFT > 0 {
            input[i] >> SHIFT
        } else {
            input[i]
        };
        let mut v16 = sat_i32_to_i16(shifted);
        if !NO_RELU && v16 < 0 {
            v16 = 0;
        }
        output[i] = v16;
    }
}

#[inline(always)]
pub fn dot2_u7i8_scalar<const OUT: usize, const SHIFT: u32>(
    output: &mut [i8; OUT],
    input_u7: &[i8],
    input_i8: &[i8],
) {
    for i in 0..OUT {
        let p0 = (input_u7[2 * i] as u8 as i32) * (input_i8[2 * i] as i32);
        let p1 = (input_u7[2 * i + 1] as u8 as i32) * (input_i8[2 * i + 1] as i32);
        let sum16 = sat_i32_to_i16(p0 + p1);
        output[i] = sat_i16_to_i8(sum16 >> SHIFT);
    }
}

#[inline(always)]
pub fn linear_i8_chunk4_scalar<const OUT: usize, const IN: usize, const SIGNED_INPUT: bool>(
    output: &mut [i32; OUT],
    input: &[i8],
    weight: &[i8],
    bias: &[i32],
) {
    output.copy_from_slice(&bias[..OUT]);
    let num_chunks = IN / 4;
    for c in 0..num_chunks {
        let in0 = if SIGNED_INPUT {
            input[4 * c] as i32
        } else {
            input[4 * c] as u8 as i32
        };
        let in1 = if SIGNED_INPUT {
            input[4 * c + 1] as i32
        } else {
            input[4 * c + 1] as u8 as i32
        };
        let in2 = if SIGNED_INPUT {
            input[4 * c + 2] as i32
        } else {
            input[4 * c + 2] as u8 as i32
        };
        let in3 = if SIGNED_INPUT {
            input[4 * c + 3] as i32
        } else {
            input[4 * c + 3] as u8 as i32
        };
        let w_base = c * OUT * 4;
        for o in 0..OUT {
            let wo = w_base + o * 4;
            output[o] += in0 * (weight[wo] as i32)
                + in1 * (weight[wo + 1] as i32)
                + in2 * (weight[wo + 2] as i32)
                + in3 * (weight[wo + 3] as i32);
        }
    }
}

#[inline(always)]
pub fn linear_i8_row4_scalar<const IN: usize>(
    output: &mut [i32; 4],
    input: &[i8; IN],
    weight: &[i8],
    bias: &[i32; 4],
) {
    for r in 0..4 {
        let mut acc = bias[r];
        let row = &weight[r * IN..(r + 1) * IN];
        for j in 0..IN {
            acc += (input[j] as u8 as i32) * (row[j] as i32);
        }
        output[r] = acc;
    }
}

#[inline(always)]
pub fn linear_i16_chunk2_prerelu_scalar<const OUT: usize, const IN: usize>(
    output: &mut [i32; OUT],
    input: &[i16],
    weight: &[i16],
    bias: &[i32; OUT],
) {
    output.copy_from_slice(bias);
    let num_chunks = IN / 2;
    for c in 0..num_chunks {
        let in0 = input[2 * c].max(0) as i32;
        let in1 = input[2 * c + 1].max(0) as i32;
        let w_base = c * OUT * 2;
        for o in 0..OUT {
            let wo = w_base + o * 2;
            output[o] += in0 * (weight[wo] as i32) + in1 * (weight[wo + 1] as i32);
        }
    }
}

// ============================================================================
// AVX2 SIMD Kernels (x86_64)
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn crelu_i32_to_i8_avx2<const SIZE: usize, const SHIFT: i32, const NO_RELU: bool>(
    output: &mut [i8; SIZE],
    input: &[i32; SIZE],
) {
    debug_assert_eq!(SIZE % 32, 0);
    let zero = _mm256_setzero_si256();
    let control = _mm256_set_epi32(7, 3, 6, 2, 5, 1, 4, 0);
    let batches = SIZE / 32;
    let in_ptr = input.as_ptr();
    let out_ptr = output.as_mut_ptr();
    for i in 0..batches {
        let base = i * 32;
        let in0 = _mm256_loadu_si256(in_ptr.add(base) as *const __m256i);
        let in1 = _mm256_loadu_si256(in_ptr.add(base + 8) as *const __m256i);
        let in2 = _mm256_loadu_si256(in_ptr.add(base + 16) as *const __m256i);
        let in3 = _mm256_loadu_si256(in_ptr.add(base + 24) as *const __m256i);
        let mut in01 = _mm256_packs_epi32(in0, in1);
        let mut in23 = _mm256_packs_epi32(in2, in3);
        if SHIFT > 0 {
            in01 = _mm256_srai_epi16(in01, SHIFT);
            in23 = _mm256_srai_epi16(in23, SHIFT);
        }
        let mut res = _mm256_packs_epi16(in01, in23);
        if !NO_RELU {
            res = _mm256_max_epi8(res, zero);
        }
        res = _mm256_permutevar8x32_epi32(res, control);
        _mm256_storeu_si256(out_ptr.add(base) as *mut __m256i, res);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn crelu_i32_to_i16_avx2<const SIZE: usize, const SHIFT: i32, const NO_RELU: bool>(
    output: &mut [i16; SIZE],
    input: &[i32; SIZE],
) {
    debug_assert_eq!(SIZE % 16, 0);
    let zero = _mm256_setzero_si256();
    let batches = SIZE / 16;
    let in_ptr = input.as_ptr();
    let out_ptr = output.as_mut_ptr();
    for i in 0..batches {
        let base = i * 16;
        let mut in0 = _mm256_loadu_si256(in_ptr.add(base) as *const __m256i);
        let mut in1 = _mm256_loadu_si256(in_ptr.add(base + 8) as *const __m256i);
        if SHIFT > 0 {
            in0 = _mm256_srai_epi32(in0, SHIFT);
            in1 = _mm256_srai_epi32(in1, SHIFT);
        }
        let mut res = _mm256_packs_epi32(in0, in1);
        if !NO_RELU {
            res = _mm256_max_epi16(res, zero);
        }
        res = _mm256_permute4x64_epi64(res, 0b11_01_10_00);
        _mm256_storeu_si256(out_ptr.add(base) as *mut __m256i, res);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn dot2_u7i8_avx2<const OUT: usize, const SHIFT: i32>(
    output: &mut [i8; OUT],
    input_u7: &[i8],
    input_i8: &[i8],
) {
    debug_assert_eq!(OUT % 32, 0);
    let batches = OUT / 32;
    let u7_ptr = input_u7.as_ptr();
    let i8_ptr = input_i8.as_ptr();
    let out_ptr = output.as_mut_ptr();
    for i in 0..batches {
        let in10 = _mm256_loadu_si256(u7_ptr.add(i * 64) as *const __m256i);
        let in11 = _mm256_loadu_si256(u7_ptr.add(i * 64 + 32) as *const __m256i);
        let in20 = _mm256_loadu_si256(i8_ptr.add(i * 64) as *const __m256i);
        let in21 = _mm256_loadu_si256(i8_ptr.add(i * 64 + 32) as *const __m256i);
        let d0 = _mm256_srai_epi16(_mm256_maddubs_epi16(in10, in20), SHIFT);
        let d1 = _mm256_srai_epi16(_mm256_maddubs_epi16(in11, in21), SHIFT);
        let packed = _mm256_permute4x64_epi64(_mm256_packs_epi16(d0, d1), 0b11_01_10_00);
        _mm256_storeu_si256(out_ptr.add(i * 32) as *mut __m256i, packed);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn dot4_u7i8_accum_avx2(acc: &mut __m256i, a: __m256i, b: __m256i, one: __m256i) {
    let p = _mm256_maddubs_epi16(a, b);
    let p32 = _mm256_madd_epi16(p, one);
    *acc = _mm256_add_epi32(*acc, p32);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn dot4_i8i8_accum_avx2(
    acc: &mut __m256i,
    a: __m256i,
    b: __m256i,
    highest_bit: __m256i,
    one: __m256i,
) {
    let msb = _mm256_and_si256(a, highest_bit);
    let low7 = _mm256_andnot_si256(highest_bit, a);
    let msb_p = _mm256_madd_epi16(_mm256_maddubs_epi16(msb, b), one);
    let low_p = _mm256_madd_epi16(_mm256_maddubs_epi16(low7, b), one);
    let diff = _mm256_sub_epi32(low_p, msb_p);
    *acc = _mm256_add_epi32(*acc, diff);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn linear_i8_chunk4_avx2<const OUT: usize, const IN: usize, const SIGNED_INPUT: bool>(
    output: &mut [i32; OUT],
    input: &[i8],
    weight: &[i8],
    bias: &[i32],
) {
    debug_assert_eq!(OUT % 8, 0);
    debug_assert_eq!(IN % 4, 0);
    let num_batches = OUT / 8;
    let num_chunks = IN / 4;
    let one = _mm256_set1_epi16(1);
    let highest_bit = _mm256_set1_epi8(-128_i8);
    let in32_ptr = input.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    let b_ptr = bias.as_ptr();
    let o_ptr = output.as_mut_ptr();

    let mut out_batch = 0;
    // Unrolled 8-YMM-register tile (64 outputs per tile) keeps all accumulators
    // in YMM registers with zero stack spills and hoists signed-input bit splits
    // once per input chunk instead of 8x per chunk.
    while out_batch + 8 <= num_batches {
        let boff = out_batch * 8;
        let mut a0 = _mm256_loadu_si256(b_ptr.add(boff) as *const __m256i);
        let mut a1 = _mm256_loadu_si256(b_ptr.add(boff + 8) as *const __m256i);
        let mut a2 = _mm256_loadu_si256(b_ptr.add(boff + 16) as *const __m256i);
        let mut a3 = _mm256_loadu_si256(b_ptr.add(boff + 24) as *const __m256i);
        let mut a4 = _mm256_loadu_si256(b_ptr.add(boff + 32) as *const __m256i);
        let mut a5 = _mm256_loadu_si256(b_ptr.add(boff + 40) as *const __m256i);
        let mut a6 = _mm256_loadu_si256(b_ptr.add(boff + 48) as *const __m256i);
        let mut a7 = _mm256_loadu_si256(b_ptr.add(boff + 56) as *const __m256i);

        let w_base = w_ptr.add(out_batch * 32);
        let stride = OUT * 4;
        if SIGNED_INPUT {
            for c in 0..num_chunks {
                let in0 = _mm256_set1_epi32(std::ptr::read_unaligned(in32_ptr.add(c)));
                let msb = _mm256_and_si256(in0, highest_bit);
                let low7 = _mm256_andnot_si256(highest_bit, in0);
                let wc = w_base.add(c * stride);
                macro_rules! step_signed {
                    ($acc:ident, $off:expr) => {{
                        let wv = _mm256_loadu_si256(wc.add($off) as *const __m256i);
                        let mp = _mm256_madd_epi16(_mm256_maddubs_epi16(msb, wv), one);
                        let lp = _mm256_madd_epi16(_mm256_maddubs_epi16(low7, wv), one);
                        $acc = _mm256_add_epi32($acc, _mm256_sub_epi32(lp, mp));
                    }};
                }
                step_signed!(a0, 0);
                step_signed!(a1, 32);
                step_signed!(a2, 64);
                step_signed!(a3, 96);
                step_signed!(a4, 128);
                step_signed!(a5, 160);
                step_signed!(a6, 192);
                step_signed!(a7, 224);
            }
        } else {
            for c in 0..num_chunks {
                let in0 = _mm256_set1_epi32(std::ptr::read_unaligned(in32_ptr.add(c)));
                let wc = w_base.add(c * stride);
                macro_rules! step_unsigned {
                    ($acc:ident, $off:expr) => {{
                        let wv = _mm256_loadu_si256(wc.add($off) as *const __m256i);
                        let p = _mm256_madd_epi16(_mm256_maddubs_epi16(in0, wv), one);
                        $acc = _mm256_add_epi32($acc, p);
                    }};
                }
                step_unsigned!(a0, 0);
                step_unsigned!(a1, 32);
                step_unsigned!(a2, 64);
                step_unsigned!(a3, 96);
                step_unsigned!(a4, 128);
                step_unsigned!(a5, 160);
                step_unsigned!(a6, 192);
                step_unsigned!(a7, 224);
            }
        }

        _mm256_storeu_si256(o_ptr.add(boff) as *mut __m256i, a0);
        _mm256_storeu_si256(o_ptr.add(boff + 8) as *mut __m256i, a1);
        _mm256_storeu_si256(o_ptr.add(boff + 16) as *mut __m256i, a2);
        _mm256_storeu_si256(o_ptr.add(boff + 24) as *mut __m256i, a3);
        _mm256_storeu_si256(o_ptr.add(boff + 32) as *mut __m256i, a4);
        _mm256_storeu_si256(o_ptr.add(boff + 40) as *mut __m256i, a5);
        _mm256_storeu_si256(o_ptr.add(boff + 48) as *mut __m256i, a6);
        _mm256_storeu_si256(o_ptr.add(boff + 56) as *mut __m256i, a7);
        out_batch += 8;
    }

    // Remainder tile (e.g. 2 batches for OUT=528)
    let rem = num_batches - out_batch;
    if rem > 0 {
        let mut acc = [_mm256_setzero_si256(); 8];
        for t in 0..rem {
            acc[t] = _mm256_loadu_si256(b_ptr.add((out_batch + t) * 8) as *const __m256i);
        }
        let w_base = w_ptr.add(out_batch * 32);
        let stride = OUT * 4;
        for c in 0..num_chunks {
            let in0 = _mm256_set1_epi32(std::ptr::read_unaligned(in32_ptr.add(c)));
            let wc = w_base.add(c * stride);
            if SIGNED_INPUT {
                for t in 0..rem {
                    let w0 = _mm256_loadu_si256(wc.add(t * 32) as *const __m256i);
                    dot4_i8i8_accum_avx2(&mut acc[t], in0, w0, highest_bit, one);
                }
            } else {
                for t in 0..rem {
                    let w0 = _mm256_loadu_si256(wc.add(t * 32) as *const __m256i);
                    dot4_u7i8_accum_avx2(&mut acc[t], in0, w0, one);
                }
            }
        }
        for t in 0..rem {
            _mm256_storeu_si256(o_ptr.add((out_batch + t) * 8) as *mut __m256i, acc[t]);
        }
    }
}

/// Vectorized 16-channel ReLU(i32) -> f32 dot product + bias for Mix9svq policy output.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn policy_dot16_avx2(p_i32: &[i32; 16], w_f32: &[f32; 16], bias: f32) -> f32 {
    let zero = _mm256_setzero_si256();
    let v0 = _mm256_cvtepi32_ps(_mm256_max_epi32(
        _mm256_loadu_si256(p_i32.as_ptr() as *const __m256i),
        zero,
    ));
    let v1 = _mm256_cvtepi32_ps(_mm256_max_epi32(
        _mm256_loadu_si256(p_i32.as_ptr().add(8) as *const __m256i),
        zero,
    ));
    let w0 = _mm256_loadu_ps(w_f32.as_ptr());
    let w1 = _mm256_loadu_ps(w_f32.as_ptr().add(8));
    let prod = _mm256_add_ps(_mm256_mul_ps(v0, w0), _mm256_mul_ps(v1, w1));
    let lo = _mm256_castps256_ps128(prod);
    let hi = _mm256_extractf128_ps(prod, 1);
    let sum4 = _mm_add_ps(lo, hi);
    let shuf = _mm_movehdup_ps(sum4);
    let sum2 = _mm_add_ps(sum4, shuf);
    let shuf2 = _mm_movehl_ps(sum2, sum2);
    let sum1 = _mm_add_ss(sum2, shuf2);
    _mm_cvtss_f32(sum1) + bias
}

/// Vectorized commit of 9 group value-sum deltas and global sum in AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn commit_vsum_avx2(
    prev_groups: &[[[i32; 64]; 3]; 3],
    deltas: &[[[i32; 64]; 3]; 3],
    next_groups: &mut [[[i32; 64]; 3]; 3],
    next_global: &mut [i32; 64],
) {
    for b in 0..8 {
        let off = b * 8;
        let mut g_acc = _mm256_setzero_si256();
        for i in 0..3 {
            for j in 0..3 {
                let old_v =
                    _mm256_loadu_si256(prev_groups[i][j].as_ptr().add(off) as *const __m256i);
                let del_v = _mm256_loadu_si256(deltas[i][j].as_ptr().add(off) as *const __m256i);
                let new_v = _mm256_add_epi32(old_v, del_v);
                _mm256_storeu_si256(
                    next_groups[i][j].as_mut_ptr().add(off) as *mut __m256i,
                    new_v,
                );
                g_acc = _mm256_add_epi32(g_acc, new_v);
            }
        }
        _mm256_storeu_si256(next_global.as_mut_ptr().add(off) as *mut __m256i, g_acc);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn linear_i8_row4_avx2<const IN: usize>(
    output: &mut [i32; 4],
    input: &[i8; IN],
    weight: &[i8],
    bias: &[i32; 4],
) {
    debug_assert_eq!(IN % 32, 0);
    let one = _mm256_set1_epi16(1);
    let mut sum0 = _mm256_setzero_si256();
    let mut sum1 = _mm256_setzero_si256();
    let mut sum2 = _mm256_setzero_si256();
    let mut sum3 = _mm256_setzero_si256();
    let in_ptr = input.as_ptr();
    let w_ptr = weight.as_ptr();
    for j in 0..(IN / 32) {
        let off = j * 32;
        let inv = _mm256_loadu_si256(in_ptr.add(off) as *const __m256i);
        dot4_u7i8_accum_avx2(
            &mut sum0,
            inv,
            _mm256_loadu_si256(w_ptr.add(off) as *const __m256i),
            one,
        );
        dot4_u7i8_accum_avx2(
            &mut sum1,
            inv,
            _mm256_loadu_si256(w_ptr.add(IN + off) as *const __m256i),
            one,
        );
        dot4_u7i8_accum_avx2(
            &mut sum2,
            inv,
            _mm256_loadu_si256(w_ptr.add(2 * IN + off) as *const __m256i),
            one,
        );
        dot4_u7i8_accum_avx2(
            &mut sum3,
            inv,
            _mm256_loadu_si256(w_ptr.add(3 * IN + off) as *const __m256i),
            one,
        );
    }
    // Horizontal sum of 4 __m256i vectors into 4 i32s
    let s01 = _mm256_hadd_epi32(sum0, sum1);
    let s23 = _mm256_hadd_epi32(sum2, sum3);
    let s0123 = _mm256_hadd_epi32(s01, s23);
    let lo = _mm256_castsi256_si128(s0123);
    let hi = _mm256_extracti128_si256(s0123, 1);
    let total = _mm_add_epi32(lo, hi);
    let b = _mm_loadu_si128(bias.as_ptr() as *const __m128i);
    _mm_storeu_si128(output.as_mut_ptr() as *mut __m128i, _mm_add_epi32(total, b));
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn linear_i16_chunk2_prerelu_avx2<const OUT: usize, const IN: usize>(
    output: &mut [i32; OUT],
    input: &[i16],
    weight: &[i16],
    bias: &[i32; OUT],
) {
    debug_assert_eq!(OUT, 16);
    debug_assert_eq!(IN % 2, 0);
    let zero = _mm256_setzero_si256();
    let mut acc0 = _mm256_loadu_si256(bias.as_ptr() as *const __m256i);
    let mut acc1 = _mm256_loadu_si256(bias.as_ptr().add(8) as *const __m256i);
    let in32_ptr = input.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    for c in 0..(IN / 2) {
        let mut in0 = _mm256_set1_epi32(std::ptr::read_unaligned(in32_ptr.add(c)));
        in0 = _mm256_max_epi16(in0, zero);
        let w0 = _mm256_loadu_si256(w_ptr.add(c * 32) as *const __m256i);
        let w1 = _mm256_loadu_si256(w_ptr.add(c * 32 + 16) as *const __m256i);
        acc0 = _mm256_add_epi32(acc0, _mm256_madd_epi16(in0, w0));
        acc1 = _mm256_add_epi32(acc1, _mm256_madd_epi16(in0, w1));
    }
    _mm256_storeu_si256(output.as_mut_ptr() as *mut __m256i, acc0);
    _mm256_storeu_si256(output.as_mut_ptr().add(8) as *mut __m256i, acc1);
}

/// Fused 4-candidate AVX2 pointwise policy conv (`32 -> 16` pre-ReLU i16) +
/// `policy_dot16` output projection. Loads `pw_w` (1 KiB) once per 4 candidates
/// and keeps all 16-channel accumulators in YMM registers (100% bit-exact with
/// `linear_i16_chunk2_prerelu_avx2` + `policy_dot16_avx2`).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
pub unsafe fn policy_eval_batch4_avx2(
    in0: &[i16; 32],
    in1: &[i16; 32],
    in2: &[i16; 32],
    in3: &[i16; 32],
    pw_w: &[i16; 512],
    pw_b: &[i32; 16],
    out_w: &[f32; 16],
    out_bias: f32,
) -> [f32; 4] {
    let zero = _mm256_setzero_si256();
    let b0 = _mm256_loadu_si256(pw_b.as_ptr() as *const __m256i);
    let b1 = _mm256_loadu_si256(pw_b.as_ptr().add(8) as *const __m256i);
    let (mut a00, mut a01) = (b0, b1);
    let (mut a10, mut a11) = (b0, b1);
    let (mut a20, mut a21) = (b0, b1);
    let (mut a30, mut a31) = (b0, b1);

    let p0 = in0.as_ptr() as *const i32;
    let p1 = in1.as_ptr() as *const i32;
    let p2 = in2.as_ptr() as *const i32;
    let p3 = in3.as_ptr() as *const i32;
    let w_ptr = pw_w.as_ptr();

    for c in 0..16 {
        let w0 = _mm256_loadu_si256(w_ptr.add(c * 32) as *const __m256i);
        let w1 = _mm256_loadu_si256(w_ptr.add(c * 32 + 16) as *const __m256i);

        let v0 = _mm256_max_epi16(_mm256_set1_epi32(std::ptr::read_unaligned(p0.add(c))), zero);
        a00 = _mm256_add_epi32(a00, _mm256_madd_epi16(v0, w0));
        a01 = _mm256_add_epi32(a01, _mm256_madd_epi16(v0, w1));

        let v1 = _mm256_max_epi16(_mm256_set1_epi32(std::ptr::read_unaligned(p1.add(c))), zero);
        a10 = _mm256_add_epi32(a10, _mm256_madd_epi16(v1, w0));
        a11 = _mm256_add_epi32(a11, _mm256_madd_epi16(v1, w1));

        let v2 = _mm256_max_epi16(_mm256_set1_epi32(std::ptr::read_unaligned(p2.add(c))), zero);
        a20 = _mm256_add_epi32(a20, _mm256_madd_epi16(v2, w0));
        a21 = _mm256_add_epi32(a21, _mm256_madd_epi16(v2, w1));

        let v3 = _mm256_max_epi16(_mm256_set1_epi32(std::ptr::read_unaligned(p3.add(c))), zero);
        a30 = _mm256_add_epi32(a30, _mm256_madd_epi16(v3, w0));
        a31 = _mm256_add_epi32(a31, _mm256_madd_epi16(v3, w1));
    }

    let fw0 = _mm256_loadu_ps(out_w.as_ptr());
    let fw1 = _mm256_loadu_ps(out_w.as_ptr().add(8));
    #[inline(always)]
    unsafe fn reduce16(
        a0: __m256i,
        a1: __m256i,
        zero: __m256i,
        fw0: __m256,
        fw1: __m256,
        bias: f32,
    ) -> f32 {
        let f0 = _mm256_cvtepi32_ps(_mm256_max_epi32(a0, zero));
        let f1 = _mm256_cvtepi32_ps(_mm256_max_epi32(a1, zero));
        let prod = _mm256_add_ps(_mm256_mul_ps(f0, fw0), _mm256_mul_ps(f1, fw1));
        let lo = _mm256_castps256_ps128(prod);
        let hi = _mm256_extractf128_ps(prod, 1);
        let sum4 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum4);
        let sum2 = _mm_add_ps(sum4, shuf);
        let shuf2 = _mm_movehl_ps(sum2, sum2);
        _mm_cvtss_f32(_mm_add_ss(sum2, shuf2)) + bias
    }
    [
        reduce16(a00, a01, zero, fw0, fw1, out_bias),
        reduce16(a10, a11, zero, fw0, fw1, out_bias),
        reduce16(a20, a21, zero, fw0, fw1, out_bias),
        reduce16(a30, a31, zero, fw0, fw1, out_bias),
    ]
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn avg_pool_2x2_u8_avx2(
    q00: &mut [i8; 64],
    q01: &mut [i8; 64],
    q10: &mut [i8; 64],
    q11: &mut [i8; 64],
    g: &[[[i8; 64]; 3]; 3],
) {
    for b in 0..2 {
        let off = b * 32;
        let v00 = _mm256_loadu_si256(g[0][0].as_ptr().add(off) as *const __m256i);
        let v01 = _mm256_loadu_si256(g[0][1].as_ptr().add(off) as *const __m256i);
        let v02 = _mm256_loadu_si256(g[0][2].as_ptr().add(off) as *const __m256i);
        let v10 = _mm256_loadu_si256(g[1][0].as_ptr().add(off) as *const __m256i);
        let v11 = _mm256_loadu_si256(g[1][1].as_ptr().add(off) as *const __m256i);
        let v12 = _mm256_loadu_si256(g[1][2].as_ptr().add(off) as *const __m256i);
        let v20 = _mm256_loadu_si256(g[2][0].as_ptr().add(off) as *const __m256i);
        let v21 = _mm256_loadu_si256(g[2][1].as_ptr().add(off) as *const __m256i);
        let v22 = _mm256_loadu_si256(g[2][2].as_ptr().add(off) as *const __m256i);

        let r00 = _mm256_avg_epu8(_mm256_avg_epu8(v00, v01), _mm256_avg_epu8(v10, v11));
        let r01 = _mm256_avg_epu8(_mm256_avg_epu8(v01, v02), _mm256_avg_epu8(v11, v12));
        let r10 = _mm256_avg_epu8(_mm256_avg_epu8(v10, v11), _mm256_avg_epu8(v20, v21));
        let r11 = _mm256_avg_epu8(_mm256_avg_epu8(v11, v12), _mm256_avg_epu8(v21, v22));

        _mm256_storeu_si256(q00.as_mut_ptr().add(off) as *mut __m256i, r00);
        _mm256_storeu_si256(q01.as_mut_ptr().add(off) as *mut __m256i, r01);
        _mm256_storeu_si256(q10.as_mut_ptr().add(off) as *mut __m256i, r10);
        _mm256_storeu_si256(q11.as_mut_ptr().add(off) as *mut __m256i, r11);
    }
}

/// Hot incremental `move()` kernel (AVX2): updates `map_sum[new]`, applies 3x3
/// depthwise conv delta to `map_conv`, and accumulates channels `32..64` into
/// `vsum_group[gi][gj]`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
pub unsafe fn update_point_avx2(
    old_codebook: &[i16; 64],
    new_codebook: &[i16; 64],
    old_map_sum: &[i16; 64],
    new_map_sum: &mut [i16; 64],
    dwconv_weight: &[[i16; 32]; 9],
    map_conv: &mut [[i16; 32]],
    outer_indices: &[u16; 9],
    vsum_group: &mut [i32; 64],
) {
    let zero = _mm256_setzero_si256();
    let mut old_f = [zero; 4];
    let mut new_f = [zero; 4];
    for b in 0..4 {
        let off = b * 16;
        let old_cb = _mm256_loadu_si256(old_codebook.as_ptr().add(off) as *const __m256i);
        let new_cb = _mm256_loadu_si256(new_codebook.as_ptr().add(off) as *const __m256i);
        let old_s = _mm256_loadu_si256(old_map_sum.as_ptr().add(off) as *const __m256i);
        let new_s = _mm256_add_epi16(_mm256_sub_epi16(old_s, old_cb), new_cb);
        _mm256_storeu_si256(new_map_sum.as_mut_ptr().add(off) as *mut __m256i, new_s);
        old_f[b] = _mm256_max_epi16(old_s, zero);
        new_f[b] = _mm256_max_epi16(new_s, zero);
    }

    // Channels 0..32: depthwise 3x3 conv delta
    let old_c0 = _mm256_slli_epi16(old_f[0], 2);
    let old_c1 = _mm256_slli_epi16(old_f[1], 2);
    let new_c0 = _mm256_slli_epi16(new_f[0], 2);
    let new_c1 = _mm256_slli_epi16(new_f[1], 2);

    for k in 0..9 {
        let w_row = &dwconv_weight[8 - k];
        let w0 = _mm256_loadu_si256(w_row.as_ptr() as *const __m256i);
        let w1 = _mm256_loadu_si256(w_row.as_ptr().add(16) as *const __m256i);
        let d0 = _mm256_sub_epi16(
            _mm256_mulhi_epi16(w0, new_c0),
            _mm256_mulhi_epi16(w0, old_c0),
        );
        let d1 = _mm256_sub_epi16(
            _mm256_mulhi_epi16(w1, new_c1),
            _mm256_mulhi_epi16(w1, old_c1),
        );
        let dst = map_conv
            .get_unchecked_mut(outer_indices[k] as usize)
            .as_mut_ptr();
        let cur0 = _mm256_loadu_si256(dst as *const __m256i);
        let cur1 = _mm256_loadu_si256(dst.add(16) as *const __m256i);
        _mm256_storeu_si256(dst as *mut __m256i, _mm256_add_epi16(cur0, d0));
        _mm256_storeu_si256(dst.add(16) as *mut __m256i, _mm256_add_epi16(cur1, d1));
    }

    // Channels 32..64: direct delta into vsum_group[32..64]
    for b in 2..4 {
        let delta = _mm256_sub_epi16(new_f[b], old_f[b]);
        let lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(delta));
        let hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(delta, 1));
        let ptr = vsum_group.as_mut_ptr().add(b * 16);
        let cur0 = _mm256_loadu_si256(ptr as *const __m256i);
        let cur1 = _mm256_loadu_si256(ptr.add(8) as *const __m256i);
        _mm256_storeu_si256(ptr as *mut __m256i, _mm256_add_epi32(cur0, lo));
        _mm256_storeu_si256(ptr.add(8) as *mut __m256i, _mm256_add_epi32(cur1, hi));
    }
}

/// Hot incremental `dwconv` commit kernel (AVX2): adds delta into `new_conv`,
/// applies ReLU to old & new, and accumulates channels `0..32` into `vsum_group`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn commit_conv_cell_avx2(
    old_conv: &[i16; 32],
    new_conv: &mut [i16; 32],
    vsum_group: &mut [i32; 64],
) {
    let zero = _mm256_setzero_si256();
    for b in 0..2 {
        let off = b * 16;
        let old_v = _mm256_loadu_si256(old_conv.as_ptr().add(off) as *const __m256i);
        let delta_v = _mm256_loadu_si256(new_conv.as_ptr().add(off) as *const __m256i);
        let new_v = _mm256_add_epi16(old_v, delta_v);
        _mm256_storeu_si256(new_conv.as_mut_ptr().add(off) as *mut __m256i, new_v);
        let old_r = _mm256_max_epi16(old_v, zero);
        let new_r = _mm256_max_epi16(new_v, zero);
        let diff = _mm256_sub_epi16(new_r, old_r);
        let lo = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(diff));
        let hi = _mm256_cvtepi16_epi32(_mm256_extracti128_si256(diff, 1));
        let ptr = vsum_group.as_mut_ptr().add(off);
        let cur0 = _mm256_loadu_si256(ptr as *const __m256i);
        let cur1 = _mm256_loadu_si256(ptr.add(8) as *const __m256i);
        _mm256_storeu_si256(ptr as *mut __m256i, _mm256_add_epi32(cur0, lo));
        _mm256_storeu_si256(ptr.add(8) as *mut __m256i, _mm256_add_epi32(cur1, hi));
    }
}

// ============================================================================
// AVX-VNNI SIMD Kernels (x86_64, `_mm256_dpbusd_avx_epi32`)
// ============================================================================

/// Compute the bias offset `-128 * sum_j weight[o][j]` for a row-major `[OUT][IN]`
/// i8 matrix so signed-i8 inputs can be shifted by `+128` (`x ^ 0x80`) into `u8`
/// and evaluated with a single `_mm256_dpbusd_avx_epi32` per output tile.
pub fn compute_signed_input_bias<const OUT: usize, const IN: usize>(
    row_major_weight: &[i8],
    bias: &[i32],
) -> Vec<i32> {
    assert_eq!(row_major_weight.len(), OUT * IN);
    assert_eq!(bias.len(), OUT);
    let mut out = Vec::with_capacity(OUT);
    for o in 0..OUT {
        let mut col_sum = 0_i32;
        let row = &row_major_weight[o * IN..(o + 1) * IN];
        for &w in row {
            col_sum += i32::from(w);
        }
        out.push(bias[o].wrapping_sub(col_sum.wrapping_mul(128)));
    }
    out
}

/// AVX-VNNI chunk-4 i8 affine layer (`_mm256_dpbusd_avx_epi32`).
///
/// When `SIGNED_INPUT == true`, `bias` MUST be the pre-adjusted `signed_bias`
/// produced by [`compute_signed_input_bias`], and each input byte is shifted by
/// `+128` (`^ 0x80`) into unsigned `u8` before `dpbusd`, which is bit-identical
/// to exact `i8 * i8 -> i32` accumulation with half the instructions.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,avxvnni")]
pub unsafe fn linear_i8_chunk4_avxvnni<
    const OUT: usize,
    const IN: usize,
    const SIGNED_INPUT: bool,
>(
    output: &mut [i32; OUT],
    input: &[i8],
    weight: &[i8],
    bias: &[i32],
) {
    debug_assert_eq!(OUT % 8, 0);
    debug_assert_eq!(IN % 4, 0);
    let num_batches = OUT / 8;
    let num_chunks = IN / 4;
    let sign_flip = _mm256_set1_epi8(-128_i8);
    let in32_ptr = input.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    let b_ptr = bias.as_ptr();
    let o_ptr = output.as_mut_ptr();

    let mut out_batch = 0;
    while out_batch + 8 <= num_batches {
        let boff = out_batch * 8;
        let mut a0 = _mm256_loadu_si256(b_ptr.add(boff) as *const __m256i);
        let mut a1 = _mm256_loadu_si256(b_ptr.add(boff + 8) as *const __m256i);
        let mut a2 = _mm256_loadu_si256(b_ptr.add(boff + 16) as *const __m256i);
        let mut a3 = _mm256_loadu_si256(b_ptr.add(boff + 24) as *const __m256i);
        let mut a4 = _mm256_loadu_si256(b_ptr.add(boff + 32) as *const __m256i);
        let mut a5 = _mm256_loadu_si256(b_ptr.add(boff + 40) as *const __m256i);
        let mut a6 = _mm256_loadu_si256(b_ptr.add(boff + 48) as *const __m256i);
        let mut a7 = _mm256_loadu_si256(b_ptr.add(boff + 56) as *const __m256i);

        let w_base = w_ptr.add(out_batch * 32);
        let stride = OUT * 4;
        for c in 0..num_chunks {
            let mut in0 = _mm256_set1_epi32(std::ptr::read_unaligned(in32_ptr.add(c)));
            if SIGNED_INPUT {
                in0 = _mm256_xor_si256(in0, sign_flip);
            }
            let wc = w_base.add(c * stride);
            a0 = _mm256_dpbusd_avx_epi32(a0, in0, _mm256_loadu_si256(wc as *const __m256i));
            a1 = _mm256_dpbusd_avx_epi32(a1, in0, _mm256_loadu_si256(wc.add(32) as *const __m256i));
            a2 = _mm256_dpbusd_avx_epi32(a2, in0, _mm256_loadu_si256(wc.add(64) as *const __m256i));
            a3 = _mm256_dpbusd_avx_epi32(a3, in0, _mm256_loadu_si256(wc.add(96) as *const __m256i));
            a4 =
                _mm256_dpbusd_avx_epi32(a4, in0, _mm256_loadu_si256(wc.add(128) as *const __m256i));
            a5 =
                _mm256_dpbusd_avx_epi32(a5, in0, _mm256_loadu_si256(wc.add(160) as *const __m256i));
            a6 =
                _mm256_dpbusd_avx_epi32(a6, in0, _mm256_loadu_si256(wc.add(192) as *const __m256i));
            a7 =
                _mm256_dpbusd_avx_epi32(a7, in0, _mm256_loadu_si256(wc.add(224) as *const __m256i));
        }

        _mm256_storeu_si256(o_ptr.add(boff) as *mut __m256i, a0);
        _mm256_storeu_si256(o_ptr.add(boff + 8) as *mut __m256i, a1);
        _mm256_storeu_si256(o_ptr.add(boff + 16) as *mut __m256i, a2);
        _mm256_storeu_si256(o_ptr.add(boff + 24) as *mut __m256i, a3);
        _mm256_storeu_si256(o_ptr.add(boff + 32) as *mut __m256i, a4);
        _mm256_storeu_si256(o_ptr.add(boff + 40) as *mut __m256i, a5);
        _mm256_storeu_si256(o_ptr.add(boff + 48) as *mut __m256i, a6);
        _mm256_storeu_si256(o_ptr.add(boff + 56) as *mut __m256i, a7);
        out_batch += 8;
    }

    let rem = num_batches - out_batch;
    if rem > 0 {
        let mut acc = [_mm256_setzero_si256(); 8];
        for t in 0..rem {
            acc[t] = _mm256_loadu_si256(b_ptr.add((out_batch + t) * 8) as *const __m256i);
        }
        let w_base = w_ptr.add(out_batch * 32);
        let stride = OUT * 4;
        for c in 0..num_chunks {
            let mut in0 = _mm256_set1_epi32(std::ptr::read_unaligned(in32_ptr.add(c)));
            if SIGNED_INPUT {
                in0 = _mm256_xor_si256(in0, sign_flip);
            }
            let wc = w_base.add(c * stride);
            for t in 0..rem {
                let w0 = _mm256_loadu_si256(wc.add(t * 32) as *const __m256i);
                acc[t] = _mm256_dpbusd_avx_epi32(acc[t], in0, w0);
            }
        }
        for t in 0..rem {
            _mm256_storeu_si256(o_ptr.add((out_batch + t) * 8) as *mut __m256i, acc[t]);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,avxvnni")]
pub unsafe fn linear_i8_row4_avxvnni<const IN: usize>(
    output: &mut [i32; 4],
    input: &[i8; IN],
    weight: &[i8],
    bias: &[i32; 4],
) {
    debug_assert_eq!(IN % 32, 0);
    let mut sum0 = _mm256_setzero_si256();
    let mut sum1 = _mm256_setzero_si256();
    let mut sum2 = _mm256_setzero_si256();
    let mut sum3 = _mm256_setzero_si256();
    let in_ptr = input.as_ptr();
    let w_ptr = weight.as_ptr();
    for j in 0..(IN / 32) {
        let off = j * 32;
        let inv = _mm256_loadu_si256(in_ptr.add(off) as *const __m256i);
        sum0 = _mm256_dpbusd_avx_epi32(
            sum0,
            inv,
            _mm256_loadu_si256(w_ptr.add(off) as *const __m256i),
        );
        sum1 = _mm256_dpbusd_avx_epi32(
            sum1,
            inv,
            _mm256_loadu_si256(w_ptr.add(IN + off) as *const __m256i),
        );
        sum2 = _mm256_dpbusd_avx_epi32(
            sum2,
            inv,
            _mm256_loadu_si256(w_ptr.add(2 * IN + off) as *const __m256i),
        );
        sum3 = _mm256_dpbusd_avx_epi32(
            sum3,
            inv,
            _mm256_loadu_si256(w_ptr.add(3 * IN + off) as *const __m256i),
        );
    }
    let s01 = _mm256_hadd_epi32(sum0, sum1);
    let s23 = _mm256_hadd_epi32(sum2, sum3);
    let s0123 = _mm256_hadd_epi32(s01, s23);
    let lo = _mm256_castsi256_si128(s0123);
    let hi = _mm256_extracti128_si256(s0123, 1);
    let total = _mm_add_epi32(lo, hi);
    let b = _mm_loadu_si128(bias.as_ptr() as *const __m128i);
    _mm_storeu_si128(output.as_mut_ptr() as *mut __m128i, _mm_add_epi32(total, b));
}

/// Batched 4-input AVX-VNNI chunk-4 i8 affine layer sharing a single weight matrix.
/// Streams each 32-byte weight vector once for all 4 inputs (4x lower L1/L2 traffic
/// and 4x higher compute-to-load ratio than 4 separate calls).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,avxvnni")]
#[allow(clippy::too_many_arguments)]
pub unsafe fn linear_i8_chunk4_batch4_avxvnni<
    const OUT: usize,
    const IN: usize,
    const SIGNED_INPUT: bool,
>(
    out0: &mut [i32; OUT],
    out1: &mut [i32; OUT],
    out2: &mut [i32; OUT],
    out3: &mut [i32; OUT],
    in0: &[i8; IN],
    in1: &[i8; IN],
    in2: &[i8; IN],
    in3: &[i8; IN],
    weight: &[i8],
    bias: &[i32],
) {
    debug_assert_eq!(OUT % 16, 0);
    debug_assert_eq!(IN % 4, 0);
    let num_batches = OUT / 8;
    let num_chunks = IN / 4;
    let sign_flip = _mm256_set1_epi8(-128_i8);
    let p0 = in0.as_ptr() as *const i32;
    let p1 = in1.as_ptr() as *const i32;
    let p2 = in2.as_ptr() as *const i32;
    let p3 = in3.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    let b_ptr = bias.as_ptr();
    let o0_ptr = out0.as_mut_ptr();
    let o1_ptr = out1.as_mut_ptr();
    let o2_ptr = out2.as_mut_ptr();
    let o3_ptr = out3.as_mut_ptr();
    let stride = OUT * 4;

    let mut out_batch = 0;
    while out_batch < num_batches {
        let boff = out_batch * 8;
        let b0 = _mm256_loadu_si256(b_ptr.add(boff) as *const __m256i);
        let b1 = _mm256_loadu_si256(b_ptr.add(boff + 8) as *const __m256i);
        let (mut a00, mut a01) = (b0, b1);
        let (mut a10, mut a11) = (b0, b1);
        let (mut a20, mut a21) = (b0, b1);
        let (mut a30, mut a31) = (b0, b1);

        let w_base = w_ptr.add(out_batch * 32);
        for c in 0..num_chunks {
            let wc = w_base.add(c * stride);
            let w0 = _mm256_loadu_si256(wc as *const __m256i);
            let w1 = _mm256_loadu_si256(wc.add(32) as *const __m256i);

            let mut v0 = _mm256_set1_epi32(std::ptr::read_unaligned(p0.add(c)));
            let mut v1 = _mm256_set1_epi32(std::ptr::read_unaligned(p1.add(c)));
            let mut v2 = _mm256_set1_epi32(std::ptr::read_unaligned(p2.add(c)));
            let mut v3 = _mm256_set1_epi32(std::ptr::read_unaligned(p3.add(c)));
            if SIGNED_INPUT {
                v0 = _mm256_xor_si256(v0, sign_flip);
                v1 = _mm256_xor_si256(v1, sign_flip);
                v2 = _mm256_xor_si256(v2, sign_flip);
                v3 = _mm256_xor_si256(v3, sign_flip);
            }

            a00 = _mm256_dpbusd_avx_epi32(a00, v0, w0);
            a01 = _mm256_dpbusd_avx_epi32(a01, v0, w1);
            a10 = _mm256_dpbusd_avx_epi32(a10, v1, w0);
            a11 = _mm256_dpbusd_avx_epi32(a11, v1, w1);
            a20 = _mm256_dpbusd_avx_epi32(a20, v2, w0);
            a21 = _mm256_dpbusd_avx_epi32(a21, v2, w1);
            a30 = _mm256_dpbusd_avx_epi32(a30, v3, w0);
            a31 = _mm256_dpbusd_avx_epi32(a31, v3, w1);
        }

        _mm256_storeu_si256(o0_ptr.add(boff) as *mut __m256i, a00);
        _mm256_storeu_si256(o0_ptr.add(boff + 8) as *mut __m256i, a01);
        _mm256_storeu_si256(o1_ptr.add(boff) as *mut __m256i, a10);
        _mm256_storeu_si256(o1_ptr.add(boff + 8) as *mut __m256i, a11);
        _mm256_storeu_si256(o2_ptr.add(boff) as *mut __m256i, a20);
        _mm256_storeu_si256(o2_ptr.add(boff + 8) as *mut __m256i, a21);
        _mm256_storeu_si256(o3_ptr.add(boff) as *mut __m256i, a30);
        _mm256_storeu_si256(o3_ptr.add(boff + 8) as *mut __m256i, a31);
        out_batch += 2;
    }
}

// ============================================================================
// ARM64 NEON & NEON-DOTPROD SIMD Kernels (aarch64)
// ============================================================================

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn mulhi_i16_neon(a: int16x8_t, b: int16x8_t) -> int16x8_t {
    let ab_lo = vmull_s16(vget_low_s16(a), vget_low_s16(b));
    let ab_hi = vmull_high_s16(a, b);
    vuzp2q_s16(vreinterpretq_s16_s32(ab_lo), vreinterpretq_s16_s32(ab_hi))
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn dot2_u7i8_pair_neon(a: int8x16_t, b: int8x16_t) -> int16x8_t {
    let a_u8 = vreinterpretq_u8_s8(a);
    let tl = vmulq_s16(
        vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(a_u8))),
        vmovl_s8(vget_low_s8(b)),
    );
    let th = vmulq_s16(
        vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(a_u8))),
        vmovl_s8(vget_high_s8(b)),
    );
    vqaddq_s16(vuzp1q_s16(tl, th), vuzp2q_s16(tl, th))
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn dot4_u7i8_accum_neon(acc: &mut int32x4_t, a: int8x16_t, b: int8x16_t) {
    let p0 = vmull_s8(vget_low_s8(a), vget_low_s8(b));
    let p1 = vmull_high_s8(a, b);
    let sum = vpaddq_s16(p0, p1);
    *acc = vpadalq_s16(*acc, sum);
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn dot4_i8i8_accum_neon(acc: &mut int32x4_t, a: int8x16_t, b: int8x16_t) {
    // Widen before adding: two (-128) * (-128) products sum to +32768, overflowing i16.
    let p0 = vmull_s8(vget_low_s8(a), vget_low_s8(b));
    let p1 = vmull_high_s8(a, b);
    let sum01 = vpaddq_s32(vpaddlq_s16(p0), vpaddlq_s16(p1));
    *acc = vaddq_s32(*acc, sum01);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn crelu_i32_to_i8_neon<const SIZE: usize, const SHIFT: i32, const NO_RELU: bool>(
    output: &mut [i8; SIZE],
    input: &[i32; SIZE],
) {
    debug_assert_eq!(SIZE % 16, 0);
    let zero = vdupq_n_s8(0);
    let batches = SIZE / 16;
    let in_ptr = input.as_ptr();
    let out_ptr = output.as_mut_ptr();
    for i in 0..batches {
        let base = i * 16;
        let in0 = vld1q_s32(in_ptr.add(base));
        let in1 = vld1q_s32(in_ptr.add(base + 4));
        let in2 = vld1q_s32(in_ptr.add(base + 8));
        let in3 = vld1q_s32(in_ptr.add(base + 12));
        let mut in01 = vqmovn_high_s32(vqmovn_s32(in0), in1);
        let mut in23 = vqmovn_high_s32(vqmovn_s32(in2), in3);
        if SHIFT > 0 {
            in01 = vshrq_n_s16::<SHIFT>(in01);
            in23 = vshrq_n_s16::<SHIFT>(in23);
        }
        let mut res = vqmovn_high_s16(vqmovn_s16(in01), in23);
        if !NO_RELU {
            res = vmaxq_s8(res, zero);
        }
        vst1q_s8(out_ptr.add(base), res);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn crelu_i32_to_i16_neon<const SIZE: usize, const SHIFT: i32, const NO_RELU: bool>(
    output: &mut [i16; SIZE],
    input: &[i32; SIZE],
) {
    debug_assert_eq!(SIZE % 8, 0);
    let zero = vdupq_n_s16(0);
    let batches = SIZE / 8;
    let in_ptr = input.as_ptr();
    let out_ptr = output.as_mut_ptr();
    for i in 0..batches {
        let base = i * 8;
        let mut in0 = vld1q_s32(in_ptr.add(base));
        let mut in1 = vld1q_s32(in_ptr.add(base + 4));
        if SHIFT > 0 {
            let neg_shift = vdupq_n_s32(-SHIFT);
            in0 = vshlq_s32(in0, neg_shift);
            in1 = vshlq_s32(in1, neg_shift);
        }
        let mut res = vqmovn_high_s32(vqmovn_s32(in0), in1);
        if !NO_RELU {
            res = vmaxq_s16(res, zero);
        }
        vst1q_s16(out_ptr.add(base), res);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn dot2_u7i8_neon<const OUT: usize, const SHIFT: i32>(
    output: &mut [i8; OUT],
    input_u7: &[i8],
    input_i8: &[i8],
) {
    debug_assert_eq!(OUT % 16, 0);
    let batches = OUT / 16;
    let u7_ptr = input_u7.as_ptr();
    let i8_ptr = input_i8.as_ptr();
    let out_ptr = output.as_mut_ptr();
    for i in 0..batches {
        let in10 = vld1q_s8(u7_ptr.add(i * 32));
        let in11 = vld1q_s8(u7_ptr.add(i * 32 + 16));
        let in20 = vld1q_s8(i8_ptr.add(i * 32));
        let in21 = vld1q_s8(i8_ptr.add(i * 32 + 16));
        let d0 = vshrq_n_s16::<SHIFT>(dot2_u7i8_pair_neon(in10, in20));
        let d1 = vshrq_n_s16::<SHIFT>(dot2_u7i8_pair_neon(in11, in21));
        let packed = vqmovn_high_s16(vqmovn_s16(d0), d1);
        vst1q_s8(out_ptr.add(i * 16), packed);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn linear_i8_chunk4_neon<const OUT: usize, const IN: usize, const SIGNED_INPUT: bool>(
    output: &mut [i32; OUT],
    input: &[i8],
    weight: &[i8],
    bias: &[i32],
) {
    debug_assert_eq!(OUT % 4, 0);
    debug_assert_eq!(IN % 4, 0);
    let num_batches = OUT / 4;
    let num_chunks = IN / 4;
    let in32_ptr = input.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    let b_ptr = bias.as_ptr();
    let o_ptr = output.as_mut_ptr();
    let stride = OUT * 4;

    let mut out_batch = 0;
    // 16-register accumulator tile (64 outputs per tile) leverages AArch64's 32 NEON
    // vector registers with zero stack spills.
    while out_batch + 16 <= num_batches {
        let boff = out_batch * 4;
        let mut a0 = vld1q_s32(b_ptr.add(boff));
        let mut a1 = vld1q_s32(b_ptr.add(boff + 4));
        let mut a2 = vld1q_s32(b_ptr.add(boff + 8));
        let mut a3 = vld1q_s32(b_ptr.add(boff + 12));
        let mut a4 = vld1q_s32(b_ptr.add(boff + 16));
        let mut a5 = vld1q_s32(b_ptr.add(boff + 20));
        let mut a6 = vld1q_s32(b_ptr.add(boff + 24));
        let mut a7 = vld1q_s32(b_ptr.add(boff + 28));
        let mut a8 = vld1q_s32(b_ptr.add(boff + 32));
        let mut a9 = vld1q_s32(b_ptr.add(boff + 36));
        let mut a10 = vld1q_s32(b_ptr.add(boff + 40));
        let mut a11 = vld1q_s32(b_ptr.add(boff + 44));
        let mut a12 = vld1q_s32(b_ptr.add(boff + 48));
        let mut a13 = vld1q_s32(b_ptr.add(boff + 52));
        let mut a14 = vld1q_s32(b_ptr.add(boff + 56));
        let mut a15 = vld1q_s32(b_ptr.add(boff + 60));

        let w_base = w_ptr.add(out_batch * 16);
        for c in 0..num_chunks {
            let in0 = vreinterpretq_s8_s32(vdupq_n_s32(std::ptr::read_unaligned(in32_ptr.add(c))));
            let wc = w_base.add(c * stride);
            macro_rules! step {
                ($acc:ident, $off:expr) => {{
                    let wv = vld1q_s8(wc.add($off));
                    if SIGNED_INPUT {
                        dot4_i8i8_accum_neon(&mut $acc, in0, wv);
                    } else {
                        dot4_u7i8_accum_neon(&mut $acc, in0, wv);
                    }
                }};
            }
            step!(a0, 0);
            step!(a1, 16);
            step!(a2, 32);
            step!(a3, 48);
            step!(a4, 64);
            step!(a5, 80);
            step!(a6, 96);
            step!(a7, 112);
            step!(a8, 128);
            step!(a9, 144);
            step!(a10, 160);
            step!(a11, 176);
            step!(a12, 192);
            step!(a13, 208);
            step!(a14, 224);
            step!(a15, 240);
        }

        vst1q_s32(o_ptr.add(boff), a0);
        vst1q_s32(o_ptr.add(boff + 4), a1);
        vst1q_s32(o_ptr.add(boff + 8), a2);
        vst1q_s32(o_ptr.add(boff + 12), a3);
        vst1q_s32(o_ptr.add(boff + 16), a4);
        vst1q_s32(o_ptr.add(boff + 20), a5);
        vst1q_s32(o_ptr.add(boff + 24), a6);
        vst1q_s32(o_ptr.add(boff + 28), a7);
        vst1q_s32(o_ptr.add(boff + 32), a8);
        vst1q_s32(o_ptr.add(boff + 36), a9);
        vst1q_s32(o_ptr.add(boff + 40), a10);
        vst1q_s32(o_ptr.add(boff + 44), a11);
        vst1q_s32(o_ptr.add(boff + 48), a12);
        vst1q_s32(o_ptr.add(boff + 52), a13);
        vst1q_s32(o_ptr.add(boff + 56), a14);
        vst1q_s32(o_ptr.add(boff + 60), a15);
        out_batch += 16;
    }

    let rem = num_batches - out_batch;
    if rem > 0 {
        let mut acc = [vdupq_n_s32(0); 16];
        for t in 0..rem {
            acc[t] = vld1q_s32(b_ptr.add((out_batch + t) * 4));
        }
        let w_base = w_ptr.add(out_batch * 16);
        for c in 0..num_chunks {
            let in0 = vreinterpretq_s8_s32(vdupq_n_s32(std::ptr::read_unaligned(in32_ptr.add(c))));
            let wc = w_base.add(c * stride);
            for t in 0..rem {
                let wv = vld1q_s8(wc.add(t * 16));
                if SIGNED_INPUT {
                    dot4_i8i8_accum_neon(&mut acc[t], in0, wv);
                } else {
                    dot4_u7i8_accum_neon(&mut acc[t], in0, wv);
                }
            }
        }
        for t in 0..rem {
            vst1q_s32(o_ptr.add((out_batch + t) * 4), acc[t]);
        }
    }
}

/// ARMv8.2+ `sdot` (`vdotq_s32`) helper using stable inline assembly because
/// `std::arch::aarch64::vdotq_s32` is still gated behind `stdarch_neon_dotprod`.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon,dotprod")]
#[inline]
unsafe fn sdot_s32_neon(mut acc: int32x4_t, a: int8x16_t, b: int8x16_t) -> int32x4_t {
    core::arch::asm!(
        "sdot {acc:v}.4s, {a:v}.16b, {b:v}.16b",
        acc = inout(vreg) acc,
        a = in(vreg) a,
        b = in(vreg) b,
        options(pure, nomem, nostack),
    );
    acc
}

/// ARMv8.2+ `dotprod` (`sdot`) chunk-4 i8 affine layer.
/// Handles both `u7 * i8` (`0..=127`) and signed `i8 * i8` (`-128..=127`) identically
/// in a single `sdot` instruction with zero bias adjustment.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon,dotprod")]
pub unsafe fn linear_i8_chunk4_neon_dotprod<const OUT: usize, const IN: usize>(
    output: &mut [i32; OUT],
    input: &[i8],
    weight: &[i8],
    bias: &[i32],
) {
    debug_assert_eq!(OUT % 4, 0);
    debug_assert_eq!(IN % 4, 0);
    let num_batches = OUT / 4;
    let num_chunks = IN / 4;
    let in32_ptr = input.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    let b_ptr = bias.as_ptr();
    let o_ptr = output.as_mut_ptr();
    let stride = OUT * 4;

    let mut out_batch = 0;
    while out_batch + 16 <= num_batches {
        let boff = out_batch * 4;
        let mut a0 = vld1q_s32(b_ptr.add(boff));
        let mut a1 = vld1q_s32(b_ptr.add(boff + 4));
        let mut a2 = vld1q_s32(b_ptr.add(boff + 8));
        let mut a3 = vld1q_s32(b_ptr.add(boff + 12));
        let mut a4 = vld1q_s32(b_ptr.add(boff + 16));
        let mut a5 = vld1q_s32(b_ptr.add(boff + 20));
        let mut a6 = vld1q_s32(b_ptr.add(boff + 24));
        let mut a7 = vld1q_s32(b_ptr.add(boff + 28));
        let mut a8 = vld1q_s32(b_ptr.add(boff + 32));
        let mut a9 = vld1q_s32(b_ptr.add(boff + 36));
        let mut a10 = vld1q_s32(b_ptr.add(boff + 40));
        let mut a11 = vld1q_s32(b_ptr.add(boff + 44));
        let mut a12 = vld1q_s32(b_ptr.add(boff + 48));
        let mut a13 = vld1q_s32(b_ptr.add(boff + 52));
        let mut a14 = vld1q_s32(b_ptr.add(boff + 56));
        let mut a15 = vld1q_s32(b_ptr.add(boff + 60));

        let w_base = w_ptr.add(out_batch * 16);
        for c in 0..num_chunks {
            let in0 = vreinterpretq_s8_s32(vdupq_n_s32(std::ptr::read_unaligned(in32_ptr.add(c))));
            let wc = w_base.add(c * stride);
            a0 = sdot_s32_neon(a0, in0, vld1q_s8(wc));
            a1 = sdot_s32_neon(a1, in0, vld1q_s8(wc.add(16)));
            a2 = sdot_s32_neon(a2, in0, vld1q_s8(wc.add(32)));
            a3 = sdot_s32_neon(a3, in0, vld1q_s8(wc.add(48)));
            a4 = sdot_s32_neon(a4, in0, vld1q_s8(wc.add(64)));
            a5 = sdot_s32_neon(a5, in0, vld1q_s8(wc.add(80)));
            a6 = sdot_s32_neon(a6, in0, vld1q_s8(wc.add(96)));
            a7 = sdot_s32_neon(a7, in0, vld1q_s8(wc.add(112)));
            a8 = sdot_s32_neon(a8, in0, vld1q_s8(wc.add(128)));
            a9 = sdot_s32_neon(a9, in0, vld1q_s8(wc.add(144)));
            a10 = sdot_s32_neon(a10, in0, vld1q_s8(wc.add(160)));
            a11 = sdot_s32_neon(a11, in0, vld1q_s8(wc.add(176)));
            a12 = sdot_s32_neon(a12, in0, vld1q_s8(wc.add(192)));
            a13 = sdot_s32_neon(a13, in0, vld1q_s8(wc.add(208)));
            a14 = sdot_s32_neon(a14, in0, vld1q_s8(wc.add(224)));
            a15 = sdot_s32_neon(a15, in0, vld1q_s8(wc.add(240)));
        }

        vst1q_s32(o_ptr.add(boff), a0);
        vst1q_s32(o_ptr.add(boff + 4), a1);
        vst1q_s32(o_ptr.add(boff + 8), a2);
        vst1q_s32(o_ptr.add(boff + 12), a3);
        vst1q_s32(o_ptr.add(boff + 16), a4);
        vst1q_s32(o_ptr.add(boff + 20), a5);
        vst1q_s32(o_ptr.add(boff + 24), a6);
        vst1q_s32(o_ptr.add(boff + 28), a7);
        vst1q_s32(o_ptr.add(boff + 32), a8);
        vst1q_s32(o_ptr.add(boff + 36), a9);
        vst1q_s32(o_ptr.add(boff + 40), a10);
        vst1q_s32(o_ptr.add(boff + 44), a11);
        vst1q_s32(o_ptr.add(boff + 48), a12);
        vst1q_s32(o_ptr.add(boff + 52), a13);
        vst1q_s32(o_ptr.add(boff + 56), a14);
        vst1q_s32(o_ptr.add(boff + 60), a15);
        out_batch += 16;
    }

    let rem = num_batches - out_batch;
    if rem > 0 {
        let mut acc = [vdupq_n_s32(0); 16];
        for t in 0..rem {
            acc[t] = vld1q_s32(b_ptr.add((out_batch + t) * 4));
        }
        let w_base = w_ptr.add(out_batch * 16);
        for c in 0..num_chunks {
            let in0 = vreinterpretq_s8_s32(vdupq_n_s32(std::ptr::read_unaligned(in32_ptr.add(c))));
            let wc = w_base.add(c * stride);
            for t in 0..rem {
                acc[t] = sdot_s32_neon(acc[t], in0, vld1q_s8(wc.add(t * 16)));
            }
        }
        for t in 0..rem {
            vst1q_s32(o_ptr.add((out_batch + t) * 4), acc[t]);
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn linear_i8_row4_neon<const IN: usize>(
    output: &mut [i32; 4],
    input: &[i8; IN],
    weight: &[i8],
    bias: &[i32; 4],
) {
    debug_assert_eq!(IN % 16, 0);
    let mut sum0 = vdupq_n_s32(0);
    let mut sum1 = vdupq_n_s32(0);
    let mut sum2 = vdupq_n_s32(0);
    let mut sum3 = vdupq_n_s32(0);
    let in_ptr = input.as_ptr();
    let w_ptr = weight.as_ptr();
    for j in 0..(IN / 16) {
        let off = j * 16;
        let inv = vld1q_s8(in_ptr.add(off));
        dot4_u7i8_accum_neon(&mut sum0, inv, vld1q_s8(w_ptr.add(off)));
        dot4_u7i8_accum_neon(&mut sum1, inv, vld1q_s8(w_ptr.add(IN + off)));
        dot4_u7i8_accum_neon(&mut sum2, inv, vld1q_s8(w_ptr.add(2 * IN + off)));
        dot4_u7i8_accum_neon(&mut sum3, inv, vld1q_s8(w_ptr.add(3 * IN + off)));
    }
    let s01 = vpaddq_s32(sum0, sum1);
    let s23 = vpaddq_s32(sum2, sum3);
    let total = vpaddq_s32(s01, s23);
    let b = vld1q_s32(bias.as_ptr());
    vst1q_s32(output.as_mut_ptr(), vaddq_s32(total, b));
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon,dotprod")]
pub unsafe fn linear_i8_row4_neon_dotprod<const IN: usize>(
    output: &mut [i32; 4],
    input: &[i8; IN],
    weight: &[i8],
    bias: &[i32; 4],
) {
    debug_assert_eq!(IN % 16, 0);
    let mut sum0 = vdupq_n_s32(0);
    let mut sum1 = vdupq_n_s32(0);
    let mut sum2 = vdupq_n_s32(0);
    let mut sum3 = vdupq_n_s32(0);
    let in_ptr = input.as_ptr();
    let w_ptr = weight.as_ptr();
    for j in 0..(IN / 16) {
        let off = j * 16;
        let inv = vld1q_s8(in_ptr.add(off));
        sum0 = sdot_s32_neon(sum0, inv, vld1q_s8(w_ptr.add(off)));
        sum1 = sdot_s32_neon(sum1, inv, vld1q_s8(w_ptr.add(IN + off)));
        sum2 = sdot_s32_neon(sum2, inv, vld1q_s8(w_ptr.add(2 * IN + off)));
        sum3 = sdot_s32_neon(sum3, inv, vld1q_s8(w_ptr.add(3 * IN + off)));
    }
    let s01 = vpaddq_s32(sum0, sum1);
    let s23 = vpaddq_s32(sum2, sum3);
    let total = vpaddq_s32(s01, s23);
    let b = vld1q_s32(bias.as_ptr());
    vst1q_s32(output.as_mut_ptr(), vaddq_s32(total, b));
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn linear_i16_chunk2_prerelu_neon<const OUT: usize, const IN: usize>(
    output: &mut [i32; OUT],
    input: &[i16],
    weight: &[i16],
    bias: &[i32; OUT],
) {
    debug_assert_eq!(OUT, 16);
    debug_assert_eq!(IN % 2, 0);
    let zero = vdupq_n_s16(0);
    let mut acc0 = vld1q_s32(bias.as_ptr());
    let mut acc1 = vld1q_s32(bias.as_ptr().add(4));
    let mut acc2 = vld1q_s32(bias.as_ptr().add(8));
    let mut acc3 = vld1q_s32(bias.as_ptr().add(12));
    let in32_ptr = input.as_ptr() as *const i32;
    let w_ptr = weight.as_ptr();
    for c in 0..(IN / 2) {
        let in0 = vmaxq_s16(
            vreinterpretq_s16_s32(vdupq_n_s32(std::ptr::read_unaligned(in32_ptr.add(c)))),
            zero,
        );
        let in_lo = vget_low_s16(in0);
        let wc = w_ptr.add(c * 32);
        macro_rules! step {
            ($acc:ident, $off:expr) => {{
                let w = vld1q_s16(wc.add($off));
                let lo = vmull_s16(in_lo, vget_low_s16(w));
                let hi = vmull_high_s16(in0, w);
                $acc = vaddq_s32($acc, vpaddq_s32(lo, hi));
            }};
        }
        step!(acc0, 0);
        step!(acc1, 8);
        step!(acc2, 16);
        step!(acc3, 24);
    }
    vst1q_s32(output.as_mut_ptr(), acc0);
    vst1q_s32(output.as_mut_ptr().add(4), acc1);
    vst1q_s32(output.as_mut_ptr().add(8), acc2);
    vst1q_s32(output.as_mut_ptr().add(12), acc3);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn policy_dot16_neon(p_i32: &[i32; 16], w_f32: &[f32; 16], bias: f32) -> f32 {
    let zero = vdupq_n_s32(0);
    let v0 = vcvtq_f32_s32(vmaxq_s32(vld1q_s32(p_i32.as_ptr()), zero));
    let v1 = vcvtq_f32_s32(vmaxq_s32(vld1q_s32(p_i32.as_ptr().add(4)), zero));
    let v2 = vcvtq_f32_s32(vmaxq_s32(vld1q_s32(p_i32.as_ptr().add(8)), zero));
    let v3 = vcvtq_f32_s32(vmaxq_s32(vld1q_s32(p_i32.as_ptr().add(12)), zero));
    let mut acc0 = vmulq_f32(v0, vld1q_f32(w_f32.as_ptr()));
    let mut acc1 = vmulq_f32(v1, vld1q_f32(w_f32.as_ptr().add(4)));
    acc0 = vfmaq_f32(acc0, v2, vld1q_f32(w_f32.as_ptr().add(8)));
    acc1 = vfmaq_f32(acc1, v3, vld1q_f32(w_f32.as_ptr().add(12)));
    vaddvq_f32(vaddq_f32(acc0, acc1)) + bias
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn avg_pool_2x2_u8_neon(
    q00: &mut [i8; 64],
    q01: &mut [i8; 64],
    q10: &mut [i8; 64],
    q11: &mut [i8; 64],
    g: &[[[i8; 64]; 3]; 3],
) {
    for b in 0..4 {
        let off = b * 16;
        let v00 = vld1q_u8(g[0][0].as_ptr().add(off) as *const u8);
        let v01 = vld1q_u8(g[0][1].as_ptr().add(off) as *const u8);
        let v02 = vld1q_u8(g[0][2].as_ptr().add(off) as *const u8);
        let v10 = vld1q_u8(g[1][0].as_ptr().add(off) as *const u8);
        let v11 = vld1q_u8(g[1][1].as_ptr().add(off) as *const u8);
        let v12 = vld1q_u8(g[1][2].as_ptr().add(off) as *const u8);
        let v20 = vld1q_u8(g[2][0].as_ptr().add(off) as *const u8);
        let v21 = vld1q_u8(g[2][1].as_ptr().add(off) as *const u8);
        let v22 = vld1q_u8(g[2][2].as_ptr().add(off) as *const u8);

        let r00 = vrhaddq_u8(vrhaddq_u8(v00, v01), vrhaddq_u8(v10, v11));
        let r01 = vrhaddq_u8(vrhaddq_u8(v01, v02), vrhaddq_u8(v11, v12));
        let r10 = vrhaddq_u8(vrhaddq_u8(v10, v11), vrhaddq_u8(v20, v21));
        let r11 = vrhaddq_u8(vrhaddq_u8(v11, v12), vrhaddq_u8(v21, v22));

        vst1q_u8(q00.as_mut_ptr().add(off) as *mut u8, r00);
        vst1q_u8(q01.as_mut_ptr().add(off) as *mut u8, r01);
        vst1q_u8(q10.as_mut_ptr().add(off) as *mut u8, r10);
        vst1q_u8(q11.as_mut_ptr().add(off) as *mut u8, r11);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[allow(clippy::too_many_arguments)]
pub unsafe fn update_point_neon(
    old_codebook: &[i16; 64],
    new_codebook: &[i16; 64],
    old_map_sum: &[i16; 64],
    new_map_sum: &mut [i16; 64],
    dwconv_weight: &[[i16; 32]; 9],
    map_conv: &mut [[i16; 32]],
    outer_indices: &[u16; 9],
    vsum_group: &mut [i32; 64],
) {
    let zero = vdupq_n_s16(0);
    let mut old_f = [zero; 8];
    let mut new_f = [zero; 8];
    for b in 0..8 {
        let off = b * 8;
        let old_cb = vld1q_s16(old_codebook.as_ptr().add(off));
        let new_cb = vld1q_s16(new_codebook.as_ptr().add(off));
        let old_s = vld1q_s16(old_map_sum.as_ptr().add(off));
        let new_s = vaddq_s16(vsubq_s16(old_s, old_cb), new_cb);
        vst1q_s16(new_map_sum.as_mut_ptr().add(off), new_s);
        old_f[b] = vmaxq_s16(old_s, zero);
        new_f[b] = vmaxq_s16(new_s, zero);
    }

    // Channels 0..32: depthwise 3x3 conv delta
    let old_c0 = vshlq_n_s16::<2>(old_f[0]);
    let old_c1 = vshlq_n_s16::<2>(old_f[1]);
    let old_c2 = vshlq_n_s16::<2>(old_f[2]);
    let old_c3 = vshlq_n_s16::<2>(old_f[3]);
    let new_c0 = vshlq_n_s16::<2>(new_f[0]);
    let new_c1 = vshlq_n_s16::<2>(new_f[1]);
    let new_c2 = vshlq_n_s16::<2>(new_f[2]);
    let new_c3 = vshlq_n_s16::<2>(new_f[3]);

    for k in 0..9 {
        let w_row = &dwconv_weight[8 - k];
        let w0 = vld1q_s16(w_row.as_ptr());
        let w1 = vld1q_s16(w_row.as_ptr().add(8));
        let w2 = vld1q_s16(w_row.as_ptr().add(16));
        let w3 = vld1q_s16(w_row.as_ptr().add(24));
        let d0 = vsubq_s16(mulhi_i16_neon(w0, new_c0), mulhi_i16_neon(w0, old_c0));
        let d1 = vsubq_s16(mulhi_i16_neon(w1, new_c1), mulhi_i16_neon(w1, old_c1));
        let d2 = vsubq_s16(mulhi_i16_neon(w2, new_c2), mulhi_i16_neon(w2, old_c2));
        let d3 = vsubq_s16(mulhi_i16_neon(w3, new_c3), mulhi_i16_neon(w3, old_c3));
        let dst = map_conv
            .get_unchecked_mut(outer_indices[k] as usize)
            .as_mut_ptr();
        vst1q_s16(dst, vaddq_s16(vld1q_s16(dst), d0));
        vst1q_s16(dst.add(8), vaddq_s16(vld1q_s16(dst.add(8)), d1));
        vst1q_s16(dst.add(16), vaddq_s16(vld1q_s16(dst.add(16)), d2));
        vst1q_s16(dst.add(24), vaddq_s16(vld1q_s16(dst.add(24)), d3));
    }

    // Channels 32..64: direct delta into vsum_group[32..64]
    for b in 4..8 {
        let diff = vsubq_s16(new_f[b], old_f[b]);
        let ptr = vsum_group.as_mut_ptr().add(b * 8);
        let cur0 = vld1q_s32(ptr);
        let cur1 = vld1q_s32(ptr.add(4));
        vst1q_s32(ptr, vaddw_s16(cur0, vget_low_s16(diff)));
        vst1q_s32(ptr.add(4), vaddw_high_s16(cur1, diff));
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn commit_conv_cell_neon(
    old_conv: &[i16; 32],
    new_conv: &mut [i16; 32],
    vsum_group: &mut [i32; 64],
) {
    let zero = vdupq_n_s16(0);
    for b in 0..4 {
        let off = b * 8;
        let old_v = vld1q_s16(old_conv.as_ptr().add(off));
        let delta_v = vld1q_s16(new_conv.as_ptr().add(off));
        let new_v = vaddq_s16(old_v, delta_v);
        vst1q_s16(new_conv.as_mut_ptr().add(off), new_v);
        let diff = vsubq_s16(vmaxq_s16(new_v, zero), vmaxq_s16(old_v, zero));
        let ptr = vsum_group.as_mut_ptr().add(off);
        let cur0 = vld1q_s32(ptr);
        let cur1 = vld1q_s32(ptr.add(4));
        vst1q_s32(ptr, vaddw_s16(cur0, vget_low_s16(diff)));
        vst1q_s32(ptr.add(4), vaddw_high_s16(cur1, diff));
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub unsafe fn commit_vsum_neon(
    prev_groups: &[[[i32; 64]; 3]; 3],
    deltas: &[[[i32; 64]; 3]; 3],
    next_groups: &mut [[[i32; 64]; 3]; 3],
    next_global: &mut [i32; 64],
) {
    for b in 0..16 {
        let off = b * 4;
        let mut g_acc = vdupq_n_s32(0);
        for i in 0..3 {
            for j in 0..3 {
                let old_v = vld1q_s32(prev_groups[i][j].as_ptr().add(off));
                let del_v = vld1q_s32(deltas[i][j].as_ptr().add(off));
                let new_v = vaddq_s32(old_v, del_v);
                vst1q_s32(next_groups[i][j].as_mut_ptr().add(off), new_v);
                g_acc = vaddq_s32(g_acc, new_v);
            }
        }
        vst1q_s32(next_global.as_mut_ptr().add(off), g_acc);
    }
}
