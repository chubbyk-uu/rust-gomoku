//! Shared application controller for desktop and mobile frontends.

use std::time::Instant;

use serde::Serialize;

use crate::{
    apply_engine_profile, move_to_xy, xy_to_move, Board, EngineConfig, EngineProfile,
    ForbiddenKind, RootSearcher, RootTrace, RuleSet, SearchLimits, SearchResult, Side, BLACK,
    BOARD_SIZE, EMPTY, WHITE,
};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDifficulty {
    Beginner,
    Junior,
    Intermediate,
    Senior,
    Master,
}

impl SearchDifficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Junior => "junior",
            Self::Intermediate => "intermediate",
            Self::Senior => "senior",
            Self::Master => "master",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Beginner => "入门",
            Self::Junior => "初级",
            Self::Intermediate => "中级",
            Self::Senior => "高级",
            Self::Master => "大师",
        }
    }

    fn settings(self) -> (i32, i32, bool) {
        match self {
            Self::Beginner => (1, 10, false),
            Self::Junior => (2, 10, false),
            Self::Intermediate => (4, 20, false),
            Self::Senior => (6, 30, true),
            Self::Master => (8, 40, true),
        }
    }

    pub fn apply_to_config(self, config: &mut EngineConfig) {
        let (depth, width, tactical) = self.settings();
        config.root_search.depth = depth;
        config.root_search.wide = width;
        config.root_search.timed_max_wide = width;
        config.runtime.compute_vcf = tactical;
        config.runtime.compute_vct = tactical;
    }

    fn detect(config: &EngineConfig) -> Option<Self> {
        [
            Self::Beginner,
            Self::Junior,
            Self::Intermediate,
            Self::Senior,
            Self::Master,
        ]
        .into_iter()
        .find(|difficulty| {
            let (depth, width, tactical) = difficulty.settings();
            config.root_search.depth == depth
                && config.root_search.wide == width
                && config.runtime.compute_vcf == tactical
                && config.runtime.compute_vct == tactical
        })
    }
}

impl Default for SearchDifficulty {
    fn default() -> Self {
        Self::Intermediate
    }
}

