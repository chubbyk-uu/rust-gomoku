//! Native Rapfi-aligned NNUE engine core (isolated from classical `pygomoku` base).

pub mod board;
pub mod classical_model;
pub mod history;
pub mod movegen;
pub mod movepick;
pub mod pattern;
pub mod search;
pub mod tt;

pub use board::{chebyshev_distance, step_pos, CandRange, FastBoard, NONE_MOVE, PASS_MOVE};
pub use pattern::{
    lookup_defence, lookup_pattern, PCodeP4, Pattern, Pattern2x, Pattern4, PATTERN4_NB, PATTERN_NB,
};
pub use search::{FastSearchConfig, FastSearchResult, FastSearcher};
pub use tt::FastTT;
