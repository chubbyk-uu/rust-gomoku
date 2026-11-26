//! Shared application controller for desktop and mobile frontends.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use crate::nnue::mix9svq_files_exist_for_rule;
use crate::search::abdada::{SearchWorkerPool, ABDADA_PARALLEL_START_DEPTH};
use crate::{
    move_to_xy, xy_to_move, Board, EngineConfig, ForbiddenKind, RootSearcher, RootTrace, RuleSet,
    SearchLimits, SearchResult, Side, TranspositionTable, BLACK, BOARD_SIZE,
    DEFAULT_ROOT_VCT_DEPTH, EMPTY, MAX_SEARCH_THREADS, WHITE,
};

pub const DEFAULT_TIME_LIMIT_MS: u32 = 1000;
pub const SUPPORTED_TIME_LIMITS_MS: &[u32] = &[500, 1000, 2500, 5000];
/// Default TT bucket bits for interactive desktop GUI & Android play: `20` (32 MiB).
pub const DEFAULT_INTERACTIVE_TT_BITS: u32 = 20;

/// Whether the AI plays in Casual fixed-depth mode (classical eval) or Pro
/// timed mode (NNUE when weights are available).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchClockMode {
    /// Casual fixed-depth difficulty presets (Beginner..Master, classical eval).
    Untimed,
    /// Pro per-move time cap for the AI (NNUE timed mode, unbounded up to `timed_max_depth`).
    #[default]
    Timed,
}

impl SearchClockMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Untimed => "untimed",
            Self::Timed => "timed",
        }
    }
}

impl std::str::FromStr for SearchClockMode {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.to_ascii_lowercase().as_str() {
            "untimed" | "fixed" | "casual" => Ok(Self::Untimed),
            "timed" | "pro" => Ok(Self::Timed),
            _ => Err(format!("unknown clock mode: {raw}")),
        }
    }
}

/// Who controls the two sides of the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    /// One human side, the engine plays the other.
    #[default]
    VsEngine,
    /// Two humans alternate on the same device; the engine never moves.
    TwoPlayer,
}

impl GameMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GameMode::VsEngine => "vs_engine",
            GameMode::TwoPlayer => "two_player",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchDifficulty {
    /// 入门: classical `d2 / w10`, VCF off, VCT off.
    Beginner,
    /// 普通: classical `d4 / w20`, VCF off, VCT off.
    #[default]
    Normal,
    /// 高级: classical `d6 / w30`, VCF on, VCT off.
    Senior,
    /// 大师: classical `d8 / w40`, VCF on, VCT on (`d6`).
    Master,
}

impl SearchDifficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Normal => "normal",
            Self::Senior => "senior",
            Self::Master => "master",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Beginner => "入门",
            Self::Normal => "普通",
            Self::Senior => "高级",
            Self::Master => "大师",
        }
    }

    /// `(depth, width, compute_vcf, compute_vct, root_vct_depth)`.
    fn settings(self) -> (i32, i32, bool, bool, i32) {
        match self {
            Self::Beginner => (2, 10, false, false, 6),
            Self::Normal => (4, 20, false, false, 6),
            Self::Senior => (6, 30, true, false, 6),
            Self::Master => (8, 40, true, true, 6),
        }
    }

    pub fn apply_to_config(self, config: &mut EngineConfig) {
        let (depth, width, compute_vcf, compute_vct, root_vct_depth) = self.settings();
        config.root_search.depth = depth;
        config.root_search.wide = width;
        config.runtime.compute_vcf = compute_vcf;
        config.runtime.compute_vct = compute_vct;
        config.runtime.root_vct_depth = root_vct_depth;
    }

    fn detect(config: &EngineConfig) -> Option<Self> {
        [Self::Beginner, Self::Normal, Self::Senior, Self::Master]
            .into_iter()
            .find(|difficulty| {
                let (depth, width, compute_vcf, compute_vct, root_vct_depth) =
                    difficulty.settings();
                config.root_search.depth == depth
                    && config.root_search.wide == width
                    && config.runtime.compute_vcf == compute_vcf
                    && config.runtime.compute_vct == compute_vct
                    && config.runtime.root_vct_depth == root_vct_depth
            })
    }
}

impl std::str::FromStr for SearchDifficulty {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.to_ascii_lowercase().as_str() {
            "beginner" | "intro" | "junior" | "easy" => Ok(Self::Beginner),
            "normal" | "intermediate" | "medium" => Ok(Self::Normal),
            "senior" | "advanced" => Ok(Self::Senior),
            "master" | "hard" => Ok(Self::Master),
            _ => Err(format!("unknown search difficulty: {raw}")),
        }
    }
}

#[derive(Clone)]
pub struct GameController {
    config: EngineConfig,
    board: Board,
    human_side: Side,
    mode: GameMode,
    engine_thinking: bool,
    status: String,
    error: Option<String>,
    last_mark: Option<(usize, usize)>,
    last_result: Option<(SearchResult, bool)>,
    last_trace: Option<RootTrace>,
    last_search_ms: Option<f64>,
    nnue_history: Option<Vec<NnuePositionEstimate>>,
    difficulty: Option<SearchDifficulty>,
    clock_mode: SearchClockMode,
    time_limit_ms: u32,
    generation: u64,
    search_tt: Option<TranspositionTable>,
    fast_searcher: Option<Arc<crate::fast_engine::FastSearcher>>,
    search_pool: Arc<Mutex<Option<SearchWorkerPool>>>,
}

#[derive(Clone)]
pub struct EngineSearchTask {
    board: Board,
    config: EngineConfig,
    limits: SearchLimits,
    generation: u64,
    tt: Option<TranspositionTable>,
    fast_searcher: Option<Arc<crate::fast_engine::FastSearcher>>,
    worker_pool: Arc<Mutex<Option<SearchWorkerPool>>>,
    record_nnue_history: bool,
}

enum EngineSearchOutcome {
    Completed {
        result: SearchResult,
        trace: Option<RootTrace>,
        elapsed_ms: f64,
        used_nnue: bool,
        estimate: Option<NnuePositionEstimate>,
        tt: Option<TranspositionTable>,
        fast_searcher: Option<Arc<crate::fast_engine::FastSearcher>>,
        worker_pool: Option<SearchWorkerPool>,
    },
    Failed {
        error: String,
    },
}

pub struct EngineSearchCompletion {
    generation: u64,
    outcome: EngineSearchOutcome,
}

impl EngineSearchCompletion {
    pub fn failed(generation: u64, error: impl Into<String>) -> Self {
        Self {
            generation,
            outcome: EngineSearchOutcome::Failed {
                error: error.into(),
            },
        }
    }

    #[cfg(test)]
    fn tt(&self) -> Option<&TranspositionTable> {
        match &self.outcome {
            EngineSearchOutcome::Completed { tt, .. } => tt.as_ref(),
            EngineSearchOutcome::Failed { .. } => None,
        }
    }

