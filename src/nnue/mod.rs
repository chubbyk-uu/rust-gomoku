//! Incremental NNUE evaluation (`Mix9svq`) compatible with Rapfi CC0 weights.

pub mod lz4;
pub mod mix9svq;
pub mod simd;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::rules::RuleSet;

pub use mix9svq::{Mix9svqEvaluator, Mix9svqWeight, NNUE_SCALING_FACTOR, NNUE_VALUE_CLAMP};

static WEIGHT_CACHE: OnceLock<Mutex<HashMap<(PathBuf, u8), Arc<Mix9svqWeight>>>> = OnceLock::new();

fn rule_key(rule: RuleSet) -> u8 {
    match rule {
        RuleSet::Freestyle => 0,
        RuleSet::Renju => 1,
    }
}

fn load_cached_weight(path: &Path, rule: RuleSet) -> io::Result<Arc<Mix9svqWeight>> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = (canonical.clone(), rule_key(rule));
    let cache = WEIGHT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(existing) = cache
        .lock()
        .expect("nnue weight cache lock")
        .get(&key)
        .cloned()
    {
        return Ok(existing);
    }
    let loaded = Arc::new(Mix9svqWeight::load_from_lz4_file(&canonical, rule)?);
    let mut guard = cache.lock().expect("nnue weight cache lock");
    let entry = guard.entry(key).or_insert_with(|| loaded.clone());
    Ok(entry.clone())
}

/// Load a `Mix9svqEvaluator` for `rule` from a directory containing Rapfi's
/// `mix9svq*.bin.lz4` weight files.
///
/// Expected filenames inside `dir`:
/// - `RuleSet::Freestyle`: `mix9svqfreestyle_bsmix.bin.lz4`
/// - `RuleSet::Renju`: `mix9svqrenju_bs15_black.bin.lz4` and
///   `mix9svqrenju_bs15_white.bin.lz4`
pub fn load_mix9svq_evaluator_for_rule(dir: &Path, rule: RuleSet) -> io::Result<Mix9svqEvaluator> {
    match rule {
        RuleSet::Freestyle => {
            let w = load_cached_weight(&dir.join("mix9svqfreestyle_bsmix.bin.lz4"), rule)?;
            Ok(Mix9svqEvaluator::new(w.clone(), w))
        }
        RuleSet::Renju => {
            let wb = load_cached_weight(&dir.join("mix9svqrenju_bs15_black.bin.lz4"), rule)?;
            let ww = load_cached_weight(&dir.join("mix9svqrenju_bs15_white.bin.lz4"), rule)?;
            Ok(Mix9svqEvaluator::new(wb, ww))
        }
    }
}
