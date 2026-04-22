//! Rust reconstruction of the `pygomoku` classic engine semantics.
//!
//! The current phase focuses on the deterministic base layer:
//! constants, shared types, move encoding, board state, and zobrist hashing.

pub mod board;
pub mod config;
pub mod constants;
pub mod eval;
pub mod patterns;
pub mod search;
pub mod threats;
pub mod types;
pub mod zobrist;

pub use board::{move_to_xy, xy_to_move, Board, BoardError};
pub use config::{
    adjust_loaded_parameters, default_eval_para, load_default_config, EngineConfig,
    EvalBucketTables, RootSearchDefaults, RuntimeOptions, SearchParameters, DEFAULT_EVAL_PARA,
};
pub use constants::{BLACK, BOARD_AREA, BOARD_SIZE, EMPTY, WHITE};
pub use eval::{
    attack_level, compute_bucket_and_attack, compute_direction_shape, eval_value_last,
    eval_value_next, evaluate_board, evaluate_board_main, evaluate_last5_branch,
    evaluate_next43_branch, find_last5_target, global_eval_backend_name, local_backend_name,
    move_value, recompute_all, recompute_point_caches, value_wide_compute,
};
pub use eval::{caches_backend_name, EvalCaches, EvalSnapshot};
pub use patterns::{
    bucket_for_lines, line_backend_name, shape_raw_from_cells_python, Line, PackedShape,
    PatternError, ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, DIRECTION_IDS, DOUBLE_SHAPE, HORIZONTAL,
    VERTICAL,
};
pub use search::{
    apply_hostile_three_extension, compute_corner_state, covered_moves, fallback_ai_move,
    generate_candidates, getmi, movegen_backend_name, new_classic_fallback_rng, order_candidates,
    order_candidates_root_classic, ordering_backend_name, rootbonus, terminal_score,
    AlphaBetaSearcher, Candidate, CandidateGenerationResult, ClassicFallbackRng, NullVctSearcher,
    ProbeResult, RootSearcher, SearchLimits, SearchOptions, SearchResult, SearchStats, TTEntry,
    TranspositionTable, VctSearchResult,
};
pub use threats::{
    broken_four_reply, forcing_threat_moves, has_open_four, has_vct_trigger, threat_moves,
    winning_threat_moves, AttackMove, ThreatBoardView, ThreatLevel, VCFResult, VCFSearcher,
    VCTResult, VCTSearcher, VcfMemoEntry, VctMemoEntry, NO_MOVE, VCFM,
};
pub use types::{Move, PlayedMove, Side};
pub use zobrist::{ZobristError, ZobristTable, DEFAULT_ZOBRIST};
