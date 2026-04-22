//! Shared value types used across the engine.

use crate::constants::{BLACK, WHITE};

pub type Move = u16;
pub type Side = i8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayedMove {
    pub move_: Move,
    pub side: Side,
}

pub fn is_valid_side(side: Side) -> bool {
    matches!(side, BLACK | WHITE)
}

pub fn opposite_side(side: Side) -> Side {
    -side
}
