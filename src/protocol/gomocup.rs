//! Gomocup protocol adapter aligned with the reference implementation.

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::{load_default_config, EngineConfig};
use crate::constants::{BOARD_AREA, BOARD_SIZE};
use crate::search::{RootSearcher, SearchLimits};

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
    pub ended: bool,
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
            ended: false,
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
            "TAKEBACK" => {
                if self.board.move_count() > 0 {
                    let _ = self.board.undo();
                }
                vec!["OK".to_string()]
            }
            "ABOUT" => vec![ABOUT_TEXT.to_string()],
            "END" => {
                self.ended = true;
                Vec::new()
            }
            _ => Vec::new(),
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
    }

    fn play_xy(&mut self, x: usize, y: usize, side: i8) -> Result<(), ()> {
        let move_ = xy_to_move(x, y).map_err(|_| ())?;
        self.board.force_side_to_move(side).map_err(|_| ())?;
        self.board.play(move_, Some(side)).map_err(|_| ())?;
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
                }
            }
            "timeout_match" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.time_left_ms = Some(if parsed == 0.0 { 99_999_999.0 } else { parsed });
                }
            }
            "time_left" => {
                if let Some(parsed) = parse_time_ms(value) {
                    self.time_left_ms = Some(parsed);
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
            "lazy_smp" => {
                if let Some(parsed) = parse_one::<i32>(value) {
                    self.config.runtime.lazy_smp = parsed != 0;
                    self.searcher = None;
                }
            }
            "lazy_smp_workers" => {
                if let Some(parsed) = parse_one::<i64>(value) {
                    self.config.runtime.lazy_smp_workers = parsed.max(0) as usize;
                    self.searcher = None;
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
        let limits = self.current_search_limits();
        let mut searcher = self
            .searcher
            .take()
            .unwrap_or_else(|| RootSearcher::new(self.config.clone()));
        searcher.config = self.config.clone();
        let result = searcher.search(&mut self.board, Some(limits));
        let trace = searcher.last_trace.clone();
        self.searcher = Some(searcher);

        let mut move_ = result.move_;
        if !self.board.is_legal_move(move_) {
            move_ = (0..BOARD_AREA as u16)
                .find(|&candidate| self.board.is_legal_move(candidate))
                .expect("engine produced no legal move on non-terminal board");
        }
        let (x, y) = move_to_xy(move_).expect("selected move stays valid");
        let side = self.board.side_to_move();
        self.play_xy(x, y, side)
            .expect("selected protocol move is legal");
        let mut responses = root_profile_messages(trace.as_ref());
        responses.push(format!("{x},{y}"));
        responses
    }
}

fn root_profile_messages(trace: Option<&crate::search::RootTrace>) -> Vec<String> {
    let Some(trace) = trace else {
        return Vec::new();
    };
    if trace.root_profiles.is_empty() {
        return Vec::new();
    }
    let mut messages = Vec::new();
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
                "MESSAGE root_candidate depth={} index={} move={},{} score={} nodes={} elapsed_ms={:.3} alpha_before={} alpha_after={} beta={} reason={}",
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
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

fn in_bounds_i32(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as usize) < BOARD_SIZE && (y as usize) < BOARD_SIZE
}
