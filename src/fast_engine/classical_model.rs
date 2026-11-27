//! Rapfi-compatible classical evaluation and policy model loader (`model210901.bin`).
//!
//! Provides:
//! - Hybrid Classical-NNUE switching gate (`ClassicalModel::margin` + `evaluate_classical`)
//!   matching `opponent/rapfi/Rapfi/eval/eval.cpp`.
//! - Learned `ClassicalPolicyScorer` (`P4SCORES` + `POLICY_CROSS`) for `QVCF` move ordering
//!   matching `opponent/rapfi/Rapfi/eval/classicalpolicy.{h,cpp}`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;

use crate::fast_engine::board::FastBoard;
use crate::fast_engine::pattern::{Pattern4, FE_BLACK, PATTERN4_NB};
use crate::fast_engine::tt::{VALUE_EVAL_MAX, VALUE_EVAL_MIN};
use crate::nnue::lz4::decompress_lz4_frame;

pub const PCODE_NB: usize = 3876;
pub const THREAT_NB: usize = 2048;
pub const TABLE_NB: usize = 4; // 0=Freestyle, 1=Standard, 2=RenjuBlack, 3=RenjuWhite
pub const RULE_NB: usize = 3; // 0=Freestyle, 1=Standard, 2=Renju
pub const POLICY_CONTEXT_NB: usize = 6;
pub const STORED_CONTEXT_NB: usize = 5;
pub const STORED_PATTERN_NB: usize = PATTERN4_NB - 1;
pub const MARGIN_LEN: usize = (VALUE_EVAL_MAX - VALUE_EVAL_MIN + 1) as usize;

const EXPECTED_DECOMPRESSED_BYTES: usize = 8
    + (TABLE_NB * PCODE_NB * 2)
    + (TABLE_NB * THREAT_NB * 2)
    + (TABLE_NB * PCODE_NB * 2 * 2)
    + 8
    + 24
    + (TABLE_NB * STORED_CONTEXT_NB * STORED_PATTERN_NB * STORED_PATTERN_NB * 2);

const EXTENSION_MAGIC: &[u8; 8] = b"RFCLMOD1";
const EXTENSION_VERSION: u32 = 8;
const EXTENSION_PAYLOAD_BYTES: u32 =
    (TABLE_NB * STORED_CONTEXT_NB * STORED_PATTERN_NB * STORED_PATTERN_NB * 2) as u32;

/// Blend weights from `opponent/rapfi/Rapfi/eval/eval.cpp:199-204`:
/// `[CURRENT_BASIC, PREVIOUS_BASIC, CURRENT_THREAT] / 256`.
pub const CLASSICAL_VALUE_BLEND_WEIGHTS: [[i32; 3]; TABLE_NB] = [
    [149, 106, 267],
    [155, 116, 268],
    [137, 122, 263],
    [138, 125, 261],
];

/// Blend weights from `opponent/rapfi/Rapfi/eval/classicalpolicy.h:100-106`:
/// `[SELF_ATTACK, SELF_DEFENSE, OPPONENT_ATTACK, OPPONENT_DEFENSE] / 256`.
pub const CLASSICAL_POLICY_BLEND_WEIGHTS: [[i32; 4]; TABLE_NB] = [
    [235, 264, -25, 121],
    [292, 265, 14, 16],
    [271, 247, 17, 6],
    [289, 272, 16, 40],
];

#[inline(always)]
pub const fn table_index<const RENJU: bool>(side: usize) -> usize {
    if RENJU {
        2 + side
    } else {
        0
    }
}

#[inline(always)]
const fn policy_pattern_from_storage_index(index: usize) -> usize {
    index + (index >= Pattern4::Forbid as usize) as usize
}

pub struct ClassicalModel {
    pub scaling_factor: f32,
    pub evals: [[i16; PCODE_NB]; TABLE_NB],
    pub evals_threat: [[i16; THREAT_NB]; TABLE_NB],
    pub p4_scores: [[[i16; 2]; PCODE_NB]; TABLE_NB],
    pub policy_cross: [[[[i16; PATTERN4_NB]; PATTERN4_NB]; POLICY_CONTEXT_NB]; TABLE_NB],
    pub policy_cross_active_mask: [u8; TABLE_NB],
    pub margin_tables: [[i32; MARGIN_LEN]; RULE_NB],
}

fn read_u32_le(buf: &[u8], off: &mut usize) -> io::Result<u32> {
    if *off + 4 > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated classical model u32",
        ));
    }
    let v = u32::from_le_bytes([buf[*off], buf[*off + 1], buf[*off + 2], buf[*off + 3]]);
    *off += 4;
    Ok(v)
}

