//! Zobrist hashing support aligned with the classic Python reference.

use std::sync::LazyLock;

use crate::constants::{BLACK, BOARD_AREA, WHITE};
use crate::types::{is_valid_side, Move, Side};

const DEFAULT_SEED: u32 = 1_232_356;
const MASK64: u64 = u64::MAX;
const CLASSIC_HASH_N: usize = 20;
const CLASSIC_HASH_AREA: usize = CLASSIC_HASH_N * CLASSIC_HASH_N + 1;

unsafe extern "C" {
    fn srand(seed: u32);
    fn rand() -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZobristError {
    InvalidMove(Move),
    InvalidSide(Side),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZobristTable {
    black: [u64; CLASSIC_HASH_AREA],
    white: [u64; CLASSIC_HASH_AREA],
    turn: u64,
}

impl ZobristTable {
    pub fn build(seed: u32) -> Self {
        let values = classic_rand64_sequence(seed, CLASSIC_HASH_AREA * 2);
        let black = values[..CLASSIC_HASH_AREA]
            .try_into()
            .expect("classic zobrist black table shape");
        let white = values[CLASSIC_HASH_AREA..CLASSIC_HASH_AREA * 2]
            .try_into()
            .expect("classic zobrist white table shape");

        // The reference hash does not encode side-to-move.
        Self {
            black,
            white,
            turn: 0,
        }
    }

    pub fn key_for(&self, move_: Move, side: Side) -> Result<u64, ZobristError> {
        let index = usize::from(move_);
        if index >= BOARD_AREA {
            return Err(ZobristError::InvalidMove(move_));
        }
        if side == BLACK {
            return Ok(self.black[index]);
        }
        if side == WHITE {
            return Ok(self.white[index]);
        }
        Err(ZobristError::InvalidSide(side))
    }

    pub fn key_for_turn(&self) -> u64 {
        self.turn
    }
}

pub static DEFAULT_ZOBRIST: LazyLock<ZobristTable> =
    LazyLock::new(|| ZobristTable::build(DEFAULT_SEED));

fn classic_rand64_sequence(seed: u32, count: usize) -> Vec<u64> {
    // The Python reference builds the classic zobrist stream from the host libc
    // `rand()` sequence, so Rust must do the same to stay aligned on the same machine.
    unsafe {
        srand(seed);
    }

    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let mut value = rand32() as u64;
        value = (value ^ (u64::from(rand32()) << 15)) & MASK64;
        value = (value ^ (u64::from(rand32()) << 30)) & MASK64;
        value = (value ^ (u64::from(rand32()) << 45)) & MASK64;
        value = (value ^ (u64::from(rand32()) << 60)) & MASK64;
        values.push(value);
    }
    values
}

fn rand32() -> u32 {
    unsafe { rand() as u32 }
}

pub fn validate_side(side: Side) -> Result<(), ZobristError> {
    if is_valid_side(side) {
        Ok(())
    } else {
        Err(ZobristError::InvalidSide(side))
    }
}
