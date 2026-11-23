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
