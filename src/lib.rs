//! Rust reconstruction of the `pygomoku` classic engine semantics.
//!
//! The current phase focuses on the deterministic base layer:
//! constants, shared types, move encoding, board state, and zobrist hashing.

pub mod app;
pub mod board;
pub mod config;
pub mod constants;
pub mod eval;
pub mod patterns;
pub mod protocol;
pub mod rules;
pub mod search;
pub mod threats;
pub mod types;
pub mod zobrist;

pub use app::{
    EngineSearchCompletion, EngineSearchTask, ForbiddenPointSnapshot, GameController,
    GameMoveSnapshot, GameParamsSnapshot, GameStateSnapshot, SearchResultSnapshot,
    SearchTraceSnapshot,
};
pub use board::{move_to_rc, move_to_xy, rc_to_move, xy_to_move, Board, BoardError};
pub use config::{
    adjust_loaded_parameters, apply_engine_profile, default_eval_para, load_config_for_profile,
    load_default_config, EngineConfig, EngineProfile, EvalBucketTables, RootSearchDefaults,
    RuntimeOptions, SearchParameters, DEFAULT_CHILD_WIDTH_RATIO_DEN, DEFAULT_CHILD_WIDTH_RATIO_NUM,
    DEFAULT_DYNAMIC_BOARD_MARGIN, DEFAULT_ENGINE_PROFILE, DEFAULT_EVAL_PARA,
    DEFAULT_FAST_HISTORY_BONUS_CAP, DEFAULT_FAST_HISTORY_BONUS_SCALE,
    DEFAULT_FAST_HISTORY_ORDERING, DEFAULT_FAST_KILLER_BONUS,
    DEFAULT_FAST_PROFILE_HISTORY_ORDERING, DEFAULT_OPPONENT_VCF_DEPTH,
    DEFAULT_OVERLAP_VCT_ALPHABETA, DEFAULT_ROOT_PROFILE, DEFAULT_ROOT_VCF_DEPTH,
    DEFAULT_ROOT_VCT_DEPTH, DEFAULT_SEARCH_DEPTH, DEFAULT_SEARCH_WIDTH,
    DEFAULT_TIMED_SEARCH_MAX_DEPTH, DEFAULT_TIMED_SEARCH_MAX_WIDTH, DEFAULT_VCF_MULTI_REPLY,
    DEFAULT_VCT_STRICT_AND_MEMO_KEY, DEFAULT_VCT_VERIFY_OPPONENT_VCF_DEPTH,
};
pub use constants::{BLACK, BOARD_AREA, BOARD_SIZE, EMPTY, WHITE};
pub use eval::{
    attack_level, compute_bucket_and_attack, compute_direction_shape,
    compute_direction_shape_for_rule, eval_value_last, eval_value_next, evaluate_board,
    evaluate_board_main, evaluate_board_main_cached, evaluate_board_main_scan,
    evaluate_last5_branch, evaluate_next43_branch, find_last5_target, global_eval_backend_name,
    local_backend_name, move_value, recompute_all, recompute_all_for_rule, recompute_point_caches,
    recompute_point_caches_for_rule, value_wide_compute, value_wide_compute_for_rule,
};
pub use eval::{caches_backend_name, EvalCaches, EvalSnapshot};
pub use patterns::{
    bucket_for_lines, line_backend_name, shape_raw_from_cells_python, Line, PackedShape,
    PatternError, ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, DIRECTION_IDS, DOUBLE_SHAPE, HORIZONTAL,
    VERTICAL,
};
pub use protocol::{GomocupProtocol, ABOUT_TEXT};
pub use rules::{classify_forbidden_move, classify_forbidden_stones, ForbiddenKind, RuleSet};
pub use search::{
    apply_hostile_three_extension, compute_corner_state, covered_moves, diagnose_candidates,
    fallback_ai_move, fallback_move_score, generate_candidates, getmi, movegen_backend_name,
    new_classic_fallback_rng, order_candidates, order_candidates_root_classic,
    ordering_backend_name, rootbonus, terminal_score, AlphaBetaSearcher, Candidate,
    CandidateDiagnostic, CandidateDiagnosticsResult, CandidateGenerationResult, ClassicFallbackRng,
    FallbackMoveScore, ProbeResult, RootCandidateProfile, RootDepthProfile, RootSearcher,
    RootTrace, SearchLimits, SearchOptions, SearchResult, SearchStats, TTBestMoveHint, TTEntry,
    TranspositionTable,
};
pub use threats::{
    broken_four_reply, forcing_threat_moves, forcing_threat_moves_for_rule, has_open_four,
    has_vct_trigger, has_vct_trigger_for_rule, threat_moves, winning_threat_moves, AttackMove,
    ThreatBoardView, ThreatLevel, VCFResult, VCFSearcher, VCTAndMemoCollisionSample, VCTDepthStats,
    VCTResult, VCTSearcher, VCTStats, VcfMemoEntry, VctMemoEntry, NO_MOVE, VCFM,
};
pub use types::{Move, PlayedMove, Side};
pub use zobrist::{ZobristError, ZobristTable, DEFAULT_ZOBRIST};