fn read_i16_le(buf: &[u8], off: &mut usize) -> io::Result<i16> {
    if *off + 2 > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated classical model i16",
        ));
    }
    let v = i16::from_le_bytes([buf[*off], buf[*off + 1]]);
    *off += 2;
    Ok(v)
}

fn read_f64_le(buf: &[u8], off: &mut usize) -> io::Result<f64> {
    if *off + 8 > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated classical model f64",
        ));
    }
    let v = f64::from_le_bytes([
        buf[*off],
        buf[*off + 1],
        buf[*off + 2],
        buf[*off + 3],
        buf[*off + 4],
        buf[*off + 5],
        buf[*off + 6],
        buf[*off + 7],
    ]);
    *off += 8;
    Ok(v)
}

impl ClassicalModel {
    pub fn from_lz4_file(path: &Path) -> io::Result<Box<Self>> {
        let compressed = std::fs::read(path)?;
        Self::from_lz4_bytes(&compressed)
    }

    pub fn from_lz4_bytes(compressed: &[u8]) -> io::Result<Box<Self>> {
        let raw = decompress_lz4_frame(compressed)?;
        if raw.len() != EXPECTED_DECOMPRESSED_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected model210901 size: got {}, expected {}",
                    raw.len(),
                    EXPECTED_DECOMPRESSED_BYTES
                ),
            ));
        }

        let mut off = 0_usize;
        let scaling_factor = read_f64_le(&raw, &mut off)? as f32;
        if !scaling_factor.is_finite() || scaling_factor <= 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid scaling factor in classical model",
            ));
        }

        let mut model = Box::new(Self {
            scaling_factor,
            evals: [[0; PCODE_NB]; TABLE_NB],
            evals_threat: [[0; THREAT_NB]; TABLE_NB],
            p4_scores: [[[0; 2]; PCODE_NB]; TABLE_NB],
            policy_cross: [[[[0; PATTERN4_NB]; PATTERN4_NB]; POLICY_CONTEXT_NB]; TABLE_NB],
            policy_cross_active_mask: [0; TABLE_NB],
            margin_tables: [[0; MARGIN_LEN]; RULE_NB],
        });

        for t in 0..TABLE_NB {
            for p in 0..PCODE_NB {
                model.evals[t][p] = read_i16_le(&raw, &mut off)?;
            }
        }
        for t in 0..TABLE_NB {
            for th in 0..THREAT_NB {
                model.evals_threat[t][th] = read_i16_le(&raw, &mut off)?;
            }
        }
        for t in 0..TABLE_NB {
            for p in 0..PCODE_NB {
                model.p4_scores[t][p][0] = read_i16_le(&raw, &mut off)?;
                model.p4_scores[t][p][1] = read_i16_le(&raw, &mut off)?;
            }
        }

        if raw.get(off..off + 8) != Some(EXTENSION_MAGIC.as_slice()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing RFCLMOD1 header in classical model",
            ));
        }
        off += 8;
        let version = read_u32_le(&raw, &mut off)?;
        let payload_bytes = read_u32_le(&raw, &mut off)?;
        let table_count = read_u32_le(&raw, &mut off)?;
        let context_count = read_u32_le(&raw, &mut off)?;
        let component_count = read_u32_le(&raw, &mut off)?;
        let pattern_count = read_u32_le(&raw, &mut off)?;
        if version != EXTENSION_VERSION
            || payload_bytes != EXTENSION_PAYLOAD_BYTES
            || table_count != TABLE_NB as u32
            || context_count != STORED_CONTEXT_NB as u32
            || component_count != 0
            || pattern_count != STORED_PATTERN_NB as u32
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "classical model extension metadata mismatch",
            ));
        }

        for t in 0..TABLE_NB {
            let mut active_mask = 0_u8;
            for ctx in 0..STORED_CONTEXT_NB {
                for s_idx in 0..STORED_PATTERN_NB {
                    let s = policy_pattern_from_storage_index(s_idx);
                    for o_idx in 0..STORED_PATTERN_NB {
                        let o = policy_pattern_from_storage_index(o_idx);
                        let score = read_i16_le(&raw, &mut off)?;
                        model.policy_cross[t][ctx][s][o] = score;
                        if score != 0 {
                            active_mask |= 1_u8 << ctx;
                        }
                    }
                }
            }
            model.policy_cross_active_mask[t] = active_mask;
        }
        debug_assert_eq!(off, raw.len());

        // Precompute `classicalEvalMargin` lookup tables (`eval.cpp:85-106`,
        // `evalconfig.h:42-44`).
        const WL_SCALE: [f32; RULE_NB] = [1.22, 0.86, 1.24];
        const WL_EXP: [f32; RULE_NB] = [3.04, 3.44, 2.56];
        const M_SCALE: [f32; RULE_NB] = [351.0, 379.2, 389.4];
        for v in VALUE_EVAL_MIN..=VALUE_EVAL_MAX {
            let win_rate = 1.0_f32 / (1.0_f32 + (-(v as f32) / scaling_factor).exp());
            let wl_rate = 2.0_f32 * (win_rate - 0.5_f32);
            let idx = (v - VALUE_EVAL_MIN) as usize;
            for r in 0..RULE_NB {
                let x = WL_SCALE[r] * wl_rate;
                let x2 = x * x;
                model.margin_tables[r][idx] = (M_SCALE[r] * (-x2.powf(WL_EXP[r])).exp()) as i32;
            }
        }

        Ok(model)
    }

    #[inline(always)]
    pub fn get_value_black<const RENJU: bool>(&self, pcode_b: u16, pcode_w: u16) -> i32 {
        // SAFETY: `pcode_b` and `pcode_w` are generated from `[0..PCODE_NB)` (`< 3876`).
        unsafe {
            let vb = *self
                .evals
                .get_unchecked(table_index::<RENJU>(0))
                .get_unchecked(pcode_b as usize) as i32;
            let vw = *self
                .evals
                .get_unchecked(table_index::<RENJU>(1))
                .get_unchecked(pcode_w as usize) as i32;
            vb - vw
        }
    }

    #[inline(always)]
    pub fn margin<const RENJU: bool>(&self, classical_eval: i32) -> i32 {
        let clamped = classical_eval.clamp(VALUE_EVAL_MIN, VALUE_EVAL_MAX);
        let idx = (clamped - VALUE_EVAL_MIN) as usize;
        let r = if RENJU { 2 } else { 0 };
        self.margin_tables[r][idx]
    }
}

