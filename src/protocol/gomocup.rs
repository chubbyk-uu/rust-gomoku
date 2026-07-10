//! Gomocup protocol adapter aligned with the reference implementation.

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::{apply_engine_profile, load_default_config, EngineConfig, EngineProfile};
use crate::constants::{BOARD_AREA, BOARD_SIZE};
use crate::rules::RuleSet;
use crate::search::{RootSearcher, SearchLimits, TranspositionTable, MAX_TT_BUCKET_BITS};

const MAX_PROTOCOL_TIME_MS: f64 = i32::MAX as f64;

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
    pending_error: Option<String>,
    searcher: Option<RootSearcher>,
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
                self.handle_info(&parts[1..]);
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

    fn handle_info(&mut self, args: &[&str]) {
        if args.len() < 2 {
            return;
        }
        let key = args[0].to_ascii_lowercase();
        let value = args[1];
        match key.as_str() {
            "timeout_turn" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.timeout_turn_ms = Some(if parsed == 0.0 { 200.0 } else { parsed });
                } else if is_oversized_time(value) {
                    self.pending_error = Some("time limit is too large".to_string());
                }
            }
            "timeout_match" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.time_left_ms = Some(if parsed == 0.0 { 99_999_999.0 } else { parsed });
                } else if is_oversized_time(value) {
                    self.pending_error = Some("time limit is too large".to_string());
                }
            }
            "time_left" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.time_left_ms = Some(parsed);
                } else if is_oversized_time(value) {
                    self.pending_error = Some("time limit is too large".to_string());
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
                    self.config.runtime.root_vcf_depth = parsed.max(0);
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
                    self.config.runtime.compute_vct = parsed != 0;
                    self.searcher = None;
                }
            }
            "root_vct_depth" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.root_vct_depth = parsed.max(0);
                    self.searcher = None;
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
            "fast_history_ordering" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.fast_history_ordering = parsed != 0;
                    self.searcher = None;
                }
            }
            "tt_bits" => {
                if let Some(parsed) = parse_one::<u32>(value) {
                    if parsed <= MAX_TT_BUCKET_BITS {
                        self.tt_bits = Some(parsed);
                        self.searcher = None;
                    } else {
                        self.pending_error = Some(format!(
                            "tt_bits must be between 0 and {MAX_TT_BUCKET_BITS}"
                        ));
                    }
                }
            }
            "profile" => {
                if let Ok(parsed) = value.parse::<EngineProfile>() {
                    apply_engine_profile(&mut self.config, parsed);
                    self.searcher = None;
                }
            }
            "rule" => {
                if self.board.move_count() == 0 {
                    if let Ok(parsed) = value.parse::<RuleSet>() {
                        if self.config.rule_set != parsed {
                            self.config.rule_set = parsed;
                            self.searcher = None;
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

    pub fn current_search_limits(&self) -> SearchLimits {
        if let Some(default_limits) = self.default_limits {
            SearchLimits {
                max_depth: default_limits.max_depth,
                root_width: default_limits.root_width,
                node_limit: self.node_limit.or(default_limits.node_limit),
                time_limit_ms: self
                    .timeout_turn_ms
                    .or(self.time_left_ms)
                    .or(default_limits.time_limit_ms),
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
                time_limit_ms: self.timeout_turn_ms.or(self.time_left_ms),
            }
        }
    }

    fn search_move(&mut self) -> Vec<String> {
        if let Some(message) = self.pending_error.take() {
            return vec![format!("ERROR {message}.")];
        }
        if self.board.is_draw() {
            return vec!["ERROR Board is full.".to_string()];
        }
        if self.board.winner() != 0 {
            return vec!["ERROR Board is terminal.".to_string()];
        }
        let limits = self.current_search_limits();
        let mut searcher = match self.searcher.take() {
            Some(searcher) => searcher,
            None => match self.tt_bits {
                Some(bits) => match TranspositionTable::try_new(bits) {
                    Ok(tt) => RootSearcher::with_tt(self.config.clone(), tt),
                    Err(_) => {
                        return vec!["ERROR Unable to allocate transposition table.".to_string()]
                    }
                },
                None => RootSearcher::new(self.config.clone()),
            },
        };
        searcher.config = self.config.clone();
        let result = searcher
            .try_search(&mut self.board, Some(limits))
            .expect("protocol checked the board is non-terminal");
        let trace = searcher.last_trace.clone();
        self.searcher = Some(searcher);

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
