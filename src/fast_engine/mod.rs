//! Native Rapfi-aligned NNUE engine core (isolated from classical `pygomoku` base).
//!
//! Milestone 1 provides the zero-allocation 15x15 `FastBoard` with dense-key
//! `Pattern` / 14-level `Pattern4` tables, exact Renju forbidden detection,
//! and incremental candidate bitboards.

pub mod board;
pub mod pattern;

pub use board::{CandRange, FastBoard, PASS_MOVE};
pub use pattern::{
    lookup_defence, lookup_pattern, PCodeP4, Pattern, Pattern2x, Pattern4, PATTERN4_NB, PATTERN_NB,
};
