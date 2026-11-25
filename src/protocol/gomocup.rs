//! Gomocup protocol adapter aligned with the reference implementation.

use std::sync::{Arc, Condvar, Mutex};

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::{
    default_tt_bits_for_search, load_default_config, EngineConfig, MAX_SEARCH_THREADS,
};
use crate::constants::{BOARD_AREA, BOARD_SIZE};
use crate::rules::RuleSet;
use crate::search::{RootSearcher, SearchLimits, TranspositionTable, MAX_TT_BUCKET_BITS};

const MAX_PROTOCOL_TIME_MS: f64 = i32::MAX as f64;
const PREWARM_LOADING: u8 = 0;
const PREWARM_READY: u8 = 1;
const PREWARM_FAILED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrewarmStatus {
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingProtocolError {
    TimeLimitTooLarge,
    TtBitsOutOfRange,
}

impl PendingProtocolError {
    fn message(self) -> String {
        match self {
            Self::TimeLimitTooLarge => "time limit is too large".to_string(),
            Self::TtBitsOutOfRange => {
                format!("tt_bits must be between 0 and {MAX_TT_BUCKET_BITS}")
            }
        }
    }
}

pub const ABOUT_TEXT: &str = "name=\"rust_gomoku\", version=\"0.1\", author=\"OpenAI\", country=\"China\", www=\"https://example.invalid/\"";

#[derive(Clone, Debug)]
pub struct GomocupProtocol {
    pub config: EngineConfig,
    pub default_limits: Option<SearchLimits>,
    pub board: Board,
    pub board_mode: bool,
    pub board_lines: Vec<(usize, usize, i32)>,
    pub timeout_turn_ms: Option<f64>,
    pub time_left_ms: Option<f64>,
    pub node_limit: Option<usize>,
    pub tt_bits: Option<u32>,
    pub ended: bool,
    pending_error: Option<PendingProtocolError>,
    searcher: Option<RootSearcher>,
    auto_tt_max_bits: u32,
    max_memory_tt_bits: Option<u32>,
    nnue_prewarm: Option<(std::path::PathBuf, RuleSet, Arc<(Mutex<u8>, Condvar)>)>,
}

impl Default for GomocupProtocol {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl GomocupProtocol {
    pub fn new(config: Option<EngineConfig>, search_limits: Option<SearchLimits>) -> Self {
        Self {
            config: config.unwrap_or_else(load_default_config),
            default_limits: search_limits,
            board: Board::new(),
            board_mode: false,
            board_lines: Vec::new(),
            timeout_turn_ms: None,
            time_left_ms: None,
            node_limit: None,
            tt_bits: None,
            ended: false,
            pending_error: None,
            searcher: None,
            auto_tt_max_bits: MAX_TT_BUCKET_BITS,
            max_memory_tt_bits: None,
            nnue_prewarm: None,
        }
    }

    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        let raw = line.trim();
        if raw.is_empty() {
            return Vec::new();
        }

        if self.board_mode {
            if raw.eq_ignore_ascii_case("DONE") {
                self.board_mode = false;
                return self.handle_board_done();
            }
            let parts: Vec<_> = raw.split(',').collect();
            if parts.len() != 3 {
                return vec!["ERROR Board format error.".to_string()];
            }
            let Some(x) = parse_one::<i32>(parts[0]) else {
                return vec!["ERROR Board format error.".to_string()];
            };
            let Some(y) = parse_one::<i32>(parts[1]) else {
                return vec!["ERROR Board format error.".to_string()];
            };
            let Some(side) = parse_one::<i32>(parts[2]) else {
                return vec!["ERROR Board format error.".to_string()];
            };
            if !in_bounds_i32(x, y) {
                return vec!["ERROR Board format error.".to_string()];
            }
            self.board_lines.push((x as usize, y as usize, side));
            return Vec::new();
        }

        let parts: Vec<_> = raw.split_whitespace().collect();
        let command = parts[0].to_ascii_uppercase();

        match command.as_str() {
            "START" => self.handle_start(&parts),
            "RECTSTART" => self.handle_rectstart(&parts),
            "RESTART" => {
                self.reset_engine();
                vec!["OK".to_string()]
            }
            "BEGIN" => self.search_move(),
            "TURN" => self.handle_turn(&parts),
            "BOARD" => {
                self.board_mode = true;
                self.board_lines.clear();
                Vec::new()
            }
            "INFO" => {
                let rest = raw[parts[0].len()..].trim_start();
                self.handle_info(&parts[1..], rest);
                Vec::new()
            }
            "TAKEBACK" => self.handle_takeback(&parts),
            "ABOUT" => vec![ABOUT_TEXT.to_string()],
            "END" => {
                self.ended = true;
                Vec::new()
            }
            _ => vec!["UNKNOWN".to_string()],
        }
    }

    fn handle_start(&mut self, parts: &[&str]) -> Vec<String> {
        if parts.len() != 2 {
            return vec!["ERROR Size error.".to_string()];
        }
        let Some(size) = parse_one::<usize>(parts[1]) else {
            return vec!["ERROR Size error.".to_string()];
        };
        if size != BOARD_SIZE {
            return vec!["ERROR Size error.".to_string()];
        }
        self.reset_engine();
        self.prewarm_nnue_weights();
        vec!["OK".to_string()]
    }

    fn handle_rectstart(&mut self, parts: &[&str]) -> Vec<String> {
        if parts.len() != 2 || !parts[1].contains(',') {
            return vec!["ERROR Size error.".to_string()];
        }
        let dims: Vec<_> = parts[1].splitn(2, ',').collect();
        let Some(sx) = parse_one::<usize>(dims[0]) else {
            return vec!["ERROR Size error.".to_string()];
        };
        let Some(sy) = parse_one::<usize>(dims[1]) else {
            return vec!["ERROR Size error.".to_string()];
        };
        if sx != BOARD_SIZE || sy != BOARD_SIZE {
            return vec!["ERROR Size error.".to_string()];
        }
        self.reset_engine();
        self.prewarm_nnue_weights();
        vec!["OK".to_string()]
    }

    fn handle_turn(&mut self, parts: &[&str]) -> Vec<String> {
        if parts.len() != 2 || !parts[1].contains(',') {
            return vec!["ERROR Turn format error.".to_string()];
        }
        let coords: Vec<_> = parts[1].splitn(2, ',').collect();
        let Some(x) = parse_one::<i32>(coords[0]) else {
            return vec!["ERROR Turn format error.".to_string()];
        };
        let Some(y) = parse_one::<i32>(coords[1]) else {
            return vec!["ERROR Turn format error.".to_string()];
        };
        if !in_bounds_i32(x, y) {
            return vec!["ERROR Turn format error.".to_string()];
        }
        let move_ = xy_to_move(x as usize, y as usize).expect("bounds checked");
        if !self.board.is_legal_move(move_) {
            return vec!["ERROR Illegal move.".to_string()];
        }
        let side = self.board.side_to_move();
        if self.play_xy(x as usize, y as usize, side).is_err() {
            return vec!["ERROR Illegal move.".to_string()];
        }
        self.search_move()
    }

    fn reset_engine(&mut self) {
        self.board = Board::new();
        self.searcher = None;
        self.board_mode = false;
        self.board_lines.clear();
        self.pending_error = None;
    }

    fn handle_takeback(&mut self, parts: &[&str]) -> Vec<String> {
        if parts.len() != 2 || !parts[1].contains(',') {
            return vec!["ERROR Takeback error.".to_string()];
        }
        let coords: Vec<_> = parts[1].splitn(2, ',').collect();
        let (Some(x), Some(y)) = (parse_one::<i32>(coords[0]), parse_one::<i32>(coords[1])) else {
            return vec!["ERROR Takeback error.".to_string()];
        };
        if !in_bounds_i32(x, y) {
            return vec!["ERROR Takeback error.".to_string()];
        }
        let Some(last) = self.board.move_history().last() else {
            return vec!["ERROR Takeback error.".to_string()];
        };
        let expected = xy_to_move(x as usize, y as usize).expect("bounds checked");
        if last.move_ != expected || self.board.undo().is_err() {
            return vec!["ERROR Takeback error.".to_string()];
        }
        vec!["OK".to_string()]
    }

    fn play_xy(&mut self, x: usize, y: usize, side: i8) -> Result<(), ()> {
        let move_ = xy_to_move(x, y).map_err(|_| ())?;
        if !self
            .board
            .is_legal_move_for_rule(move_, side, self.config.rule_set)
        {
            return Err(());
        }
        self.board.force_side_to_move(side).map_err(|_| ())?;
        self.board
            .play_for_rule(move_, Some(side), self.config.rule_set)
            .map_err(|_| ())?;
        Ok(())
    }

    fn handle_board_done(&mut self) -> Vec<String> {
        let black_moves: Vec<_> = self
            .board_lines
            .iter()
            .filter_map(|&(x, y, side)| (side == 1).then_some((x, y)))
            .collect();
        let white_moves: Vec<_> = self
            .board_lines
            .iter()
            .filter_map(|&(x, y, side)| (side != 1).then_some((x, y)))
            .collect();

        let (first_side_moves, second_side_moves) = if black_moves.len() == white_moves.len() {
            (black_moves, white_moves)
        } else if black_moves.len() + 1 == white_moves.len() {
            (white_moves, black_moves)
        } else {
            self.board_lines.clear();
            return vec!["ERROR Board error.".to_string()];
        };

        self.board = Board::new();
        let mut failed = false;
        let max_len = first_side_moves.len().max(second_side_moves.len());
        for idx in 0..max_len {
            if idx < first_side_moves.len() {
                let (x, y) = first_side_moves[idx];
                let side = self.board.side_to_move();
                failed |= self.play_xy(x, y, side).is_err();
            }
            if idx < second_side_moves.len() {
                let (x, y) = second_side_moves[idx];
                let side = self.board.side_to_move();
                failed |= self.play_xy(x, y, side).is_err();
            }
            if failed {
                break;
            }
        }

        if failed {
            self.board = Board::new();
            self.board_lines.clear();
            return vec!["ERROR Board error.".to_string()];
        }

        self.board_lines.clear();
        self.search_move()
    }

    fn handle_info(&mut self, args: &[&str], raw_rest: &str) {
        if args.is_empty() {
            return;
        }
        let key = args[0].to_ascii_lowercase();
        if key == "nnue_dir" {
            let raw_val = raw_rest.get(args[0].len()..).unwrap_or("").trim();
            let unquoted = if raw_val.len() >= 2
                && ((raw_val.starts_with('"') && raw_val.ends_with('"'))
                    || (raw_val.starts_with('\'') && raw_val.ends_with('\'')))
            {
                &raw_val[1..raw_val.len() - 1]
            } else {
                raw_val
            };
            let trimmed = unquoted.trim();
            if trimmed.is_empty() {
                self.config.runtime.nnue_dir = None;
                self.config.runtime.use_nnue = false;
            } else {
                self.config.runtime.nnue_dir = Some(trimmed.to_string());
                self.config.runtime.use_nnue = true;
            }
            self.searcher = None;
            self.nnue_prewarm = None;
            let _ = self.ensure_prewarm_status();
            return;
        }
        if args.len() < 2 {
            return;
        }
        let value = args[1];
        match key.as_str() {
            "timeout_turn" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.timeout_turn_ms = Some(if parsed == 0.0 { 200.0 } else { parsed });
                    self.clear_pending_error(PendingProtocolError::TimeLimitTooLarge);
                } else if is_oversized_time(value) {
                    self.pending_error = Some(PendingProtocolError::TimeLimitTooLarge);
                }
            }
            "timeout_match" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.time_left_ms = Some(if parsed == 0.0 { 99_999_999.0 } else { parsed });
                    self.clear_pending_error(PendingProtocolError::TimeLimitTooLarge);
                } else if is_oversized_time(value) {
                    self.pending_error = Some(PendingProtocolError::TimeLimitTooLarge);
                }
            }
            "time_left" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.time_left_ms = Some(parsed);
                    self.clear_pending_error(PendingProtocolError::TimeLimitTooLarge);
                } else if is_oversized_time(value) {
                    self.pending_error = Some(PendingProtocolError::TimeLimitTooLarge);
                }
            }
            "max_node" => {
                if let Some(parsed) = parse_one::<i64>(value) {
                    self.node_limit = (parsed > 0).then_some(parsed as usize);
                }
            }
            "compute_vcf" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.compute_vcf = parsed != 0;
                    self.searcher = None;
                }
            }
            "root_vcf_depth" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.set_root_vcf_depth(parsed.max(0));
                    self.searcher = None;
                }
            }
            "opponent_vcf_depth" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.opponent_vcf_depth = parsed.max(0);
                    self.searcher = None;
                }
            }
            "vct_verify_opponent_vcf_depth" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.vct_verify_opponent_vcf_depth = parsed.max(0);
                    self.searcher = None;
                }
            }
            "vcf_multi_reply" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.vcf_multi_reply = parsed != 0;
                    self.searcher = None;
                }
            }
            "nonroot_vcf" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.nonroot_vcf = parsed != 0;
                    self.searcher = None;
                }
            }
            "compute_vct" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.set_compute_vct(parsed != 0);
                    self.searcher = None;
                }
            }
            "root_vct_depth" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.root_vct_depth = parsed.max(0);
                    self.searcher = None;
                }
            }
            "vct_use_dfpn" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.vct_use_dfpn = parsed != 0;
                    self.searcher = None;
                }
            }
            "use_nnue" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.use_nnue = parsed != 0;
                    self.searcher = None;
                    let _ = self.ensure_prewarm_status();
                }
            }
            "vct_strict_and_memo_key" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.vct_strict_and_memo_key = parsed != 0;
                    self.searcher = None;
                }
            }
            "overlap_vct_alphabeta" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.overlap_vct_alphabeta = parsed != 0;
                    self.searcher = None;
                }
            }
            "thread_num" | "threads" => {
                if let Some(parsed) = parse_one::<usize>(value) {
                    self.config.runtime.threads = parsed.clamp(1, MAX_SEARCH_THREADS);
                    self.searcher = None;
                }
            }
            "max_memory" => {
                if let Some(parsed) = parse_one::<u64>(value) {
                    let new_cap = if parsed == 0 {
                        None
                    } else {
                        let buckets = parsed / 32;
                        let bits = if buckets == 0 {
                            0
                        } else {
                            buckets.ilog2().min(MAX_TT_BUCKET_BITS)
                        };
                        Some(bits)
                    };
                    if self.max_memory_tt_bits != new_cap {
                        self.max_memory_tt_bits = new_cap;
                        self.searcher = None;
                    }
                }
            }
            "tt_bits" => {
                if let Some(parsed) = parse_one::<u32>(value) {
                    if parsed <= MAX_TT_BUCKET_BITS {
                        self.tt_bits = Some(parsed);
                        self.searcher = None;
                        self.clear_pending_error(PendingProtocolError::TtBitsOutOfRange);
                    } else {
                        self.pending_error = Some(PendingProtocolError::TtBitsOutOfRange);
                    }
                }
            }
            "rule" => {
                if self.board.move_count() == 0 {
                    if let Ok(parsed) = value.parse::<RuleSet>() {
                        if self.config.rule_set != parsed {
                            self.config.rule_set = parsed;
                            self.searcher = None;
                            let _ = self.ensure_prewarm_status();
                        }
                    }
                }
            }
            "root_profile" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.root_profile = parsed != 0;
                }
            }
            "static" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.static_board = parsed % 2 != 0;
                    self.searcher = None;
                }
            }
            "dynamic_board_margin" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.dynamic_board_margin = parsed.max(0);
                    self.searcher = None;
                }
            }
            _ => {}
        }
    }

    fn prewarm_nnue_weights(&mut self) {
        if !self.config.runtime.use_nnue {
            self.nnue_prewarm = None;
            return;
        }
        if let Some(dir) = self.config.runtime.nnue_dir.clone() {
            let path = std::path::Path::new(&dir);
            let canonical_dir = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let res = crate::nnue::load_mix9svq_evaluator_for_rule(path, self.config.rule_set);
            let code = if res.is_ok() {
                PREWARM_READY
            } else {
                PREWARM_FAILED
            };
            self.nnue_prewarm = Some((
                canonical_dir,
                self.config.rule_set,
                Arc::new((Mutex::new(code), Condvar::new())),
            ));
            let other = match self.config.rule_set {
                RuleSet::Freestyle => RuleSet::Renju,
                RuleSet::Renju => RuleSet::Freestyle,
            };
            let _ = crate::nnue::load_mix9svq_evaluator_for_rule(path, other);
        }
    }

    /// Deduplicated, non-blocking prewarm state machine for `(canonical_dir, rule_set)`.
    /// Spawns at most one background loader per `(canonical_dir, rule)` and caches
    /// `Failed` until the NNUE config or symlink target changes.
    fn ensure_prewarm_status(&mut self) -> Option<PrewarmStatus> {
        if !self.config.runtime.use_nnue {
            self.nnue_prewarm = None;
            return None;
        }
        let Some(dir) = self.config.runtime.nnue_dir.clone() else {
            self.nnue_prewarm = None;
            return Some(PrewarmStatus::Failed);
        };
        let rule = self.config.rule_set;
        let path = std::path::Path::new(&dir);
        let canonical_dir = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if crate::nnue::is_mix9svq_cached_for_rule(&canonical_dir, rule) {
            self.nnue_prewarm = Some((
                canonical_dir,
                rule,
                Arc::new((Mutex::new(PREWARM_READY), Condvar::new())),
            ));
            return Some(PrewarmStatus::Ready);
        }
        if let Some((prev_dir, prev_rule, pair)) = &self.nnue_prewarm {
            if prev_dir == &canonical_dir && *prev_rule == rule {
                let state = *pair.0.lock().expect("nnue prewarm state lock");
                match state {
                    PREWARM_FAILED => return Some(PrewarmStatus::Failed),
                    PREWARM_LOADING => return Some(PrewarmStatus::Loading),
                    // `PREWARM_READY` but not present in `WEIGHT_CACHE` means the
                    // underlying target changed; fall through to re-warm.
                    _ => {}
                }
            }
        }
        // Fail fast without spawning a thread when the weight files do not exist.
        if !crate::nnue::mix9svq_files_exist_for_rule(&canonical_dir, rule) {
            self.nnue_prewarm = Some((
                canonical_dir,
                rule,
                Arc::new((Mutex::new(PREWARM_FAILED), Condvar::new())),
            ));
            return Some(PrewarmStatus::Failed);
        }
        let pair = Arc::new((Mutex::new(PREWARM_LOADING), Condvar::new()));
        self.nnue_prewarm = Some((canonical_dir.clone(), rule, Arc::clone(&pair)));
        let pair_bg = Arc::clone(&pair);
        let spawn_res = std::thread::Builder::new()
            .name("gomoku-nnue-prewarm".to_string())
            .spawn(move || {
                let code = match crate::nnue::load_mix9svq_evaluator_for_rule(&canonical_dir, rule)
                {
                    Ok(_) => PREWARM_READY,
                    Err(_) => PREWARM_FAILED,
                };
                let (lock, cvar) = &*pair_bg;
                let mut guard = lock.lock().expect("nnue prewarm state lock");
                *guard = code;
                cvar.notify_all();
            });
        if spawn_res.is_err() {
            let (lock, cvar) = &*pair;
            let mut guard = lock.lock().expect("nnue prewarm state lock");
            *guard = PREWARM_FAILED;
            cvar.notify_all();
            return Some(PrewarmStatus::Failed);
        }
        Some(PrewarmStatus::Loading)
    }

    /// Wait up to `max_wait` for an in-flight prewarm to finish, returning the
    /// latest `PrewarmStatus`. Never blocks beyond `max_wait`.
    fn wait_prewarm_bounded(&self, max_wait: std::time::Duration) -> PrewarmStatus {
        let Some((_, _, pair)) = &self.nnue_prewarm else {
            return PrewarmStatus::Failed;
        };
        let (lock, cvar) = &**pair;
        let mut guard = lock.lock().expect("nnue prewarm state lock");
        let deadline = std::time::Instant::now() + max_wait;
        while *guard == PREWARM_LOADING {
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                break;
            };
            if remaining.is_zero() {
                break;
            }
            let (next_guard, timeout_res) = cvar
                .wait_timeout(guard, remaining)
                .expect("nnue prewarm cvar wait");
            guard = next_guard;
            if timeout_res.timed_out() {
                break;
            }
        }
        match *guard {
            PREWARM_READY => PrewarmStatus::Ready,
            PREWARM_FAILED => PrewarmStatus::Failed,
            _ => PrewarmStatus::Loading,
        }
    }

    /// Strictly non-blocking check whether match-clock pacing should apply.
    /// Returns `true` during both `Loading` and `Ready` states so a cold-start
    /// move in a match-clock game never consumes the entire `time_left`; returns
    /// `false` once `Failed` (or classical mode) so classical semantics apply.
    fn should_pace_match_clock(&self) -> bool {
        if !self.config.runtime.use_nnue {
            return false;
        }
        if let Some(searcher) = &self.searcher {
            return searcher.has_nnue();
        }
        let Some(dir) = self.config.runtime.nnue_dir.as_deref() else {
            return false;
        };
        let rule = self.config.rule_set;
        let path = std::path::Path::new(dir);
        let canonical_dir = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some((prev_dir, prev_rule, pair)) = &self.nnue_prewarm {
            if prev_dir == &canonical_dir && *prev_rule == rule {
                let state = *pair.0.lock().expect("nnue prewarm state lock");
                return state != PREWARM_FAILED;
            }
        }
        crate::nnue::is_mix9svq_cached_for_rule(&canonical_dir, rule)
            || crate::nnue::mix9svq_files_exist_for_rule(&canonical_dir, rule)
    }

    pub fn current_search_limits(&self) -> SearchLimits {
        let match_budget = self.time_left_ms.map(|left| {
            // Keep classical base protocol alignment 100% exact; enable dynamic
            // match-clock pacing when NNUE mode is ready or loading and a real
            // match clock (below the timeout_match=0 sentinel) is in effect.
            if left >= 50_000_000.0 || !self.should_pace_match_clock() {
                return left;
            }
            let plies = self.board.move_count();
            let expected_moves_left = (40_usize.saturating_sub(plies / 4)).max(12) as f64;
            let reserve = (left * 0.05).min(50.0);
            let slice = ((left - reserve).max(1.0) / expected_moves_left).max(10.0);
            slice.min(left)
        });
        let time_budget = match (self.timeout_turn_ms, match_budget) {
            (Some(turn), Some(match_ms)) => Some(turn.min(match_ms)),
            (Some(turn), None) => Some(turn),
            (None, Some(match_ms)) => Some(match_ms),
            (None, None) => None,
        };
        if let Some(default_limits) = self.default_limits {
            SearchLimits {
                max_depth: default_limits.max_depth,
                root_width: default_limits.root_width,
                node_limit: self.node_limit.or(default_limits.node_limit),
                time_limit_ms: time_budget.or(default_limits.time_limit_ms),
            }
        } else {
            let has_time_control = self.timeout_turn_ms.is_some() || self.time_left_ms.is_some();
            let defaults = if has_time_control {
                SearchLimits::timed_from_config(&self.config)
            } else {
                SearchLimits::fixed_from_config(&self.config)
            };
            SearchLimits {
                max_depth: defaults.max_depth,
                root_width: defaults.root_width,
                node_limit: self.node_limit,
                time_limit_ms: time_budget,
            }
        }
    }

    fn search_move(&mut self) -> Vec<String> {
        let cmd_start = std::time::Instant::now();
        if let Some(error) = self.pending_error.take() {
            return vec![format!("ERROR {}.", error.message())];
        }
        if self.board.is_draw() {
            return vec!["ERROR Board is full.".to_string()];
        }
        if self.board.winner() != 0 {
            return vec!["ERROR Board is terminal.".to_string()];
        }
        let mut limits = self.current_search_limits();
        let expected_auto_bits = self.tt_bits.is_none().then(|| {
            let effective_threads = if limits.node_limit.is_some() {
                1
            } else {
                self.config.runtime.threads
            };
            let mut bits = default_tt_bits_for_search(effective_threads, limits.max_depth)
                .min(self.auto_tt_max_bits);
            if let Some(mem_bits) = self.max_memory_tt_bits {
                bits = bits.min(mem_bits);
            }
            bits
        });
        // Check deduplicated prewarm state for NNUE weights:
        // - Under ANY timed budget, never wait unboundedly for slow storage:
        //   wait at most a bounded fraction of the turn budget (`min(budget*0.4,
        //   budget-30ms)`) for an in-flight prewarm. If still `Loading` when
        //   that wait cap expires, play this single move with classical fallback
        //   inside the remaining turn budget (without caching the temporary
        //   searcher so the next move switches to NNUE once ready).
        // - `Failed`: reuse and cache a normal classical searcher with the
        //   exact requested `tt_bits` until NNUE config changes.
        let mut prewarm_status = self.ensure_prewarm_status();
        if prewarm_status == Some(PrewarmStatus::Loading) {
            if let Some(budget_ms) = limits.time_limit_ms {
                let elapsed_ms = cmd_start.elapsed().as_secs_f64() * 1000.0;
                let remaining_ms = (budget_ms - elapsed_ms).max(0.0);
                let wait_cap_ms = (remaining_ms * 0.4).min(remaining_ms - 30.0).max(0.0);
                if wait_cap_ms > 0.0 {
                    prewarm_status = Some(self.wait_prewarm_bounded(
                        std::time::Duration::from_secs_f64(wait_cap_ms / 1000.0),
                    ));
                }
            }
        }
        let cold_nnue_timed_fallback =
            prewarm_status == Some(PrewarmStatus::Loading) && limits.time_limit_ms.is_some();
        let nnue_failed = prewarm_status == Some(PrewarmStatus::Failed);

        let mut effective_config = self.config.clone();
        if cold_nnue_timed_fallback || nnue_failed {
            effective_config.runtime.use_nnue = false;
        }
        let cached_searcher = if cold_nnue_timed_fallback {
            None
        } else {
            self.searcher.take().filter(|searcher| {
                expected_auto_bits
                    .is_none_or(|bits| searcher.tt.bucket_count() == (1_usize << bits))
            })
        };
        let mut searcher = match cached_searcher {
            Some(searcher) => searcher,
            None => {
                // Resolve NNUE weights BEFORE allocating the TT so missing weights
                // allocate only the classical 2-way plane instead of 4-way + 2-way.
                let nnue = crate::search::alphabeta::load_nnue_for_config(&effective_config);
                if effective_config.runtime.use_nnue {
                    let final_code = if nnue.is_some() {
                        PREWARM_READY
                    } else {
                        PREWARM_FAILED
                    };
                    if let Some(dir) = effective_config.runtime.nnue_dir.as_deref() {
                        let p = std::path::Path::new(dir);
                        let canonical_dir =
                            std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                        self.nnue_prewarm = Some((
                            canonical_dir,
                            effective_config.rule_set,
                            Arc::new((Mutex::new(final_code), Condvar::new())),
                        ));
                    }
                    if nnue.is_none() {
                        effective_config.runtime.use_nnue = false;
                    }
                }
                let prefer_four_way_tt = nnue.is_some();
                let tt_result = if cold_nnue_timed_fallback {
                    let bits = self.tt_bits.map(|b| b.min(16)).unwrap_or(16);
                    TranspositionTable::try_new_with_layout(bits, false).map(|tt| (tt, None))
                } else {
                    match self.tt_bits {
                        Some(bits) => {
                            TranspositionTable::try_new_with_layout(bits, prefer_four_way_tt)
                                .map(|tt| (tt, None))
                        }
                        None => {
                            let preferred_bits = expected_auto_bits.expect("auto bits computed");
                            let floor_bits = preferred_bits.min(16);
                            TranspositionTable::try_new_with_fallback_layout(
                                preferred_bits,
                                floor_bits,
                                prefer_four_way_tt,
                            )
                            .map(|(tt, actual_bits)| {
                                let clamped = (actual_bits < preferred_bits).then_some(actual_bits);
                                (tt, clamped)
                            })
                        }
                    }
                };
                match tt_result {
                    Ok((tt, clamped_bits)) => {
                        if let Some(actual_bits) = clamped_bits {
                            self.auto_tt_max_bits = actual_bits;
                        }
                        RootSearcher::with_tt_and_nnue(effective_config.clone(), tt, nnue)
                    }
                    Err(_) => {
                        return vec!["ERROR Unable to allocate transposition table.".to_string()]
                    }
                }
            }
        };
        searcher.config = effective_config;
        if let Some(budget_ms) = limits.time_limit_ms.as_mut() {
            let setup_ms = cmd_start.elapsed().as_secs_f64() * 1000.0;
            *budget_ms = (*budget_ms - setup_ms).max(1.0);
        }
        let result = searcher
            .try_search(&mut self.board, Some(limits))
            .expect("protocol checked the board is non-terminal");
        let trace = searcher.last_trace.clone();
        if !cold_nnue_timed_fallback {
            self.searcher = Some(searcher);
        }

        let mut move_ = result.move_;
        if !self.board.is_legal_move_for_rule(
            move_,
            self.board.side_to_move(),
            self.config.rule_set,
        ) {
            let Some(fallback) = (0..BOARD_AREA as u16).find(|&candidate| {
                self.board.is_legal_move_for_rule(
                    candidate,
                    self.board.side_to_move(),
                    self.config.rule_set,
                )
            }) else {
                return vec!["ERROR No legal move.".to_string()];
            };
            move_ = fallback;
        }
        let (x, y) = move_to_xy(move_).expect("selected move stays valid");
        let side = self.board.side_to_move();
        self.play_xy(x, y, side)
            .expect("selected protocol move is legal");
        let mut responses = trace_messages(trace.as_ref());
        responses.push(format!("{x},{y}"));
        responses
    }

    fn clear_pending_error(&mut self, error: PendingProtocolError) {
        if self.pending_error == Some(error) {
            self.pending_error = None;
        }
    }
}

