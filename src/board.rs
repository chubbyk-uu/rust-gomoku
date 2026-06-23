//! Board state, move encoding, and deterministic make/unmake semantics.
//!
//! Public protocol-style coordinates use `(x, y) = (column, row)`, matching
//! Gomocup and GUI conventions. The internal grid is stored as
//! `grid[row][column]`, and `Move` is encoded as `row * BOARD_SIZE + column`.

use crate::constants::{BLACK, BOARD_AREA, BOARD_SIZE, EMPTY};
use crate::rules::{classify_forbidden_move, ForbiddenKind, RuleSet};
use crate::types::{is_valid_side, opposite_side, Move, PlayedMove, Side};
use crate::zobrist::{validate_side, ZobristError, ZobristTable, DEFAULT_ZOBRIST};

const DIRECTIONS: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardError {
    CoordinatesOutOfRange { x: usize, y: usize },
    MoveOutOfRange(Move),
    InvalidSide(Side),
    WrongSideToMove { expected: Side, got: Side },
    IllegalMove(Move),
    EmptyHistory,
}

impl From<ZobristError> for BoardError {
    fn from(value: ZobristError) -> Self {
        match value {
            ZobristError::InvalidMove(move_) => Self::MoveOutOfRange(move_),
            ZobristError::InvalidSide(side) => Self::InvalidSide(side),
        }
    }
}

pub fn xy_to_move(x: usize, y: usize) -> Result<Move, BoardError> {
    if !(x < BOARD_SIZE && y < BOARD_SIZE) {
        return Err(BoardError::CoordinatesOutOfRange { x, y });
    }
    Ok((y * BOARD_SIZE + x) as Move)
}

pub fn rc_to_move(row: usize, col: usize) -> Result<Move, BoardError> {
    xy_to_move(col, row)
}

pub fn move_to_xy(move_: Move) -> Result<(usize, usize), BoardError> {
    let index = usize::from(move_);
    if index >= BOARD_AREA {
        return Err(BoardError::MoveOutOfRange(move_));
    }
    Ok((index % BOARD_SIZE, index / BOARD_SIZE))
}

