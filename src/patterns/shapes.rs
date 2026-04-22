//! Packed shape definitions and direction identifiers.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum ShapeLabel {
    L0 = 0,
    L1S = 1,
    L1 = 2,
    L2S = 3,
    L2BB = 4,
    L2B = 5,
    L2 = 6,
    L3S = 7,
    L3B = 8,
    L3 = 9,
    L4S = 10,
    L4 = 11,
    L5 = 12,
    L6 = 13,
}

impl ShapeLabel {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            0 => Self::L0,
            1 => Self::L1S,
            2 => Self::L1,
            3 => Self::L2S,
            4 => Self::L2BB,
            5 => Self::L2B,
            6 => Self::L2,
            7 => Self::L3S,
            8 => Self::L3B,
            9 => Self::L3,
            10 => Self::L4S,
            11 => Self::L4,
            12 => Self::L5,
            13 => Self::L6,
            _ => Self::L0,
        }
    }
}

pub const HORIZONTAL: i32 = 0;
pub const VERTICAL: i32 = 1;
pub const DIAGONAL_DOWN: i32 = 2;
pub const DIAGONAL_UP: i32 = 3;

pub const DIRECTION_IDS: (i32, i32, i32, i32) = (HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedShape {
    pub raw: i32,
}

impl PackedShape {
    pub fn label(self) -> ShapeLabel {
        ShapeLabel::from_raw((self.raw >> 16) & 0xF)
    }

    pub fn aux(self) -> i32 {
        self.raw & 0xF
    }
}
