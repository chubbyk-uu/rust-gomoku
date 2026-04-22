//! Core engine constants aligned with the Python reference implementation.

pub const BOARD_SIZE: usize = 15;
pub const BOARD_AREA: usize = BOARD_SIZE * BOARD_SIZE;

pub const BLACK: i8 = 1;
pub const WHITE: i8 = -1;
pub const EMPTY: i8 = 0;

pub const DSHAPE_SIZE: usize = 92;

pub const WIN: i32 = 15_000;
pub const INF: i32 = 20_000;
pub const NEXT5: u32 = 0x1000;
pub const LAST5: u32 = 0x200;
pub const NEXT4: u32 = 0x10;
pub const NEXT43: u32 = 0x1;

pub const HASHF_EMPTY: u8 = 0;
pub const HASHF_EXACT: u8 = 1;
pub const HASHF_ALPHA: u8 = 2;
pub const HASHF_BETA: u8 = 3;