    #[cfg(test)]
    fn has_worker_pool(&self) -> bool {
        matches!(
            &self.outcome,
            EngineSearchOutcome::Completed {
                worker_pool: Some(_),
                ..
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GameStateSnapshot {
    pub board_size: usize,
    pub cells: Vec<i8>,
    pub moves: Vec<GameMoveSnapshot>,
    pub forbidden_points: Vec<ForbiddenPointSnapshot>,
    pub human_side: i8,
    pub side_to_move: i8,
    pub winner: i8,
    pub draw: bool,
    pub move_count: usize,
    pub can_play: bool,
    pub engine_thinking: bool,
    pub status: String,
    pub error: Option<String>,
    pub last_mark: Option<[usize; 2]>,
    pub last_result: Option<SearchResultSnapshot>,
    pub last_trace: Option<SearchTraceSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nnue_history: Option<Vec<NnuePositionEstimate>>,
    pub params: GameParamsSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GameMoveSnapshot {
    pub x: usize,
    pub y: usize,
    pub side: i8,
    pub number: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ForbiddenPointSnapshot {
    pub x: usize,
    pub y: usize,
    pub kind: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchResultSnapshot {
    #[serde(rename = "move_xy")]
    pub move_xy: [usize; 2],
    pub score: i32,
    pub depth: i32,
    pub nodes: usize,
    pub ms: Option<f64>,
    pub used_nnue: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct NnuePositionEstimate {
    pub ply: usize,
    pub black_win: f32,
    pub draw: f32,
    pub white_win: f32,
}

impl NnuePositionEstimate {
    fn from_logits(board: &Board, (win, loss, draw): (f32, f32, f32)) -> Self {
        let max = win.max(loss).max(draw);
        let win = (win - max).exp();
        let loss = (loss - max).exp();
        let draw = (draw - max).exp();
        let total = win + loss + draw;
        let (black, white) = if board.side_to_move() == BLACK {
            (win, loss)
        } else {
            (loss, win)
        };
        Self {
            ply: board.move_count(),
            black_win: black / total,
            draw: draw / total,
            white_win: white / total,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchTraceSnapshot {
    pub used_vcf: bool,
    pub vcf_found: bool,
    pub used_vct: bool,
    pub vct_triggered: bool,
    pub vct_ms: Option<f64>,
    pub vct_found: bool,
    pub vct_accepted: bool,
    pub vct_reject_reason: Option<&'static str>,
    pub alphabeta_ms: Option<f64>,
    pub overlap_used: bool,
    pub overlap_ab_ms: Option<f64>,
    pub overlap_ab_cancelled: bool,
    pub overlap_wait_ms: Option<f64>,
    pub tt_snapshot_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GameParamsSnapshot {
    pub mode: &'static str,
    pub clock_mode: &'static str,
    pub time_limit_ms: u32,
    pub difficulty: &'static str,
    pub use_nnue: bool,
    pub nnue_available: bool,
    pub depth: i32,
    pub width: usize,
    pub threads: usize,
    pub compute_vcf: bool,
    pub root_vcf_depth: i32,
    pub opponent_vcf_depth: i32,
    pub compute_vct: bool,
    pub root_vct_depth: i32,
    pub overlap_vct_alphabeta: bool,
    pub static_board: bool,
    pub dynamic_board_margin: i32,
    pub rule: &'static str,
}

impl GameController {
    pub fn new(config: EngineConfig) -> Self {
        Self::new_with_clock(config, SearchClockMode::default())
    }

    pub fn new_with_clock(config: EngineConfig, clock_mode: SearchClockMode) -> Self {
        let difficulty = SearchDifficulty::detect(&config);
        Self {
            config,
            board: Board::new(),
            human_side: BLACK,
            mode: GameMode::VsEngine,
            engine_thinking: false,
            status: "请选择执黑或执白，然后开始对局。".to_string(),
            error: None,
            last_mark: None,
            last_result: None,
            last_trace: None,
            last_search_ms: None,
            nnue_history: None,
            difficulty,
            clock_mode,
            time_limit_ms: DEFAULT_TIME_LIMIT_MS,
            generation: 0,
            search_tt: None,
            fast_searcher: None,
            search_pool: Arc::new(Mutex::new(None)),
        }
    }

    /// Record post-move NNUE estimates for frontends that display a game chart.
    pub fn with_nnue_history(mut self) -> Self {
        self.nnue_history.get_or_insert_with(Vec::new);
        self
    }

    pub fn set_nnue_dir(&mut self, nnue_dir: Option<String>) {
        if self.engine_thinking {
            return;
        }
        let normalized = nnue_dir.filter(|s| !s.trim().is_empty());
        if self.config.runtime.nnue_dir != normalized {
            self.config.runtime.nnue_dir = normalized;
            self.search_tt = None;
            if let Some(fs) = &self.fast_searcher {
                fs.clear_tt();
            }
            if let Some(history) = &mut self.nnue_history {
                history.clear();
            }
        }
    }

    fn nnue_available_for(&self, rule: RuleSet) -> bool {
        self.config
            .runtime
            .nnue_dir
            .as_deref()
            .is_some_and(|dir| mix9svq_files_exist_for_rule(Path::new(dir), rule))
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn new_game(&mut self, human_side: Side, rule: RuleSet, mode: GameMode) -> bool {
        self.new_game_with_clock(human_side, rule, mode, self.clock_mode)
    }

    pub fn new_game_with_clock(
        &mut self,
        human_side: Side,
        rule: RuleSet,
        mode: GameMode,
        clock_mode: SearchClockMode,
    ) -> bool {
        if human_side != BLACK && human_side != WHITE {
            self.error = Some("执棋方必须是黑方或白方。".to_string());
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.board.reset();
        self.config.rule_set = rule;
        self.human_side = human_side;
        self.mode = mode;
        self.clock_mode = clock_mode;
        self.engine_thinking = false;
        let rule_name = match rule {
            RuleSet::Freestyle => "无禁手",
            RuleSet::Renju => "有禁手",
        };
        let clock_label = match clock_mode {
            SearchClockMode::Untimed => {
                let diff = self
                    .difficulty
                    .map(SearchDifficulty::label)
                    .unwrap_or("自定义");
                format!(" / 休闲·{diff}")
            }
            SearchClockMode::Timed => {
                let engine_tag = if self.nnue_available_for(rule) {
                    "专业"
                } else {
                    "专业(经典)"
                };
                format!(
                    " / {engine_tag} {}",
                    format_time_limit_label(self.time_limit_ms)
                )
            }
        };
        self.status = match mode {
            GameMode::TwoPlayer => format!("新局开始（双人 / {rule_name}）：黑方先行。"),
            GameMode::VsEngine if human_side == BLACK => {
                format!("新局开始（{rule_name}{clock_label}）：你执黑，请落子。")
            }
            GameMode::VsEngine => {
                format!("新局开始（{rule_name}{clock_label}）：你执白，引擎执黑思考中。")
            }
        };
        self.error = None;
        self.last_mark = None;
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
        if let Some(history) = &mut self.nnue_history {
            history.clear();
        }
        self.search_tt = None;
        if let Some(fs) = &self.fast_searcher {
            fs.clear_tt();
        }
        if self.search_limits().max_depth < ABDADA_PARALLEL_START_DEPTH {
            if let Ok(mut guard) = self.search_pool.lock() {
                *guard = None;
            }
        }
        true
    }

    pub fn play_human(&mut self, x: usize, y: usize) -> bool {
        self.error = None;
        if !self.can_human_play() {
            self.error = Some("现在不能落子。".to_string());
            return false;
        }

        let Ok(move_) = xy_to_move(x, y) else {
            self.error = Some("缺少坐标或坐标超出棋盘。".to_string());
            return false;
        };
        // In two-player mode the side to move alternates, so judge forbidden
        // points against whoever is on turn; vs-engine judges the human side.
        let judged_side = match self.mode {
            GameMode::TwoPlayer => self.board.side_to_move(),
            GameMode::VsEngine => self.human_side,
        };
        let forbidden = self
            .board
            .forbidden_kind_for_rule(move_, judged_side, self.config.rule_set)
            .ok()
            .filter(|kind| kind.is_forbidden());
        if let Some(kind) = forbidden {
            self.error = Some(format!(
                "黑棋禁手：{}，不能落在这里。",
                forbidden_name(kind)
            ));
            return false;
        }

        match self.board.play_for_rule(move_, None, self.config.rule_set) {
            Ok(_) => {
                self.last_mark = Some((x, y));
                match self.mode {
                    GameMode::TwoPlayer => {
                        // The engine never moves; report the result / next turn
                        // and always return false so no engine search starts.
                        let winner = self.board.winner();
                        self.status = if winner == BLACK {
                            "黑方胜。".to_string()
                        } else if winner == WHITE {
                            "白方胜。".to_string()
                        } else if self.board.is_draw() {
                            "和棋。".to_string()
                        } else if self.board.side_to_move() == BLACK {
                            "轮到黑方落子。".to_string()
                        } else {
                            "轮到白方落子。".to_string()
                        };
                        false
                    }
                    GameMode::VsEngine => {
                        if self.board.winner() == self.human_side {
                            self.status = "你赢了。".to_string();
                            false
                        } else if self.board.is_draw() {
                            self.status = "和棋。".to_string();
                            false
                        } else {
                            self.status = "你已落子，引擎思考中。".to_string();
                            true
                        }
                    }
                }
            }
            Err(err) => {
                self.error = Some(format!("非法落子：{err:?}"));
                false
            }
        }
    }

    pub fn undo_turn(&mut self) {
        if self.engine_thinking {
            self.error = Some("引擎思考中，暂不能悔棋。".to_string());
            return;
        }
        self.error = None;
        if self.board.move_count() == 0 {
            self.error = Some("当前没有可悔棋步。".to_string());
            return;
        }

        self.generation = self.generation.wrapping_add(1);
        self.search_tt = None;
        if self.mode == GameMode::TwoPlayer {
            // Two humans alternate, so a single undo hands the turn back.
            let _ = self.board.undo();
            self.last_mark = last_move_xy(&self.board);
            self.last_result = None;
            self.last_trace = None;
            self.last_search_ms = None;
            self.status = if self.board.side_to_move() == BLACK {
                "已悔棋，轮到黑方落子。".to_string()
            } else {
                "已悔棋，轮到白方落子。".to_string()
            };
            return;
        }

        let mut undone = 0;
        while self.board.move_count() > 0 && undone < 2 {
            if self.board.undo().is_ok() {
                undone += 1;
            }
            if self.board.side_to_move() == self.human_side {
                break;
            }
        }
        if let Some(history) = &mut self.nnue_history {
            history.retain(|sample| sample.ply <= self.board.move_count());
        }
        self.last_mark = last_move_xy(&self.board);
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
        self.status = if self.board.side_to_move() == self.human_side {
            "已悔棋，请继续落子。".to_string()
        } else if self.board.move_count() == 0 && self.human_side == WHITE {
            "已撤回引擎首手；点击“我执白”可重新让引擎开局。".to_string()
        } else {
            "已悔棋，当前不是你的回合。".to_string()
        };
    }

    pub fn set_difficulty(&mut self, difficulty: SearchDifficulty) {
        if self.engine_thinking {
            self.error = Some("引擎思考中，暂不能切换难度。".to_string());
            return;
        }
        let prev_config = self.config.clone();
        let prev_difficulty = self.difficulty;
        let (depth, width, compute_vcf, compute_vct, _) = difficulty.settings();
        difficulty.apply_to_config(&mut self.config);
        self.difficulty = Some(difficulty);
        if prev_difficulty != self.difficulty || prev_config != self.config {
            self.search_tt = None;
            if self.clock_mode == SearchClockMode::Untimed && depth < ABDADA_PARALLEL_START_DEPTH {
                if let Ok(mut guard) = self.search_pool.lock() {
                    *guard = None;
                }
            }
        }
        self.error = None;
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
        let tactics = match (compute_vcf, compute_vct) {
            (false, false) => "无战术搜索",
            (true, false) => "VCF 开启",
            (true, true) => "VCF/VCT 开启",
            (false, true) => "VCT 开启",
        };
        self.status = format!(
            "已切换到休闲·{}难度（d{depth}/w{width}，{tactics}），下一次引擎思考生效。",
            difficulty.label(),
        );
    }

    pub fn set_time_limit_ms(&mut self, ms: u32) -> bool {
        if self.engine_thinking {
            self.error = Some("引擎思考中，暂不能切换每步限时。".to_string());
            return false;
        }
        if !SUPPORTED_TIME_LIMITS_MS.contains(&ms) {
            self.error = Some("不支持的限时档位，请选择 500、1000、2500 或 5000 ms。".to_string());
            return false;
        }
        self.time_limit_ms = ms;
        self.error = None;
        self.status = format!(
            "已切换专业限时为 {}，下一次引擎思考生效。",
            format_time_limit_label(ms)
        );
        true
    }

    pub fn set_threads(&mut self, threads: usize) {
        if self.engine_thinking {
            self.error = Some("引擎思考中，暂不能切换线程数。".to_string());
            return;
        }
        let threads = threads.clamp(1, MAX_SEARCH_THREADS);
        let prev_threads = self.config.runtime.threads;
        if prev_threads != threads {
            if (prev_threads > 1) != (threads > 1) {
                self.search_tt = None;
            }
            self.config.runtime.threads = threads;
            if threads == 1 {
                if let Ok(mut guard) = self.search_pool.lock() {
                    *guard = None;
                }
            }
        }
        self.error = None;
        self.status = format!("已切换为 {threads} 线程，下一次引擎思考生效。");
    }

    pub fn set_overlap_vct_alphabeta(&mut self, enabled: bool) {
        if self.engine_thinking {
            return;
        }
        self.config.runtime.overlap_vct_alphabeta = enabled;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub fn prepare_engine_search(&mut self) -> Option<EngineSearchTask> {
        if self.mode == GameMode::TwoPlayer
            || self.engine_thinking
            || self.board.is_terminal()
            || self.board.side_to_move() == self.human_side
        {
            return None;
        }
        self.engine_thinking = true;
        self.error = None;
        self.status = "引擎思考中...".to_string();
        let taken_pool = self
            .search_pool
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        Some(EngineSearchTask {
            board: self.board.clone(),
            config: self.effective_config(),
            limits: self.search_limits(),
            generation: self.generation,
            tt: self.search_tt.clone(),
            fast_searcher: self.fast_searcher.clone(),
            worker_pool: Arc::new(Mutex::new(taken_pool)),
            record_nnue_history: self.nnue_history.is_some(),
        })
    }

    pub fn commit_engine_search(&mut self, completion: EngineSearchCompletion) -> bool {
        if self.generation != completion.generation {
            return false;
        }
        let (result, trace, elapsed_ms, used_nnue, estimate, tt, fast_searcher, worker_pool) =
            match completion.outcome {
                EngineSearchOutcome::Completed {
                    result,
                    trace,
                    elapsed_ms,
                    used_nnue,
                    estimate,
                    tt,
                    fast_searcher,
                    worker_pool,
                } => (
                    result,
                    trace,
                    elapsed_ms,
                    used_nnue,
                    estimate,
                    tt,
                    fast_searcher,
                    worker_pool,
                ),
                EngineSearchOutcome::Failed { error } => {
                    self.error = Some(error);
                    self.status = "引擎搜索失败。".to_string();
                    self.engine_thinking = false;
                    return true;
                }
            };
        if let Some(tt) = tt {
            self.search_tt = Some(tt);
        }
        if let Some(fs) = fast_searcher {
            self.fast_searcher = Some(fs);
        }
        if self.config.runtime.threads > 1
            && self.search_limits().max_depth >= ABDADA_PARALLEL_START_DEPTH
        {
            if let Some(pool) = worker_pool {
                let mut guard = self
                    .search_pool
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard = Some(pool);
            }
        }
        if self.board.winner() == EMPTY && self.board.side_to_move() != self.human_side {
            if !self.board.is_legal_move_for_rule(
                result.move_,
                self.board.side_to_move(),
                self.config.rule_set,
            ) {
                self.error = Some("引擎返回禁手或非法落子。".to_string());
                self.status = "引擎落子失败。".to_string();
            } else {
                match self
                    .board
                    .play_for_rule(result.move_, None, self.config.rule_set)
                {
                    Ok(_) => {
                        self.last_mark = move_to_xy(result.move_).ok();
                        self.last_result = Some((result, used_nnue));
                        self.last_trace = trace;
                        self.last_search_ms = Some((elapsed_ms * 1000.0).round() / 1000.0);
                        if let (Some(history), Some(sample)) = (&mut self.nnue_history, estimate) {
                            history.push(sample);
                        }
                        self.status = if self.board.winner() != EMPTY {
                            "引擎获胜。".to_string()
                        } else if self.board.is_draw() {
                            "和棋。".to_string()
                        } else {
                            "引擎已落子，轮到你。".to_string()
                        };
                    }
                    Err(err) => {
                        self.error = Some(format!("引擎返回非法落子：{err:?}"));
                        self.status = "引擎落子失败。".to_string();
                    }
                }
            }
        }
        self.engine_thinking = false;
        true
    }

    pub fn snapshot(&self) -> GameStateSnapshot {
        let last_result = self.last_result.map(|(result, used_nnue)| {
            let (x, y) = move_to_xy(result.move_).expect("engine move stays valid");
            SearchResultSnapshot {
                move_xy: [x, y],
                score: result.score,
                depth: result.depth,
                nodes: result.nodes,
                ms: self.last_search_ms,
                used_nnue,
            }
        });
        let last_trace = self.last_trace.as_ref().map(|trace| SearchTraceSnapshot {
            used_vcf: trace.used_vcf,
            vcf_found: trace.vcf_found,
            used_vct: trace.used_vct,
            vct_triggered: trace.vct_triggered,
            vct_ms: trace.vct_ms,
            vct_found: trace.vct_found,
            vct_accepted: trace.vct_accepted,
            vct_reject_reason: trace.vct_reject_reason,
            alphabeta_ms: trace.alphabeta_ms,
            overlap_used: trace.overlap_used,
            overlap_ab_ms: trace.overlap_ab_ms,
            overlap_ab_cancelled: trace.overlap_ab_cancelled,
            overlap_wait_ms: trace.overlap_wait_ms,
            tt_snapshot_ms: trace.tt_snapshot_ms,
        });
        let effective = self.effective_config();
        let limits = self.search_limits();
        let nnue_available = self.nnue_available_for(effective.rule_set);
        let nnue_active = effective.runtime.use_nnue && nnue_available;
        GameStateSnapshot {
            board_size: BOARD_SIZE,
            cells: self.board_cells(),
            moves: self
                .board
                .move_history()
                .iter()
                .enumerate()
                .filter_map(|(index, played)| {
                    let (x, y) = move_to_xy(played.move_).ok()?;
                    Some(GameMoveSnapshot {
                        x,
                        y,
                        side: played.side,
                        number: index + 1,
                    })
                })
                .collect(),
            forbidden_points: self.visible_forbidden_points(),
            human_side: self.human_side,
            side_to_move: self.board.side_to_move(),
            winner: self.board.winner(),
            draw: self.board.is_draw(),
            move_count: self.board.move_count(),
            can_play: self.can_human_play(),
            engine_thinking: self.engine_thinking,
            status: self.status.clone(),
            error: self.error.clone(),
            last_mark: self.last_mark.map(|(x, y)| [x, y]),
            last_result,
            last_trace,
            nnue_history: self.nnue_history.clone(),
            params: GameParamsSnapshot {
                mode: self.mode.as_str(),
                clock_mode: self.clock_mode.as_str(),
                time_limit_ms: self.time_limit_ms,
                difficulty: self
                    .difficulty
                    .map(SearchDifficulty::as_str)
                    .unwrap_or("custom"),
                use_nnue: nnue_active,
                nnue_available,
                depth: limits.max_depth,
                width: limits.root_width,
                threads: effective.runtime.threads,
                compute_vcf: effective.runtime.compute_vcf,
                root_vcf_depth: effective.runtime.effective_root_vcf_depth(nnue_active),
                opponent_vcf_depth: effective.runtime.opponent_vcf_depth,
                compute_vct: effective.runtime.effective_compute_vct(nnue_active),
                root_vct_depth: effective.runtime.root_vct_depth,
                overlap_vct_alphabeta: effective.runtime.overlap_vct_alphabeta,
                static_board: effective.runtime.static_board,
                dynamic_board_margin: effective.runtime.dynamic_board_margin,
                rule: effective.rule_set.as_str(),
            },
        }
    }

    fn effective_config(&self) -> EngineConfig {
        let mut config = self.config.clone();
        match self.clock_mode {
            SearchClockMode::Timed => {
                config.runtime.compute_vcf = true;
                config.runtime.compute_vct = true;
                config.runtime.root_vct_depth = DEFAULT_ROOT_VCT_DEPTH;
                config.runtime.use_nnue = self.nnue_available_for(config.rule_set);
            }
            SearchClockMode::Untimed => {
                // Casual fixed-depth presets always use the classical evaluator.
                config.runtime.use_nnue = false;
            }
        }
        config
    }

    fn board_cells(&self) -> Vec<i8> {
        let mut cells = Vec::with_capacity(BOARD_SIZE * BOARD_SIZE);
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                cells.push(self.board.at(x, y).expect("coordinates stay in range"));
            }
        }
        cells
    }

    fn can_human_play(&self) -> bool {
        if self.engine_thinking || self.board.is_terminal() {
            return false;
        }
        match self.mode {
            // Either side may be placed by a human on the same device.
            GameMode::TwoPlayer => true,
            GameMode::VsEngine => self.board.side_to_move() == self.human_side,
        }
    }

    fn search_limits(&self) -> SearchLimits {
        match self.clock_mode {
            SearchClockMode::Untimed => SearchLimits::fixed_from_config(&self.config),
            SearchClockMode::Timed => {
                let effective = self.effective_config();
                SearchLimits {
                    time_limit_ms: Some(f64::from(self.time_limit_ms)),
                    ..SearchLimits::timed_from_config(&effective)
                }
            }
        }
    }

    fn visible_forbidden_points(&self) -> Vec<ForbiddenPointSnapshot> {
        if self.config.rule_set != RuleSet::Renju
            || self.board.side_to_move() != BLACK
            || self.board.winner() != EMPTY
        {
            return Vec::new();
        }

        let mut points = Vec::new();
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                let Ok(move_) = xy_to_move(x, y) else {
                    continue;
                };
                let Ok(kind) = self
                    .board
                    .forbidden_kind_for_rule(move_, BLACK, RuleSet::Renju)
                else {
                    continue;
                };
                if kind.is_forbidden() {
                    points.push(ForbiddenPointSnapshot {
                        x,
                        y,
                        kind: forbidden_code(kind),
                    });
                }
            }
        }
        points
    }
}

impl EngineSearchTask {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn run(mut self) -> EngineSearchCompletion {
        let nnue = crate::search::alphabeta::load_nnue_for_config(&self.config);
        if let Some(mut evaluator) = nnue {
            let start = Instant::now();
            let rule = self.config.rule_set;
            evaluator.sync_with_board(&self.board);
            let fast_searcher = self.fast_searcher.unwrap_or_else(|| {
                let tt_kb = (1_usize << DEFAULT_INTERACTIVE_TT_BITS) * 32 / 1024;
                Arc::new(crate::fast_engine::FastSearcher::new(tt_kb))
            });
            let fb = crate::fast_engine::FastBoard::from_legacy_board(
                &self.board,
                rule,
                crate::fast_engine::CandRange::Square3Line4,
            );
            let cfg = crate::fast_engine::FastSearchConfig {
                max_depth: 99,
                max_nodes: self.limits.node_limit.map(|n| n as u64),
                time_limit_ms: self.limits.time_limit_ms.map(|ms| ms.max(10.0) as u64),
                time_left_ms: None,
                num_threads: self.config.runtime.threads.max(1),
                cand_range: crate::fast_engine::CandRange::Square3Line4,
            };
            let res = fast_searcher.search(&fb, &evaluator, &cfg);
            let mut move_ = res.best_move;
            if !self
                .board
                .is_legal_move_for_rule(move_, self.board.side_to_move(), rule)
            {
                let Some(fallback) = (0..crate::constants::BOARD_AREA as u16).find(|&c| {
                    self.board
                        .is_legal_move_for_rule(c, self.board.side_to_move(), rule)
                }) else {
                    return EngineSearchCompletion::failed(self.generation, "无合法落子。");
                };
                move_ = fallback;
            }
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let result = SearchResult {
                move_,
                score: res.score,
                depth: res.depth,
                nodes: res.nodes as usize,
            };
            let estimate = if self.record_nnue_history
                && self.board.play_for_rule(result.move_, None, rule).is_ok()
                && !self.board.is_terminal()
            {
                evaluator.sync_with_board(&self.board);
                let (_, logits, _) = evaluator.evaluate_value_detailed(self.board.side_to_move());
                Some(NnuePositionEstimate::from_logits(&self.board, logits))
            } else {
                None
            };
            return EngineSearchCompletion {
                generation: self.generation,
                outcome: EngineSearchOutcome::Completed {
                    result,
                    trace: None,
                    elapsed_ms,
                    used_nnue: true,
                    estimate,
                    tt: None,
                    fast_searcher: Some(fast_searcher),
                    worker_pool: None,
                },
            };
        }

        let tt_bits = DEFAULT_INTERACTIVE_TT_BITS;
        let tt = match self.tt {
            Some(tt) => tt,
            None => match TranspositionTable::try_new_with_fallback_layout(tt_bits, 16, false) {
                Ok((tt, _)) => tt,
                Err(_) => {
                    return EngineSearchCompletion::failed(
                        self.generation,
                        "内存不足，无法分配置换表。",
                    );
                }
            },
        };
        let mut searcher = RootSearcher::with_tt_and_nnue(self.config, tt, None);
        let reused_pool = self
            .worker_pool
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        searcher.set_worker_pool(reused_pool);
        let start = Instant::now();
        let result = match searcher.try_search(&mut self.board, Some(self.limits)) {
            Ok(result) => result,
            Err(err) => {
                return EngineSearchCompletion::failed(
                    self.generation,
                    format!("引擎搜索失败：{err:?}"),
                );
            }
        };
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let worker_pool = searcher.take_worker_pool();
        EngineSearchCompletion {
            generation: self.generation,
            outcome: EngineSearchOutcome::Completed {
                result,
                trace: searcher.last_trace,
                elapsed_ms,
                used_nnue: false,
                estimate: None,
                tt: Some(searcher.tt),
                fast_searcher: None,
                worker_pool,
            },
        }
    }
}

fn format_time_limit_label(ms: u32) -> String {
    if ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else if ms % 100 == 0 {
        format!("{}.{}s", ms / 1000, (ms % 1000) / 100)
    } else {
        format!("{ms}ms")
    }
}

fn forbidden_name(kind: ForbiddenKind) -> &'static str {
    match kind {
        ForbiddenKind::None => "无",
        ForbiddenKind::DoubleThree => "三三禁手",
        ForbiddenKind::DoubleFour => "四四禁手",
        ForbiddenKind::Overline => "长连禁手",
    }
}

fn forbidden_code(kind: ForbiddenKind) -> &'static str {
    match kind {
        ForbiddenKind::None => "none",
        ForbiddenKind::DoubleThree => "double_three",
        ForbiddenKind::DoubleFour => "double_four",
        ForbiddenKind::Overline => "overline",
    }
}

fn last_move_xy(board: &Board) -> Option<(usize, usize)> {
    board
        .move_history()
        .last()
        .and_then(|played| move_to_xy(played.move_).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_default_config;

    fn double_three_controller(rule: RuleSet) -> GameController {
        let mut config = load_default_config();
        config.rule_set = rule;
        let mut controller = GameController::new(config);
        for (x, y) in [
            (6, 7),
            (0, 0),
            (8, 7),
            (0, 1),
            (7, 6),
            (0, 2),
            (7, 8),
            (0, 3),
        ] {
            controller
                .board
                .play_for_rule(xy_to_move(x, y).unwrap(), None, rule)
                .unwrap();
        }
        controller
    }

    fn shallow_controller() -> GameController {
        let mut config = load_default_config();
        config.root_search.depth = 1;
        config.root_search.wide = 8;
        GameController::new_with_clock(config, SearchClockMode::Untimed)
    }

    #[test]
    fn renju_black_turn_exposes_and_rejects_forbidden_points() {
        let mut controller = double_three_controller(RuleSet::Renju);
        let snapshot = controller.snapshot();

        assert!(snapshot.forbidden_points.contains(&ForbiddenPointSnapshot {
            x: 7,
            y: 7,
            kind: "double_three",
        }));
        let move_count = controller.board.move_count();
        assert!(!controller.play_human(7, 7));
        assert_eq!(controller.board.move_count(), move_count);
        assert!(controller.snapshot().error.unwrap().contains("三三禁手"));
    }

    #[test]
    fn freestyle_and_white_turn_do_not_expose_forbidden_points() {
        assert!(double_three_controller(RuleSet::Freestyle)
            .snapshot()
            .forbidden_points
            .is_empty());

        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));
        assert!(controller.snapshot().forbidden_points.is_empty());
    }

    #[test]
    fn stale_search_completion_does_not_mutate_new_game() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(WHITE, RuleSet::Freestyle, GameMode::VsEngine));
        let task = controller.prepare_engine_search().unwrap();
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));

        assert!(!controller.commit_engine_search(task.run()));
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, 0);
        assert_eq!(snapshot.human_side, BLACK);
        assert_eq!(snapshot.params.rule, "renju");
        assert!(!snapshot.engine_thinking);
    }

    #[test]
    fn engine_search_round_trip_plays_center_for_white_human() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(WHITE, RuleSet::Renju, GameMode::VsEngine));
        let completion = controller.prepare_engine_search().unwrap().run();

        assert!(controller.commit_engine_search(completion));
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, 1);
        assert_eq!(snapshot.moves[0].x, BOARD_SIZE / 2);
        assert_eq!(snapshot.moves[0].y, BOARD_SIZE / 2);
        assert!(snapshot.can_play);
    }

