//! Shared threat-search result types.

use crate::types::Move;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThreatLevel {
    A3 = 1,
    B4 = 2,
    A4 = 3,
    WIN5 = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttackMove {
    pub move_: Move,
    pub level: ThreatLevel,
    pub defenses: Vec<Move>,
}