impl std::str::FromStr for SearchDifficulty {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.to_ascii_lowercase().as_str() {
            "beginner" | "intro" => Ok(Self::Beginner),
            "junior" | "easy" => Ok(Self::Junior),
            "intermediate" | "normal" | "medium" => Ok(Self::Intermediate),
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
    last_result: Option<SearchResult>,
    last_trace: Option<RootTrace>,
    last_search_ms: Option<f64>,
    difficulty: Option<SearchDifficulty>,
    generation: u64,
}

#[derive(Clone)]
pub struct EngineSearchTask {
    board: Board,
    config: EngineConfig,
    limits: SearchLimits,
    generation: u64,
}

pub struct EngineSearchCompletion {
    result: SearchResult,
    trace: Option<RootTrace>,
    elapsed_ms: f64,
    generation: u64,
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
    pub move_count: usize,
    pub can_play: bool,
    pub engine_thinking: bool,
    pub status: String,
    pub error: Option<String>,
    pub last_mark: Option<[usize; 2]>,
    pub last_result: Option<SearchResultSnapshot>,
    pub last_trace: Option<SearchTraceSnapshot>,
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
    pub fast_history_ordering: bool,
    pub killer_hits: usize,
    pub history_hits: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GameParamsSnapshot {
    pub mode: &'static str,
    pub profile: &'static str,
    pub difficulty: &'static str,
    pub depth: i32,
    pub width: usize,
    pub compute_vcf: bool,
    pub root_vcf_depth: i32,
    pub opponent_vcf_depth: i32,
    pub compute_vct: bool,
    pub root_vct_depth: i32,
    pub overlap_vct_alphabeta: bool,
    pub fast_history_ordering: bool,
    pub static_board: bool,
    pub dynamic_board_margin: i32,
    pub rule: &'static str,
}

impl GameController {
    pub fn new(config: EngineConfig) -> Self {
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
            difficulty,
            generation: 0,
        }
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn new_game(&mut self, human_side: Side, rule: RuleSet, mode: GameMode) -> bool {
        if human_side != BLACK && human_side != WHITE {
            self.error = Some("执棋方必须是黑方或白方。".to_string());
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.board.reset();
        self.config.rule_set = rule;
        self.human_side = human_side;
        self.mode = mode;
        self.engine_thinking = false;
        let rule_name = match rule {
            RuleSet::Freestyle => "无禁手",
            RuleSet::Renju => "有禁手",
        };
        self.status = match mode {
            GameMode::TwoPlayer => format!("新局开始（双人 / {rule_name}）：黑方先行。"),
            GameMode::VsEngine if human_side == BLACK => {
                format!("新局开始（{rule_name}）：你执黑，请落子。")
            }
            GameMode::VsEngine => {
                format!("新局开始（{rule_name}）：你执白，引擎执黑思考中。")
            }
        };
        self.error = None;
        self.last_mark = None;
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
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

    pub fn set_profile(&mut self, profile: EngineProfile) {
        if self.engine_thinking {
            self.error = Some("引擎思考中，暂不能切换模式。".to_string());
            return;
        }
        apply_engine_profile(&mut self.config, profile);
        self.error = None;
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
        self.status = format!(
            "已切换到 {} 模式，当前棋局不变，下一次引擎思考生效。",
            match profile {
                EngineProfile::Base => "Base",
                EngineProfile::Fast => "Fast",
            }
        );
    }

    pub fn set_difficulty(&mut self, difficulty: SearchDifficulty) {
        if self.engine_thinking {
            self.error = Some("引擎思考中，暂不能切换难度。".to_string());
            return;
        }
        let (depth, width, tactical) = difficulty.settings();
        difficulty.apply_to_config(&mut self.config);
        self.difficulty = Some(difficulty);
        self.error = None;
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
        self.status = format!(
            "已切换到{}难度（d{depth}/w{width}，VCF/VCT {}），下一次引擎思考生效。",
            difficulty.label(),
            if tactical { "开启" } else { "关闭" }
        );
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub fn prepare_engine_search(&mut self) -> Option<EngineSearchTask> {
        if self.mode == GameMode::TwoPlayer
            || self.engine_thinking
            || self.board.winner() != EMPTY
            || self.board.side_to_move() == self.human_side
        {
            return None;
        }
        self.engine_thinking = true;
        self.error = None;
        self.status = "引擎思考中...".to_string();
        Some(EngineSearchTask {
            board: self.board.clone(),
            config: self.config.clone(),
            limits: self.search_limits(),
            generation: self.generation,
        })
    }

    pub fn commit_engine_search(&mut self, completion: EngineSearchCompletion) -> bool {
        if self.generation != completion.generation {
            return false;
        }
        if self.board.winner() == EMPTY && self.board.side_to_move() != self.human_side {
            if !self.board.is_legal_move_for_rule(
                completion.result.move_,
                self.board.side_to_move(),
                self.config.rule_set,
            ) {
                self.error = Some("引擎返回禁手或非法落子。".to_string());
                self.status = "引擎落子失败。".to_string();
            } else {
                match self
                    .board
                    .play_for_rule(completion.result.move_, None, self.config.rule_set)
                {
                    Ok(_) => {
                        self.last_mark = move_to_xy(completion.result.move_).ok();
                        self.last_result = Some(completion.result);
                        self.last_trace = completion.trace;
                        self.last_search_ms =
                            Some((completion.elapsed_ms * 1000.0).round() / 1000.0);
                        self.status = if self.board.winner() == EMPTY {
                            "引擎已落子，轮到你。".to_string()
                        } else {
                            "引擎获胜。".to_string()
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
        let last_result = self.last_result.map(|result| {
            let (x, y) = move_to_xy(result.move_).expect("engine move stays valid");
            SearchResultSnapshot {
                move_xy: [x, y],
                score: result.score,
                depth: result.depth,
                nodes: result.nodes,
                ms: self.last_search_ms,
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
            fast_history_ordering: trace.fast_history_ordering,
            killer_hits: trace.killer_hits,
            history_hits: trace.history_hits,
        });
        let limits = self.search_limits();
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
            move_count: self.board.move_count(),
            can_play: self.can_human_play(),
            engine_thinking: self.engine_thinking,
            status: self.status.clone(),
            error: self.error.clone(),
            last_mark: self.last_mark.map(|(x, y)| [x, y]),
            last_result,
            last_trace,
            params: GameParamsSnapshot {
                mode: self.mode.as_str(),
                profile: self.config.profile.as_str(),
                difficulty: self
                    .difficulty
                    .map(SearchDifficulty::as_str)
                    .unwrap_or("custom"),
                depth: limits.max_depth,
                width: limits.root_width,
                compute_vcf: self.config.runtime.compute_vcf,
                root_vcf_depth: self.config.runtime.root_vcf_depth,
                opponent_vcf_depth: self.config.runtime.opponent_vcf_depth,
                compute_vct: self.config.runtime.compute_vct,
                root_vct_depth: self.config.runtime.root_vct_depth,
                overlap_vct_alphabeta: self.config.runtime.overlap_vct_alphabeta,
                fast_history_ordering: self.config.runtime.fast_history_ordering,
                static_board: self.config.runtime.static_board,
                dynamic_board_margin: self.config.runtime.dynamic_board_margin,
                rule: self.config.rule_set.as_str(),
            },
        }
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
        if self.engine_thinking || self.board.winner() != EMPTY {
            return false;
        }
        match self.mode {
            // Either side may be placed by a human on the same device.
            GameMode::TwoPlayer => true,
            GameMode::VsEngine => self.board.side_to_move() == self.human_side,
        }
    }

    fn search_limits(&self) -> SearchLimits {
        SearchLimits::fixed_from_config(&self.config)
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
    pub fn run(mut self) -> EngineSearchCompletion {
        let mut searcher = RootSearcher::new(self.config);
        let start = Instant::now();
        let result = searcher.search(&mut self.board, Some(self.limits));
        EngineSearchCompletion {
            result,
            trace: searcher.last_trace,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            generation: self.generation,
        }
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
        GameController::new(config)
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
    fn undo_and_profile_switch_keep_controller_consistent() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(BLACK, RuleSet::Freestyle, GameMode::VsEngine));
        assert!(controller.play_human(7, 7));
        let completion = controller.prepare_engine_search().unwrap().run();
        assert!(controller.commit_engine_search(completion));
        controller.undo_turn();
        controller.set_profile(EngineProfile::Fast);

        let snapshot = controller.snapshot();
        assert_eq!(snapshot.move_count, 0);
        assert_eq!(snapshot.params.profile, "fast");
        assert!(snapshot.can_play);
    }

    #[test]
    fn difficulty_presets_update_search_and_tactical_settings() {
        let mut controller = GameController::new(load_default_config());
        assert_eq!(controller.snapshot().params.difficulty, "master");

        for (difficulty, name, depth, width, tactical) in [
            (SearchDifficulty::Beginner, "beginner", 1, 10, false),
            (SearchDifficulty::Junior, "junior", 2, 10, false),
            (SearchDifficulty::Intermediate, "intermediate", 4, 20, false),
            (SearchDifficulty::Senior, "senior", 6, 30, true),
            (SearchDifficulty::Master, "master", 8, 40, true),
        ] {
            controller.set_difficulty(difficulty);
            let params = controller.snapshot().params;
            assert_eq!(params.difficulty, name);
            assert_eq!(params.depth, depth);
            assert_eq!(params.width, width);
            assert_eq!(params.compute_vcf, tactical);
            assert_eq!(params.compute_vct, tactical);
        }
    }

    #[test]
    fn difficulty_switch_is_rejected_while_searching() {
        let mut controller = shallow_controller();
        assert!(controller.new_game(WHITE, RuleSet::Freestyle, GameMode::VsEngine));
        let _task = controller.prepare_engine_search().unwrap();
        controller.set_difficulty(SearchDifficulty::Junior);

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
}