fn trace_messages(trace: Option<&crate::search::RootTrace>) -> Vec<String> {
    let Some(trace) = trace else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    if trace.tt_bestmove_current_generation > 0 || trace.tt_bestmove_old_generation > 0 {
        messages.push(format!(
            "MESSAGE tt_generation current={} old={}",
            trace.tt_bestmove_current_generation, trace.tt_bestmove_old_generation
        ));
    }
    if trace.root_profiles.is_empty() {
        return messages;
    }
    for depth in &trace.root_profiles {
        messages.push(format!(
            "MESSAGE root_profile depth={} elapsed_ms={:.3} nodes={} candidates={} stopped={} score={}",
            depth.depth,
            depth.elapsed_us as f64 / 1000.0,
            depth.nodes,
            depth.candidates.len(),
            depth.stopped,
            depth.score,
        ));
        for candidate in &depth.candidates {
            let (x, y) = move_to_xy(candidate.move_).expect("profile move stays valid");
            messages.push(format!(
                "MESSAGE root_candidate depth={} index={} move={},{} score={} nodes={} elapsed_ms={:.3} alpha_before={} alpha_after={} beta={} reason={} zero_window_nodes={} zero_window_ms={:.3} full_window_nodes={} full_window_ms={:.3} pvs_research={}",
                depth.depth,
                candidate.index,
                x,
                y,
                candidate.score,
                candidate.nodes,
                candidate.elapsed_us as f64 / 1000.0,
                candidate.alpha_before,
                candidate.alpha_after,
                candidate.beta,
                candidate.reason,
                candidate.zero_window_nodes,
                candidate.zero_window_elapsed_us as f64 / 1000.0,
                candidate.full_window_nodes,
                candidate.full_window_elapsed_us as f64 / 1000.0,
                candidate.pvs_research,
            ));
        }
    }
    messages
}