pub fn move_to_rc(move_: Move) -> Result<(usize, usize), BoardError> {
    let (col, row) = move_to_xy(move_)?;
    Ok((row, col))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    size: usize,
    grid: [[Side; BOARD_SIZE]; BOARD_SIZE],
    move_history: Vec<PlayedMove>,
    side_to_move: Side,
    winner: Side,
    zobrist_table: ZobristTable,
    zobrist_key: u64,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    pub fn new() -> Self {
        Self::with_side_to_move(BLACK).expect("BLACK is a valid side")
    }

    pub fn with_side_to_move(side_to_move: Side) -> Result<Self, BoardError> {
        validate_side(side_to_move)?;
        Ok(Self {
            size: BOARD_SIZE,
            grid: [[EMPTY; BOARD_SIZE]; BOARD_SIZE],
            move_history: Vec::new(),
            side_to_move,
            winner: EMPTY,
            zobrist_table: DEFAULT_ZOBRIST.clone(),
            zobrist_key: 0,
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn move_count(&self) -> usize {
        self.move_history.len()
    }

    pub fn side_to_move(&self) -> Side {
        self.side_to_move
    }

    pub fn winner(&self) -> Side {
        self.winner
    }

    pub fn zobrist_key(&self) -> u64 {
        self.zobrist_key
    }

    pub fn move_history(&self) -> &[PlayedMove] {
        &self.move_history
    }

    pub fn in_bounds(&self, x: usize, y: usize) -> bool {
        x < BOARD_SIZE && y < BOARD_SIZE
    }

    pub fn at(&self, x: usize, y: usize) -> Result<Side, BoardError> {
        if !self.in_bounds(x, y) {
            return Err(BoardError::CoordinatesOutOfRange { x, y });
        }
        Ok(self.grid[y][x])
    }

    pub fn at_rc(&self, row: usize, col: usize) -> Result<Side, BoardError> {
        self.at(col, row)
    }

    pub fn is_legal_move(&self, move_: Move) -> bool {
        if self.winner != EMPTY {
            return false;
        }
        let Ok((col, row)) = move_to_xy(move_) else {
            return false;
        };
        self.grid[row][col] == EMPTY
    }

    pub fn forbidden_kind_for_rule(
        &self,
        move_: Move,
        side: Side,
        rule: RuleSet,
    ) -> Result<ForbiddenKind, BoardError> {
        classify_forbidden_move(self, move_, side, rule)
    }

    pub fn is_legal_move_for_rule(&self, move_: Move, side: Side, rule: RuleSet) -> bool {
        if !self.is_legal_move(move_) {
            return false;
        }
        classify_forbidden_move(self, move_, side, rule).is_ok_and(|kind| !kind.is_forbidden())
    }

    pub fn play(&mut self, move_: Move, side: Option<Side>) -> Result<PlayedMove, BoardError> {
        let side = side.unwrap_or(self.side_to_move);
        if !is_valid_side(side) {
            return Err(BoardError::InvalidSide(side));
        }
        if side != self.side_to_move {
            return Err(BoardError::WrongSideToMove {
                expected: self.side_to_move,
                got: side,
            });
        }
        if !self.is_legal_move(move_) {
            return Err(BoardError::IllegalMove(move_));
        }

        let (col, row) = move_to_xy(move_)?;
        self.grid[row][col] = side;
        let played = PlayedMove { move_, side };
        self.move_history.push(played);
        self.zobrist_key ^= self.zobrist_table.key_for_turn();
        self.zobrist_key ^= self.zobrist_table.key_for(move_, side)?;
        if self.is_winning_move(col, row, side) {
            self.winner = side;
        }
        self.side_to_move = opposite_side(side);
        Ok(played)
    }

    pub fn undo(&mut self) -> Result<PlayedMove, BoardError> {
        let played = self.move_history.pop().ok_or(BoardError::EmptyHistory)?;
        let (col, row) = move_to_xy(played.move_)?;
        self.grid[row][col] = EMPTY;
        self.zobrist_key ^= self.zobrist_table.key_for(played.move_, played.side)?;
        self.zobrist_key ^= self.zobrist_table.key_for_turn();
        self.side_to_move = played.side;
        self.winner = EMPTY;
        Ok(played)
    }

    pub fn replay(&mut self, moves: &[Move], first_side: Side) -> Result<(), BoardError> {
        if !is_valid_side(first_side) {
            return Err(BoardError::InvalidSide(first_side));
        }
        self.reset();
        self.side_to_move = first_side;
        for &move_ in moves {
            self.play(move_, None)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.grid = [[EMPTY; BOARD_SIZE]; BOARD_SIZE];
        self.move_history.clear();
        self.side_to_move = BLACK;
        self.winner = EMPTY;
        self.zobrist_key = 0;
    }

    pub fn occupied_moves(&self) -> Vec<Move> {
        self.move_history
            .iter()
            .map(|played| played.move_)
            .collect()
    }

    pub(crate) fn grid_rows(&self) -> &[[Side; BOARD_SIZE]; BOARD_SIZE] {
        &self.grid
    }

    pub(crate) fn grid_rows_mut(&mut self) -> &mut [[Side; BOARD_SIZE]; BOARD_SIZE] {
        &mut self.grid
    }

    pub(crate) fn force_side_to_move(&mut self, side: Side) -> Result<(), BoardError> {
        if !is_valid_side(side) {
            return Err(BoardError::InvalidSide(side));
        }
        if self.side_to_move != side {
            self.zobrist_key ^= self.zobrist_table.key_for_turn();
            self.side_to_move = side;
        }
        Ok(())
    }

    fn is_winning_move(&self, x: usize, y: usize, side: Side) -> bool {
        DIRECTIONS
            .into_iter()
            .any(|(dx, dy)| self.count_aligned(x, y, side, dx, dy) >= 5)
    }

    fn count_aligned(&self, x: usize, y: usize, side: Side, dx: isize, dy: isize) -> usize {
        1 + self.count_one_side(x, y, side, dx, dy) + self.count_one_side(x, y, side, -dx, -dy)
    }

    fn count_one_side(&self, x: usize, y: usize, side: Side, dx: isize, dy: isize) -> usize {
        let mut count = 0;
        let mut cx = x as isize + dx;
        let mut cy = y as isize + dy;

        while cx >= 0
            && cy >= 0
            && (cx as usize) < BOARD_SIZE
            && (cy as usize) < BOARD_SIZE
            && self.grid[cy as usize][cx as usize] == side
        {
            count += 1;
            cx += dx;
            cy += dy;
        }

        count
    }
}
