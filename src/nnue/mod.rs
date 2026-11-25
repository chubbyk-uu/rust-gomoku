//! Incremental NNUE evaluation (`Mix9svq`) compatible with Rapfi CC0 weights.

pub mod lz4;
pub mod mix9svq;
pub mod simd;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::rules::RuleSet;

pub use mix9svq::{Mix9svqEvaluator, Mix9svqWeight, NNUE_SCALING_FACTOR, NNUE_VALUE_CLAMP};

enum WeightCacheEntry {
    Loading(Arc<(Mutex<bool>, Condvar)>),
    Ready(Arc<Mix9svqWeight>),
}

static WEIGHT_CACHE: OnceLock<Mutex<HashMap<(PathBuf, u8), WeightCacheEntry>>> = OnceLock::new();

fn rule_key(rule: RuleSet) -> u8 {
    match rule {
        RuleSet::Freestyle => 0,
        RuleSet::Renju => 1,
    }
}

/// Deduplicated, process-wide cached weight loader: concurrent callers requesting
/// the same `(canonical_path, rule)` wait on the single in-flight loader instead
/// of decoding the same `.bin.lz4` file multiple times in parallel.
fn load_cached_weight(path: &Path, rule: RuleSet) -> io::Result<Arc<Mix9svqWeight>> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = (canonical.clone(), rule_key(rule));
    let cache = WEIGHT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    loop {
        let waiter = {
            let mut guard = cache.lock().expect("nnue weight cache lock");
            match guard.get(&key) {
                Some(WeightCacheEntry::Ready(existing)) => return Ok(existing.clone()),
                Some(WeightCacheEntry::Loading(pair)) => Some(pair.clone()),
                None => {
                    let pair = Arc::new((Mutex::new(false), Condvar::new()));
                    guard.insert(key.clone(), WeightCacheEntry::Loading(pair));
                    None
                }
            }
        };

        if let Some(pair) = waiter {
            let (lock, cvar) = &*pair;
            let mut done = lock.lock().expect("nnue weight waiter lock");
            while !*done {
                done = cvar.wait(done).expect("nnue weight waiter cvar");
            }
            continue;
        }

        // We own the in-flight slot for `key`.
        let load_res = Mix9svqWeight::load_from_lz4_file(&canonical, rule).map(Arc::new);
        let pair = {
            let mut guard = cache.lock().expect("nnue weight cache lock");
            let prev = match &load_res {
                Ok(loaded) => guard.insert(key.clone(), WeightCacheEntry::Ready(loaded.clone())),
                Err(_) => guard.remove(&key),
            };
            match prev {
                Some(WeightCacheEntry::Loading(p)) => Some(p),
                _ => None,
            }
        };
        if let Some(pair) = pair {
            let (lock, cvar) = &*pair;
            let mut done = lock.lock().expect("nnue weight waiter lock");
            *done = true;
            cvar.notify_all();
        }
        return load_res;
    }
}

/// Standard Rapfi `Mix9svq` weight filenames used for Freestyle and Renju.
pub const MIX9SVQ_FILENAMES: &[&str] = &[
    "mix9svqfreestyle_bsmix.bin.lz4",
    "mix9svqrenju_bs15_black.bin.lz4",
    "mix9svqrenju_bs15_white.bin.lz4",
];

/// Fast metadata check whether the expected `.bin.lz4` weight files for
/// `(dir, rule)` exist on disk.
pub fn mix9svq_files_exist_for_rule(dir: &Path, rule: RuleSet) -> bool {
    match rule {
        RuleSet::Freestyle => dir.join("mix9svqfreestyle_bsmix.bin.lz4").exists(),
        RuleSet::Renju => {
            dir.join("mix9svqrenju_bs15_black.bin.lz4").exists()
                && dir.join("mix9svqrenju_bs15_white.bin.lz4").exists()
        }
    }
}

/// Fast metadata check whether `dir` contains valid `Mix9svq` weight files for at
/// least one supported ruleset (Freestyle or Renju).
pub fn dir_has_any_mix9svq(dir: &Path) -> bool {
    mix9svq_files_exist_for_rule(dir, RuleSet::Freestyle)
        || mix9svq_files_exist_for_rule(dir, RuleSet::Renju)
}

/// Resolve the NNUE weight directory for interactive applications (desktop GUI).
///
/// Search order:
/// 1. Explicit path (e.g. CLI `--nnue-dir`).
/// 2. `RUST_GOMOKU_NNUE_DIR` environment variable.
/// 3. `<exe_dir>/nnue`, `<exe_dir>`, `./nnue`, and repo dev path
///    `opponent/rapfi-networks/mix9svq` (first existing directory containing
///    valid `Mix9svq` files).
pub fn resolve_default_nnue_dir(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = explicit.filter(|s| !s.trim().is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Ok(env_dir) = std::env::var("RUST_GOMOKU_NNUE_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("nnue"));
            candidates.push(exe_dir.to_path_buf());
        }
    }
    candidates.push(PathBuf::from("nnue"));
    candidates.push(PathBuf::from("opponent/rapfi-networks/mix9svq"));

    candidates.into_iter().find(|dir| dir_has_any_mix9svq(dir))
}

/// Non-blocking check whether the `Mix9svq` weights for `(dir, rule)` are
/// already decoded and resident in the process-wide `WEIGHT_CACHE`.
pub fn is_mix9svq_cached_for_rule(dir: &Path, rule: RuleSet) -> bool {
    let Some(cache) = WEIGHT_CACHE.get() else {
        return false;
    };
    let guard = cache.lock().expect("nnue weight cache lock");
    let has_path = |path: PathBuf| {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        matches!(
            guard.get(&(canonical, rule_key(rule))),
            Some(WeightCacheEntry::Ready(_))
        )
    };
    match rule {
        RuleSet::Freestyle => has_path(dir.join("mix9svqfreestyle_bsmix.bin.lz4")),
        RuleSet::Renju => {
            has_path(dir.join("mix9svqrenju_bs15_black.bin.lz4"))
                && has_path(dir.join("mix9svqrenju_bs15_white.bin.lz4"))
        }
    }
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