/// Position-bound Rapfi `ClassicalPolicyScorer` (`eval/classicalpolicy.{h,cpp}`).
pub struct ClassicalPolicyScorer<'a> {
    model: &'a ClassicalModel,
    renju: bool,
    self_side: usize,
    table_idx: usize,
    blend_weights: &'static [i32; 4],
    context: usize,
    policy_cross_active: bool,
}

#[inline]
pub fn classify_policy_context(board: &FastBoard, self_side: usize) -> usize {
    let oppo = self_side ^ 1;
    if board.p4_count(oppo, Pattern4::BFlex4) > 0 {
        return 0; // OPPONENT_FLEX_FOUR
    }
    if board.p4_count(oppo, Pattern4::CBlock4Flex3) > 0 {
        return 1; // OPPONENT_BLOCK4_FLEX3
    }
    let mut self_f3_plus = false;
    let mut oppo_f3_plus = false;
    for p4_u8 in (Pattern4::HFlex3 as u8)..=(Pattern4::AFive as u8) {
        let p4 = Pattern4::from_u8(p4_u8);
        self_f3_plus |= board.p4_count(self_side, p4) != 0;
        oppo_f3_plus |= board.p4_count(oppo, p4) != 0;
    }
    if self_f3_plus && oppo_f3_plus {
        2 // MUTUAL_RACE
    } else if self_f3_plus {
        3 // SELF_ATTACKING
    } else if oppo_f3_plus {
        4 // OPPONENT_ATTACKING
    } else {
        5 // QUIET
    }
}

#[inline(always)]
pub const fn compose_classical_policy_score(
    self_attack: i32,
    self_defense: i32,
    opponent_attack: i32,
    opponent_defense: i32,
    weights: &[i32; 4],
) -> i32 {
    (weights[0] * self_attack
        + weights[1] * self_defense
        + weights[2] * opponent_attack
        + weights[3] * opponent_defense)
        / 256
}

impl<'a> ClassicalPolicyScorer<'a> {
    pub fn new(model: &'a ClassicalModel, board: &FastBoard) -> Self {
        let renju = board.rule() == crate::rules::RuleSet::Renju;
        let self_side = board.side_to_move();
        let table_idx = if renju { 2 + self_side } else { 0 };
        let mask = model.policy_cross_active_mask[table_idx];
        let (context, policy_cross_active) = if mask != 0 {
            let ctx = classify_policy_context(board, self_side);
            (ctx, (mask & (1_u8 << ctx)) != 0)
        } else {
            (5, false)
        };
        Self {
            model,
            renju,
            self_side,
            table_idx,
            blend_weights: &CLASSICAL_POLICY_BLEND_WEIGHTS[table_idx],
            context,
            policy_cross_active,
        }
    }