fn parse_one<T: std::str::FromStr>(raw: &str) -> Option<T> {
    raw.trim().parse::<T>().ok()
}

fn parse_time_ms(raw: &str) -> Option<f64> {
    let parsed = parse_one::<f64>(raw)?;
    (parsed.is_finite() && parsed >= 0.0 && parsed <= MAX_PROTOCOL_TIME_MS).then_some(parsed)
}

fn is_oversized_time(raw: &str) -> bool {
    parse_one::<f64>(raw).is_some_and(|value| value.is_finite() && value > MAX_PROTOCOL_TIME_MS)
}

fn in_bounds_i32(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as usize) < BOARD_SIZE && (y as usize) < BOARD_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gomocup_auto_tt_tier_uses_effective_search_depth() {
        let mut config = load_default_config();
        config.runtime.threads = 4;

        let mut shallow = GomocupProtocol::new(
            Some(config.clone()),
            Some(SearchLimits {
                max_depth: 4,
                root_width: 15,
                ..SearchLimits::default()
            }),
        );
        assert_eq!(shallow.handle_line("START 15"), vec!["OK"]);
        let _ = shallow.handle_line("TURN 7,7");
        assert_eq!(
            shallow.searcher.as_ref().unwrap().tt.bucket_count(),
            1 << 20,
            "4T --depth 4 should auto-select 32 MiB TT (tt_bits = 20)"
        );

        let mut deep = GomocupProtocol::new(
            Some(config),
            Some(SearchLimits {
                max_depth: 5,
                root_width: 10,
                ..SearchLimits::default()
            }),
        );
        assert_eq!(deep.handle_line("START 15"), vec!["OK"]);
        let _ = deep.handle_line("TURN 7,7");
        assert_eq!(
            deep.searcher.as_ref().unwrap().tt.bucket_count(),
            1 << 21,
            "4T --depth 5 should auto-select 64 MiB TT (tt_bits = 21)"
        );
    }

    #[test]
    fn gomocup_rebuilds_auto_tt_only_when_max_node_changes_tier() {
        let mut config = load_default_config();
        config.runtime.threads = 4;

        let mut protocol = GomocupProtocol::new(
            Some(config.clone()),
            Some(SearchLimits {
                max_depth: 5,
                root_width: 8,
                ..SearchLimits::default()
            }),
        );
        assert_eq!(protocol.handle_line("START 15"), vec!["OK"]);
        let _ = protocol.handle_line("TURN 7,7");
        let tt_initial = protocol.searcher.as_ref().unwrap().tt.clone();
        assert_eq!(tt_initial.bucket_count(), 1 << 21);

        // Enabling max_node switches effective search to single-threaded -> rebuilds to 32 MiB TT.
        protocol.handle_line("INFO max_node 200");
        let _ = protocol.handle_line("TURN 0,0");
        let tt_limited = protocol.searcher.as_ref().unwrap().tt.clone();
        assert_eq!(tt_limited.bucket_count(), 1 << 20);
        assert!(!tt_limited.is_shared_with(&tt_initial));

        // Keeping max_node enabled reuses the 32 MiB searcher without rebuilding.
        protocol.handle_line("INFO max_node 300");
        let _ = protocol.handle_line("TURN 0,1");
        assert!(protocol
            .searcher
            .as_ref()
            .unwrap()
            .tt
            .is_shared_with(&tt_limited));

        // Clearing max_node restores 4T -> rebuilds back to 64 MiB TT.
        protocol.handle_line("INFO max_node 0");
        let _ = protocol.handle_line("TURN 0,2");
        assert_eq!(
            protocol.searcher.as_ref().unwrap().tt.bucket_count(),
            1 << 21
        );

        // Explicit tt_bits stays under user control and is never rebuilt by max_node changes.
        let mut explicit = GomocupProtocol::new(
            Some(config),
            Some(SearchLimits {
                max_depth: 5,
                root_width: 8,
                ..SearchLimits::default()
            }),
        );
        explicit.tt_bits = Some(19);
        assert_eq!(explicit.handle_line("START 15"), vec!["OK"]);
        let _ = explicit.handle_line("TURN 7,7");
        let tt_explicit = explicit.searcher.as_ref().unwrap().tt.clone();
        assert_eq!(tt_explicit.bucket_count(), 1 << 19);

        explicit.handle_line("INFO max_node 200");
        let _ = explicit.handle_line("TURN 0,0");
        assert!(explicit
            .searcher
            .as_ref()
            .unwrap()
            .tt
            .is_shared_with(&tt_explicit));
    }

    #[test]
    fn gomocup_info_nnue_dir_preserves_spaces_and_quotes() {
        let mut protocol = GomocupProtocol::default();
        assert!(!protocol.config.runtime.use_nnue);
        assert_eq!(protocol.config.runtime.nnue_dir, None);

        protocol.handle_line("INFO nnue_dir /tmp/my weights/mix9svq dir");
        assert!(protocol.config.runtime.use_nnue);
        assert_eq!(
            protocol.config.runtime.nnue_dir.as_deref(),
            Some("/tmp/my weights/mix9svq dir")
        );

        protocol.handle_line("INFO nnue_dir \"C:\\Program Files\\Gomoku\\mix9svq\"");
        assert!(protocol.config.runtime.use_nnue);
        assert_eq!(
            protocol.config.runtime.nnue_dir.as_deref(),
            Some("C:\\Program Files\\Gomoku\\mix9svq")
        );

        protocol.handle_line("INFO nnue_dir");
        assert!(!protocol.config.runtime.use_nnue);
        assert_eq!(protocol.config.runtime.nnue_dir, None);
    }

    #[test]
    fn gomocup_match_time_control_paces_time_left_without_altering_pure_turn_limit() {
        let mut turn_only = GomocupProtocol::default();
        turn_only.handle_line("INFO timeout_turn 1000");
        assert_eq!(
            turn_only.current_search_limits().time_limit_ms,
            Some(1000.0)
        );

        // Missing weights with use_nnue=true must fall back to classical match-clock
        // behavior instead of pacing as if NNUE were active.
        let mut missing_nnue = GomocupProtocol::default();
        missing_nnue.config.runtime.use_nnue = true;
        missing_nnue.config.runtime.nnue_dir = Some("nonexistent/weights".to_string());
        missing_nnue.handle_line("INFO timeout_match 60000");
        assert_eq!(
            missing_nnue.current_search_limits().time_limit_ms,
            Some(60000.0)
        );

        let local_weights = std::path::Path::new("opponent/rapfi-networks/mix9svq");
        if local_weights.exists() {
            let mut match_only = GomocupProtocol::default();
            match_only.handle_line(&format!("INFO nnue_dir {}", local_weights.display()));
            match_only.handle_line("START 15");
            match_only.handle_line("INFO timeout_match 60000");
            let slice = match_only
                .current_search_limits()
                .time_limit_ms
                .expect("match budget computed");
            assert!(
                (1000.0..=2500.0).contains(&slice),
                "60s match clock should pace opening move around 1.5s, got {slice}"
            );
        }

        let mut unlimited_match = GomocupProtocol::default();
        unlimited_match.handle_line("INFO timeout_turn 1000");
        unlimited_match.handle_line("INFO timeout_match 0");
        assert_eq!(
            unlimited_match.current_search_limits().time_limit_ms,
            Some(1000.0)
        );
    }

    #[test]
    fn gomocup_info_max_memory_caps_auto_tt_bits_for_4t_and_resets_on_zero() {
        let mut config = load_default_config();
        config.runtime.threads = 4;
        config.runtime.compute_vcf = false;
        config.runtime.compute_vct = false;
        let mut limits = SearchLimits::fixed_from_config(&config);
        limits.max_depth = 5;
        let mut protocol = GomocupProtocol::new(Some(config), Some(limits));
        protocol.handle_line("START 15");
        protocol.handle_line("INFO max_memory 33554432");
        assert_eq!(protocol.max_memory_tt_bits, Some(20));
        let _ = protocol.handle_line("BEGIN");
        let searcher = protocol.searcher.as_ref().expect("searcher built");
        assert_eq!(
            searcher.tt.bucket_count(),
            1_usize << 20,
            "INFO max_memory 33554432 must cap 4T TT at 32 MiB (2^20 buckets)"
        );

        // Clearing max_memory with 0 restores default 4T 64 MiB (2^21 buckets).
        protocol.handle_line("INFO max_memory 0");
        assert_eq!(protocol.max_memory_tt_bits, None);
        assert!(protocol.searcher.is_none());

        // Explicit tt_bits continues to override max_memory auto cap.
        protocol.handle_line("INFO max_memory 33554432");
        protocol.handle_line("INFO tt_bits 18");
        let _ = protocol.handle_line("RESTART");
        let _ = protocol.handle_line("BEGIN");
        let searcher = protocol.searcher.as_ref().expect("searcher rebuilt");
        assert_eq!(searcher.tt.bucket_count(), 1_usize << 18);
    }
}
