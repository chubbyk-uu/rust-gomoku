//! Rule-specific helpers that are not wired into default freestyle behavior.

pub mod forbidden;

pub use forbidden::{classify_forbidden_move, classify_forbidden_stones, ForbiddenKind, RuleSet};