    #[test]
    fn undo_keeps_controller_consistent() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));
        let completion = controller.prepare_engine_search().unwrap().run();
        assert!(controller.commit_engine_search(completion));
        controller.undo_turn();

        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, 0);
        assert!(snapshot.can_play);
    }

    #[test]
    fn difficulty_presets_update_search_and_tactical_settings() {
        let mut controller =
            GameController::new_with_clock(load_default_config(), SearchClockMode::Untimed);
        assert_eq!(controller.snapshot().params.difficulty, "master");

        for (difficulty, name, depth, width, compute_vcf, compute_vct, root_vct_depth) in [
            (
                SearchDifficulty::Beginner,
                "beginner",
                2,
                10,
                false,
                false,
                6,
            ),
            (SearchDifficulty::Normal, "normal", 4, 20, false, false, 6),
            (SearchDifficulty::Senior, "senior", 6, 30, true, false, 6),
            (SearchDifficulty::Master, "master", 8, 40, true, true, 6),
        ] {
            controller.set_difficulty(difficulty);
            let params = controller.snapshot().params;
            assert_eq!(params.difficulty, name);
            assert_eq!(params.depth, depth);
            assert_eq!(params.width, width);
            assert_eq!(params.compute_vcf, compute_vcf);
            assert_eq!(params.compute_vct, compute_vct);
            assert_eq!(params.root_vct_depth, root_vct_depth);
            assert!(!params.use_nnue);
        }

        // Legacy aliases still parse cleanly into the 4 casual tiers.
        assert_eq!(
            "junior".parse::<SearchDifficulty>(),
            Ok(SearchDifficulty::Beginner)
        );
        assert_eq!(
            "intermediate".parse::<SearchDifficulty>(),
            Ok(SearchDifficulty::Normal)
        );
    }

    #[test]
    fn difficulty_switch_is_rejected_while_searching() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(WHITE, RuleSet::Freestyle, GameMode::VsEngine));
        let _task = controller.prepare_engine_search().unwrap();
        controller.set_difficulty(SearchDifficulty::Normal);

        let snapshot = controller.snapshot();
        assert_eq!(snapshot.params.difficulty, "custom");
        assert!(snapshot.error.unwrap().contains("暂不能切换难度"));
    }

    #[test]
    fn invalid_human_side_does_not_reset_the_game() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));
        let move_count = controller.board.move_count();

        assert!(!controller.new_game(0, RuleSet::Renju, GameMode::VsEngine));
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, move_count);
        assert_eq!(snapshot.params.rule, "freestyle");
        assert!(snapshot.error.unwrap().contains("黑方或白方"));
    }

    #[test]
    fn two_player_alternates_without_engine_and_reports_mode() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::TwoPlayer));
        assert_eq!(controller.snapshot().params.mode, "two_player");

        // Black plays: turn passes to white, engine never starts.
        assert!(!controller.play_human(7, 7));
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, 1);
        assert_eq!(snapshot.side_to_move, WHITE);
        assert!(!snapshot.engine_thinking);
        assert!(snapshot.can_play);
        assert!(controller.prepare_engine_search().is_none());

        // White may also be placed by a human on the same device.
        assert!(!controller.play_human(8, 8));
        assert_eq!(controller.snapshot().side_to_move, BLACK);
    }

    #[test]
    fn two_player_renju_exposes_and_rejects_black_forbidden_points() {
        let mut controller = double_three_controller(RuleSet::Renju);
        controller.mode = GameMode::TwoPlayer;
        controller.human_side = WHITE; // irrelevant in two-player; forbidden judged by turn
        let snapshot = controller.snapshot();

        assert!(snapshot.forbidden_points.contains(&ForbiddenPointSnapshot {
            x: 7,
            y: 7,
            kind: "double_three",
        }));
        let move_count = controller.board.move_count();
        assert!(!controller.play_human(7, 7));
        assert_eq!(controller.board.move_count(), move_count);
        assert!(controller.snapshot().error.unwrap().contains("三三禁手"));
    }

    #[test]
    fn two_player_undo_reverts_single_ply() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::TwoPlayer));
        assert!(!controller.play_human(7, 7));
        assert!(!controller.play_human(8, 8));
        assert_eq!(controller.board.move_count(), 2);

        controller.undo_turn();
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, 1);
        assert_eq!(snapshot.side_to_move, WHITE);
    }

    #[test]
    fn two_player_declares_the_winning_side() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::TwoPlayer));
        // Black builds a horizontal five while white plays out of the way.
        for x in 0..4 {
            assert!(!controller.play_human(x, 0)); // black
            assert!(!controller.play_human(x, 5)); // white
        }
        assert!(!controller.play_human(4, 0)); // black completes five-in-a-row
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.winner, BLACK);
        assert_eq!(snapshot.status, "黑方胜。");
        assert!(!snapshot.can_play);
    }

    #[test]
    fn engine_move_that_fills_board_reports_draw_status() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(WHITE, RuleSet::Freestyle, GameMode::VsEngine));
        let (black, white): (Vec<u16>, Vec<u16>) =
            (0..225).partition(|m| ((m % 15 + 2 * (m / 15)) % 4) < 2);
        for i in 0..224 {
            let stones = if i % 2 == 0 { &black } else { &white };
            controller.board.play(stones[i / 2], None).unwrap();
        }
        assert_eq!(controller.board.side_to_move(), BLACK);
        let completion = controller.prepare_engine_search().unwrap().run();
        assert!(controller.commit_engine_search(completion));

        let snapshot = controller.snapshot();
        assert!(snapshot.draw);
        assert_eq!(snapshot.winner, EMPTY);
        assert_eq!(snapshot.status, "和棋。");
        assert!(!snapshot.can_play);
    }

    #[test]
    fn engine_search_reuses_tt_across_moves_and_resets_on_new_game_or_undo() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        assert!(controller.search_tt.is_none());

        assert!(controller.play_human(7, 7));
        let task1 = controller.prepare_engine_search().unwrap();
        assert!(
            task1.tt.is_none(),
            "first search should defer TT allocation to the background worker task"
        );
        assert!(controller.search_tt.is_none());
        let completion1 = task1.run();
        let tt1 = completion1.tt().unwrap().clone();
        assert!(controller.commit_engine_search(completion1));
        assert!(controller
            .search_tt
            .as_ref()
            .is_some_and(|tt| tt.is_shared_with(&tt1)));

        // Play another legal human move away from the engine's reply.
        let next_xy = (0..BOARD_SIZE)
            .flat_map(|y| (0..BOARD_SIZE).map(move |x| (x, y)))
            .find(|&(x, y)| controller.board.at(x, y) == Ok(EMPTY))
            .unwrap();
        assert!(controller.play_human(next_xy.0, next_xy.1));

        let task2 = controller.prepare_engine_search().unwrap();
        assert!(
            task2.tt.as_ref().is_some_and(|tt| tt.is_shared_with(&tt1)),
            "consecutive engine moves in the same game should reuse the same TT"
        );
        let completion2 = task2.run();
        assert!(controller.commit_engine_search(completion2));

        // Changing difficulty clears the TT so lower/different settings do not reuse stale depth entries.
        controller.set_difficulty(SearchDifficulty::Beginner);
        assert!(
            controller.search_tt.is_none(),
            "changing difficulty should clear cached TT"
        );
        // Re-populating TT, then verifying same-difficulty no-op keeps it intact.
        controller.search_tt = Some(tt1.clone());
        controller.set_difficulty(SearchDifficulty::Beginner);
        assert!(controller
            .search_tt
            .as_ref()
            .is_some_and(|tt| tt.is_shared_with(&tt1)));

        controller.search_tt = Some(tt1.clone());
        controller.undo_turn();
        assert!(
            controller.search_tt.is_none(),
            "undo should clear TT to avoid stale priority entries"
        );

        assert!(controller.play_human(next_xy.0, next_xy.1));
        let stale_task = controller.prepare_engine_search().unwrap();
        assert!(stale_task.tt.is_none());

        // Start a new game while the stale task is in flight; committing it must not overwrite TT.
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::VsEngine));
        assert!(controller.search_tt.is_none());
        let stale_completion = stale_task.run();
        assert!(!controller.commit_engine_search(stale_completion));
        assert!(
            controller.search_tt.is_none(),
            "stale completion must not repopulate search_tt after new_game"
        );
    }

    #[test]
    fn interactive_tt_stays_32mib_across_casual_and_pro_modes() {
        let mut config = load_default_config();
        SearchDifficulty::Normal.apply_to_config(&mut config);
        config.runtime.threads = 4;
        let mut controller = GameController::new_with_clock(config, SearchClockMode::Untimed);
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));

        let task = controller.prepare_engine_search().unwrap();
        let completion = task.run();
        assert_eq!(
            completion.tt().unwrap().bucket_count(),
            1 << DEFAULT_INTERACTIVE_TT_BITS,
            "Normal (d4) should allocate 32 MiB TT (tt_bits=20)"
        );
        assert!(controller.commit_engine_search(completion));
        assert!(
            controller.search_pool.lock().unwrap().is_none(),
            "d4 search should not allocate a worker pool"
        );

        controller.set_difficulty(SearchDifficulty::Senior);
        assert!(controller.search_tt.is_none());
        let next_xy = (0..BOARD_SIZE)
            .flat_map(|y| (0..BOARD_SIZE).map(move |x| (x, y)))
            .find(|&(x, y)| controller.board.at(x, y) == Ok(EMPTY))
            .unwrap();
        assert!(controller.play_human(next_xy.0, next_xy.1));

        let task = controller.prepare_engine_search().unwrap();
        // Verify Senior (d6, 4T) keeps unified 32 MiB TT (tt_bits=20) and creates a reusable pool.
        let mut narrow_task = task;
        narrow_task.limits.max_depth = 5;
        narrow_task.limits.root_width = 8;
        let completion = narrow_task.run();
        assert_eq!(
            completion.tt().unwrap().bucket_count(),
            1 << DEFAULT_INTERACTIVE_TT_BITS
        );
        assert!(controller.commit_engine_search(completion));
        assert!(
            controller.search_pool.lock().unwrap().is_some(),
            "parallel depth >= 5 should retain worker pool across moves"
        );

        // Switching back to single-threaded drops the worker pool and resets TT.
        controller.set_threads(1);
        assert_eq!(controller.snapshot().params.threads, 1);
        assert!(controller.search_tt.is_none());
        assert!(controller.search_pool.lock().unwrap().is_none());
    }

    #[test]
    fn stale_multithread_task_does_not_leak_worker_pool_into_single_thread_new_game() {
        let mut config = load_default_config();
        SearchDifficulty::Senior.apply_to_config(&mut config);
        config.runtime.threads = 4;
        let mut controller = GameController::new_with_clock(config, SearchClockMode::Untimed);
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));

        let mut stale_task = controller.prepare_engine_search().unwrap();
        stale_task.limits.max_depth = 5;
        stale_task.limits.root_width = 8;

        // Start a new game and switch to 1 thread while the old 4-thread task is in flight.
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        controller.set_threads(1);
        assert_eq!(controller.snapshot().params.threads, 1);
        assert!(controller.search_pool.lock().unwrap().is_none());

        let stale_completion = stale_task.run();
        assert!(
            stale_completion.has_worker_pool(),
            "old 4T d5 task should have created a worker pool"
        );
        assert!(!controller.commit_engine_search(stale_completion));
        assert!(
            controller.search_pool.lock().unwrap().is_none(),
            "stale completion must not store its worker pool into the 1-thread new game"
        );
    }

    #[test]
    fn stale_task_started_late_does_not_steal_new_game_worker_pool() {
        let mut config = load_default_config();
        SearchDifficulty::Senior.apply_to_config(&mut config);
        config.runtime.threads = 4;
        let mut controller = GameController::new_with_clock(config, SearchClockMode::Untimed);
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));

        // Prepare an old task but do not run it yet.
        let mut stale_task = controller.prepare_engine_search().unwrap();
        stale_task.limits.max_depth = 5;
        stale_task.limits.root_width = 8;

        // Start a new 4-thread game, complete one move, and cache its worker pool.
        assert!(controller.new_game(BLACK, RuleSet::Renju, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));
        let mut fresh_task = controller.prepare_engine_search().unwrap();
        fresh_task.limits.max_depth = 5;
        fresh_task.limits.root_width = 8;
        let fresh_completion = fresh_task.run();
        assert!(controller.commit_engine_search(fresh_completion));
        assert!(controller.search_pool.lock().unwrap().is_some());

        // Running and rejecting the old task later must leave the new game's pool intact.
        let stale_completion = stale_task.run();
        assert!(!controller.commit_engine_search(stale_completion));
        assert!(
            controller.search_pool.lock().unwrap().is_some(),
            "stale task must not steal or drop the new game's cached worker pool"
        );
    }

    #[test]
    fn pro_timed_mode_is_default_at_1000ms_and_supports_midgame_time_changes() {
        let mut config = load_default_config();
        SearchDifficulty::Normal.apply_to_config(&mut config);
        config.runtime.threads = 4;
        let mut controller = GameController::new(config);
        assert_eq!(controller.snapshot().params.clock_mode, "timed");
        assert_eq!(controller.snapshot().params.time_limit_ms, 1000);
        assert!(!controller.snapshot().params.use_nnue);

        assert!(controller.new_game_with_clock(
            BLACK,
            RuleSet::Renju,
            GameMode::VsEngine,
            SearchClockMode::Timed,
        ));
        let params = controller.snapshot().params;
        assert_eq!(params.clock_mode, "timed");
        assert_eq!(params.time_limit_ms, 1000);
        assert_eq!(params.depth, crate::DEFAULT_TIMED_SEARCH_MAX_DEPTH);
        assert!(params.compute_vcf);
        assert!(params.compute_vct);

        assert!(controller.set_time_limit_ms(500));
        assert_eq!(controller.snapshot().params.time_limit_ms, 500);
        assert!(controller.set_time_limit_ms(2500));
        assert_eq!(controller.snapshot().params.time_limit_ms, 2500);
        assert!(!controller.set_time_limit_ms(200));
        assert!(!controller.set_time_limit_ms(2000));
        assert_eq!(controller.snapshot().params.time_limit_ms, 2500);
        assert!(controller.set_time_limit_ms(500));

        assert!(controller.play_human(7, 7));
        let mut task = controller.prepare_engine_search().unwrap();
        assert_eq!(task.limits.time_limit_ms, Some(500.0));
        assert_eq!(task.limits.max_depth, crate::DEFAULT_TIMED_SEARCH_MAX_DEPTH);
        // Reject time limit changes while the engine is thinking.
        assert!(!controller.set_time_limit_ms(1000));

        // Keep unit test runtime short while reaching parallel depth >= 5.
        task.limits.time_limit_ms = Some(200.0);
        let completion = task.run();
        assert_eq!(
            completion.tt().unwrap().bucket_count(),
            1 << DEFAULT_INTERACTIVE_TT_BITS
        );
        assert!(controller.commit_engine_search(completion));
        assert!(controller.search_pool.lock().unwrap().is_some());

        // Switching back to casual Normal on a new game restores d4 and drops the pool.
        assert!(controller.new_game_with_clock(
            BLACK,
            RuleSet::Renju,
            GameMode::VsEngine,
            SearchClockMode::Untimed,
        ));
        assert_eq!(controller.snapshot().params.clock_mode, "untimed");
        assert_eq!(controller.snapshot().params.depth, 4);
        assert!(controller.search_pool.lock().unwrap().is_none());

        // Explicit custom timed_max_wide (e.g. GUI --width 8) is preserved in timed mode.
        let mut custom_width_cfg = load_default_config();
        SearchDifficulty::Normal.apply_to_config(&mut custom_width_cfg);
        custom_width_cfg.root_search.wide = 8;
        custom_width_cfg.root_search.timed_max_wide = 8;
        let mut custom_controller = GameController::new(custom_width_cfg);
        assert!(custom_controller.new_game_with_clock(
            BLACK,
            RuleSet::Renju,
            GameMode::VsEngine,
            SearchClockMode::Timed,
        ));
        assert_eq!(custom_controller.snapshot().params.width, 8);
    }

    #[test]
    fn switching_threads_with_same_tt_capacity_does_not_mix_tt_depth_scales() {
        let mut cfg = load_default_config();
        SearchDifficulty::Normal.apply_to_config(&mut cfg);
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.threads = 4;

        let mut controller = GameController::new(cfg.clone());
        assert!(controller.new_game_with_clock(
            BLACK,
            RuleSet::Freestyle,
            GameMode::VsEngine,
            SearchClockMode::Untimed,
        ));
        assert!(controller.play_human(7, 7));
        let first_task = controller.prepare_engine_search().unwrap();
        let first_done = first_task.run();
        assert!(controller.commit_engine_search(first_done));
        let board_after_reply = controller.board.clone();

        // Switch 4 -> 1 on d4/w20 (same TT capacity) and play [5,7].
        controller.set_threads(1);
        assert!(controller.play_human(5, 7));
        let second_task = controller.prepare_engine_search().unwrap();
        let second_done = second_task.run();
        let (hot_move, hot_score, hot_nodes) = match &second_done.outcome {
            EngineSearchOutcome::Completed { result, .. } => {
                (result.move_, result.score, result.nodes)
            }
            EngineSearchOutcome::Failed { error, .. } => panic!("unexpected failure: {error:?}"),
        };
        assert!(controller.commit_engine_search(second_done));

        // Compare against a cold 1T search on the exact same board.
        let mut cold_board = board_after_reply;
        cold_board
            .play_for_rule(xy_to_move(5, 7).unwrap(), None, RuleSet::Freestyle)
            .unwrap();
        let mut cold_cfg = cfg.clone();
        cold_cfg.runtime.threads = 1;
        let mut cold_searcher = RootSearcher::new(cold_cfg.clone());
        let limits = controller.search_limits();
        let cold = cold_searcher.search(&mut cold_board.clone(), Some(limits));
        assert_eq!(hot_move, cold.move_);
        assert_eq!(hot_score, cold.score);
        assert_eq!(hot_nodes, cold.nodes);

        // Also verify direct TT reuse across new RootSearcher instances (4 -> 1 and 1 -> 4).
        let shared_tt = TranspositionTable::new(16);
        let mut cfg_4t = cfg;
        cfg_4t.runtime.threads = 4;
        let mut s4 = RootSearcher::with_tt(cfg_4t.clone(), shared_tt.clone());
        let _ = s4.search(&mut cold_board.clone(), Some(limits));

        let mut s1 = RootSearcher::with_tt(cold_cfg, shared_tt.clone());
        let r1 = s1.search(&mut cold_board.clone(), Some(limits));
        assert_eq!(r1.move_, cold.move_);
        assert_eq!(r1.score, cold.score);
        assert_eq!(r1.nodes, cold.nodes);

        let mut s4_again = RootSearcher::with_tt(cfg_4t, shared_tt);
        let r4 = s4_again.search(&mut cold_board.clone(), Some(limits));
        assert!(cold_board.is_legal_move(r4.move_));
    }

    #[test]
    fn pro_mode_uses_nnue_when_weights_present_and_casual_always_uses_classical() {
        let dir = Path::new("opponent/rapfi-networks/mix9svq");
        if !mix9svq_files_exist_for_rule(dir, RuleSet::Renju) {
            return;
        }
        let mut config = load_default_config();
        SearchDifficulty::Normal.apply_to_config(&mut config);
        config.runtime.threads = 4;
        config.runtime.nnue_dir = Some(dir.to_string_lossy().into_owned());
        let mut controller = GameController::new(config);

        assert!(controller.new_game_with_clock(
            BLACK,
            RuleSet::Renju,
            GameMode::VsEngine,
            SearchClockMode::Timed,
        ));
        let pro_params = controller.snapshot().params;
        assert!(pro_params.nnue_available);
        assert!(pro_params.use_nnue);
        assert!(pro_params.compute_vcf);
        assert!(
            !pro_params.compute_vct,
            "4T NNUE pro mode skips serial root df-pn VCT by default"
        );
        assert_eq!(pro_params.root_vcf_depth, 16);

        // Switching to Casual must disable NNUE even with nnue_dir configured.
        assert!(controller.new_game_with_clock(
            BLACK,
            RuleSet::Renju,
            GameMode::VsEngine,
            SearchClockMode::Untimed,
        ));
        let casual_params = controller.snapshot().params;
        assert!(casual_params.nnue_available);
        assert!(!casual_params.use_nnue);
        assert_eq!(casual_params.depth, 4);
    }

    #[test]
    fn position_estimate_maps_current_turn_to_black_and_white() {
        let mut board = Board::new();
        let logits = (1000.0, 999.0, 998.0);
        let black_to_move = NnuePositionEstimate::from_logits(&board, logits);
        assert!(black_to_move.black_win > black_to_move.white_win);
        board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
        let white_to_move = NnuePositionEstimate::from_logits(&board, logits);
        assert_eq!(white_to_move.ply, 1);
        assert_eq!(white_to_move.black_win, black_to_move.white_win);
        assert_eq!(white_to_move.white_win, black_to_move.black_win);
        assert_eq!(white_to_move.draw, black_to_move.draw);
        let sum = white_to_move.black_win + white_to_move.white_win + white_to_move.draw;
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pro_mode_runs_fast_engine_and_records_nnue_position_estimate() {
        let dir = Path::new("opponent/rapfi-networks/mix9svq");
        if !mix9svq_files_exist_for_rule(dir, RuleSet::Renju) {
            return;
        }
        let mut config = load_default_config();
        SearchDifficulty::Normal.apply_to_config(&mut config);
        config.runtime.threads = 4;
        config.runtime.nnue_dir = Some(dir.to_string_lossy().into_owned());
        let mut controller =
            GameController::new_with_clock(config, SearchClockMode::Timed).with_nnue_history();

        assert!(controller.new_game_with_clock(
            BLACK,
            RuleSet::Renju,
            GameMode::VsEngine,
            SearchClockMode::Timed,
        ));
        assert!(controller.play_human(7, 7));

        let mut task = controller.prepare_engine_search().unwrap();
        task.limits.time_limit_ms = Some(150.0);
        let completion = task.run();
        assert!(controller.commit_engine_search(completion));
        assert!(controller.fast_searcher.is_some());

        let snap = controller.snapshot();
        let res = snap.last_result.expect("pro search result");
        assert!(res.used_nnue);
        assert!(res.depth >= 1);
        assert!(res.nodes >= 1);
        let hist = snap.nnue_history.expect("nnue history recorded");
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].ply, 2);
        let sum = hist[0].black_win + hist[0].draw + hist[0].white_win;
        assert!((sum - 1.0).abs() < 1e-5);
    }
}
