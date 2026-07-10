//! Search modules aligned with the classic reference.

pub mod alphabeta;
pub mod movegen;
pub mod ordering;
pub mod root;
pub mod tt;

pub use alphabeta::{
    compute_corner_state, rootbonus, terminal_score, AlphaBetaSearcher, RootCandidateProfile,
    SearchOptions, SearchStats,
};
pub use movegen::{
    apply_hostile_three_extension, covered_moves, diagnose_candidates, generate_candidates,
    movegen_backend_name, Candidate, CandidateDiagnostic, CandidateDiagnosticsResult,
    CandidateGenerationResult,
};
pub use ordering::{getmi, order_candidates, order_candidates_root_classic, ordering_backend_name};
pub use root::{
    fallback_ai_move, fallback_move_score, new_classic_fallback_rng, ClassicFallbackRng,
    FallbackMoveScore, RootDepthProfile, RootSearchError, RootSearcher, RootTrace, SearchLimits,
    SearchResult,
};
pub use tt::{
    ProbeResult, TTBestMoveHint, TTEntry, TTError, TranspositionTable, MAX_TT_BUCKET_BITS,
};