    #[inline(always)]
    pub fn score(
        &self,
        pcode_black: u16,
        pcode_white: u16,
        self_p4: Pattern4,
        oppo_p4: Pattern4,
    ) -> i16 {
        let (t_b, t_w) = if self.renju { (2, 3) } else { (0, 0) };
        let pair_b = self.model.p4_scores[t_b][pcode_black as usize];
        let pair_w = self.model.p4_scores[t_w][pcode_white as usize];
        let (self_pair, oppo_pair) = if self.self_side == FE_BLACK {
            (pair_b, pair_w)
        } else {
            (pair_w, pair_b)
        };
        let mut res = compose_classical_policy_score(
            self_pair[0] as i32,
            oppo_pair[1] as i32,
            oppo_pair[0] as i32,
            self_pair[1] as i32,
            self.blend_weights,
        );
        if self.policy_cross_active {
            res += self.model.policy_cross[self.table_idx][self.context][self_p4 as usize]
                [oppo_p4 as usize] as i32;
        }
        res.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

static CLASSICAL_PTR: AtomicPtr<ClassicalModel> = AtomicPtr::new(std::ptr::null_mut());
static PROBED_DEFAULT: AtomicBool = AtomicBool::new(false);
static LOAD_LOCK: Mutex<()> = Mutex::new(());

fn collect_candidate_paths(hint_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = hint_dir {
        out.push(dir.join("model210901.bin"));
        out.push(dir.join("../classical/model210901.bin"));
    }
    if let Ok(env_dir) = std::env::var("RUST_GOMOKU_NNUE_DIR") {
        let p = PathBuf::from(env_dir.trim());
        out.push(p.join("model210901.bin"));
        out.push(p.join("../classical/model210901.bin"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            out.push(exe_dir.join("nnue/model210901.bin"));
            out.push(exe_dir.join("model210901.bin"));
        }
    }
    out.push(PathBuf::from("nnue/model210901.bin"));
    out.push(PathBuf::from(
        "opponent/rapfi-networks/classical/model210901.bin",
    ));
    out.push(PathBuf::from(
        "opponent/baselines/rapfi_mix9svq/model210901.bin",
    ));
    out
}

/// Ensure the process-wide `ClassicalModel` (`model210901.bin`) is loaded if available.
pub fn warm_classical_model(hint_dir: Option<&Path>) -> Option<&'static ClassicalModel> {
    let ptr = CLASSICAL_PTR.load(Ordering::Acquire);
    if !ptr.is_null() {
        // SAFETY: `ptr` points to a leaked `Box<ClassicalModel>` valid for `'static`.
        return Some(unsafe { &*ptr });
    }
    if hint_dir.is_none() && PROBED_DEFAULT.load(Ordering::Acquire) {
        return None;
    }

    let _guard = LOAD_LOCK.lock().expect("classical model load lock");
    let ptr2 = CLASSICAL_PTR.load(Ordering::Acquire);
    if !ptr2.is_null() {
        return Some(unsafe { &*ptr2 });
    }

    for path in collect_candidate_paths(hint_dir) {
        if path.is_file() {
            if let Ok(model) = ClassicalModel::from_lz4_file(&path) {
                let raw = Box::into_raw(model);
                CLASSICAL_PTR.store(raw, Ordering::Release);
                return Some(unsafe { &*raw });
            }
        }
    }
    if hint_dir.is_none() {
        PROBED_DEFAULT.store(true, Ordering::Release);
    }
    None
}

/// Lock-free lookup of the process-wide `ClassicalModel` (`model210901.bin`).
#[inline(always)]
pub fn classical_model() -> Option<&'static ClassicalModel> {
    let ptr = CLASSICAL_PTR.load(Ordering::Acquire);
    if !ptr.is_null() {
        // SAFETY: `ptr` points to a leaked `Box<ClassicalModel>` valid for `'static`.
        Some(unsafe { &*ptr })
    } else if PROBED_DEFAULT.load(Ordering::Relaxed) {
        None
    } else {
        warm_classical_model(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classical_policy_compose_matches_rapfi_static_checks() {
        assert_eq!(
            compose_classical_policy_score(123, -45, 77, -91, &[256, 256, 0, 0]),
            78
        );
        assert_eq!(
            compose_classical_policy_score(-123, 45, -77, 91, &[256, 256, 0, 0]),
            -78
        );
        assert_eq!(
            compose_classical_policy_score(-123, 0, 0, 0, &[255, 0, 0, 0]),
            -122
        );
    }

    #[test]
    fn classical_model_loads_and_computes_margins() {
        let p = Path::new("opponent/rapfi-networks/classical/model210901.bin");
        if !p.exists() {
            return;
        }
        let m = warm_classical_model(Some(Path::new("opponent/rapfi-networks/mix9svq")))
            .expect("load model210901.bin");
        assert!((m.scaling_factor - 200.0).abs() < 1e-5);
        assert_eq!(m.margin::<false>(0), 351);
        assert_eq!(m.margin::<true>(0), 389);
        assert!(m.margin::<true>(300) < m.margin::<true>(0));
        assert!(m.margin::<true>(800) < m.margin::<true>(300));
        assert_eq!(m.margin::<true>(500), m.margin::<true>(-500));
    }
}
