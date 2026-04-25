//! Directional line extraction and pattern helpers.

use crate::board::Board;
use crate::constants::{BLACK, BOARD_SIZE, EMPTY, WHITE};
use crate::patterns::shape_table::shape_table_lookup;
use crate::patterns::shapes::{PackedShape, DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};

const SENTINEL: i32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternError {
    InvalidDirection(i32),
}

pub fn line_backend_name() -> &'static str {
    "python"
}

pub fn shape_raw_from_cells_python(cells: &[i32], point_index: usize, freestyle: bool) -> i32 {
    let p = point_index + 2;
    let stone = cells[p];
    if stone != i32::from(BLACK) && stone != i32::from(WHITE) {
        return 0;
    }

    let mut ssp = 0_i32;
    let mut si = 0_i32;
    let mut sj = 0_i32;
    let mut forward_blocked = false;
    let mut backward_blocked = false;

    let forward_masks = [16, 8, 4, 2, 1];
    let backward_masks = [32, 64, 128, 256, 512];

    let mut offset = 1_usize;
    while offset <= 5 {
        let mask = forward_masks[offset - 1];
        let value = cells[p + offset];
        if value == i32::from(EMPTY) {
            offset += 1;
            continue;
        }
        if value == stone {
            ssp |= mask;
        } else {
            sj = (offset - 1) as i32;
            forward_blocked = true;
            break;
        }
        offset += 1;
    }
    if !forward_blocked {
        sj = 5;
    }

    offset = 1;
    while offset <= 5 {
        let mask = backward_masks[offset - 1];
        let value = cells[p - offset];
        if value == i32::from(EMPTY) {
            offset += 1;
            continue;
        }
        if value == stone {
            ssp |= mask;
        } else {
            si = (offset - 1) as i32;
            backward_blocked = true;
            break;
        }
        offset += 1;
    }
    if !backward_blocked {
        si = 5;
    }

    ssp >>= 5 - sj;
    let table_index = (1 << si) * ((1 << sj) + 62) - 63 + ssp;
    let row = if stone == i32::from(BLACK) && !freestyle {
        1
    } else {
        0
    };
    let trt = shape_table_lookup(row as usize, table_index as usize);
    ((trt & 0xF0) << 12) | (trt & 0xF)
}

pub fn shape_raw_from_board_python(
    board: &Board,
    pivot: usize,
    direction: i32,
    point_index: usize,
    freestyle: bool,
) -> Result<i32, PatternError> {
    let mut cells = [SENTINEL; BOARD_SIZE + 4];
    fill_cells_python(
        &mut cells,
        board.grid_rows(),
        board.size(),
        pivot,
        direction,
    )?;
    Ok(shape_raw_from_cells_python(
        &cells[..board.size() + 4],
        point_index,
        freestyle,
    ))
}

pub(crate) fn shape_raw_from_board_point_python(
    board: &Board,
    pivot: usize,
    direction: i32,
    point_index: usize,
    freestyle: bool,
) -> Result<i32, PatternError> {
    let grid = board.grid_rows();
    let size = board.size();
    match direction {
        HORIZONTAL => Ok(shape_raw_horizontal_point(
            grid,
            size,
            pivot,
            point_index,
            freestyle,
        )),
        VERTICAL => Ok(shape_raw_vertical_point(
            grid,
            size,
            pivot,
            point_index,
            freestyle,
        )),
        DIAGONAL_DOWN => {
            let r = diagonal_index_range(size, pivot);
            Ok(shape_raw_diagonal_down_point(
                grid,
                size,
                pivot,
                point_index,
                freestyle,
                r.start,
                r.end,
            ))
        }
        DIAGONAL_UP => {
            let r = diagonal_index_range(size, pivot);
            Ok(shape_raw_diagonal_up_point(
                grid,
                size,
                pivot,
                point_index,
                freestyle,
                r.start,
                r.end,
            ))
        }
        _ => Err(PatternError::InvalidDirection(direction)),
    }
}

#[inline(always)]
fn shape_raw_horizontal_point(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    size: usize,
    pivot: usize,
    point_index: usize,
    freestyle: bool,
) -> i32 {
    shape_raw_from_logical_indices(size, point_index, freestyle, |i| i32::from(grid[i][pivot]))
}

#[inline(always)]
fn shape_raw_vertical_point(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    size: usize,
    pivot: usize,
    point_index: usize,
    freestyle: bool,
) -> i32 {
    shape_raw_from_logical_indices(size, point_index, freestyle, |i| i32::from(grid[pivot][i]))
}

#[inline(always)]
fn shape_raw_diagonal_down_point(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    size: usize,
    pivot: usize,
    point_index: usize,
    freestyle: bool,
    lo: usize,
    hi: usize,
) -> i32 {
    shape_raw_from_logical_indices(size, point_index, freestyle, |i| {
        if i >= lo && i < hi {
            i32::from(grid[i][pivot - i])
        } else {
            SENTINEL
        }
    })
}

#[inline(always)]
fn shape_raw_diagonal_up_point(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    size: usize,
    pivot: usize,
    point_index: usize,
    freestyle: bool,
    lo: usize,
    hi: usize,
) -> i32 {
    shape_raw_from_logical_indices(size, point_index, freestyle, |i| {
        if i >= lo && i < hi {
            i32::from(grid[size - 1 - i][pivot - i])
        } else {
            SENTINEL
        }
    })
}

#[inline(always)]
fn shape_raw_from_logical_indices<F>(
    size: usize,
    point_index: usize,
    freestyle: bool,
    mut cell_at_index: F,
) -> i32
where
    F: FnMut(usize) -> i32,
{
    debug_assert!(point_index < size);
    let stone = if point_index < size {
        cell_at_index(point_index)
    } else {
        SENTINEL
    };
    if stone != i32::from(BLACK) && stone != i32::from(WHITE) {
        return 0;
    }

    let mut ssp = 0_i32;
    let mut si = 0_i32;
    let mut sj = 0_i32;
    let mut forward_blocked = false;
    let mut backward_blocked = false;

    let forward_masks = [16, 8, 4, 2, 1];
    let backward_masks = [32, 64, 128, 256, 512];

    let mut offset = 1_usize;
    while offset <= 5 {
        let mask = forward_masks[offset - 1];
        let i = point_index + offset;
        let value = if i < size { cell_at_index(i) } else { SENTINEL };
        if value == i32::from(EMPTY) {
            offset += 1;
            continue;
        }
        if value == stone {
            ssp |= mask;
        } else {
            sj = (offset - 1) as i32;
            forward_blocked = true;
            break;
        }
        offset += 1;
    }
    if !forward_blocked {
        sj = 5;
    }

    offset = 1;
    while offset <= 5 {
        let mask = backward_masks[offset - 1];
        let value = point_index
            .checked_sub(offset)
            .map_or(SENTINEL, &mut cell_at_index);
        if value == i32::from(EMPTY) {
            offset += 1;
            continue;
        }
        if value == stone {
            ssp |= mask;
        } else {
            si = (offset - 1) as i32;
            backward_blocked = true;
            break;
        }
        offset += 1;
    }
    if !backward_blocked {
        si = 5;
    }

    ssp >>= 5 - sj;
    let table_index = (1 << si) * ((1 << sj) + 62) - 63 + ssp;
    let row = if stone == i32::from(BLACK) && !freestyle {
        1
    } else {
        0
    };
    let trt = shape_table_lookup(row as usize, table_index as usize);
    ((trt & 0xF0) << 12) | (trt & 0xF)
}

fn diagonal_index_range(size: usize, pivot: usize) -> std::ops::Range<usize> {
    if pivot < size {
        0..pivot + 1
    } else {
        pivot - size + 1..size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_shape_reader_matches_full_line_extraction_on_varied_boards() {
        let boards = [
            vec![
                (7, 7, BLACK),
                (8, 7, WHITE),
                (6, 8, BLACK),
                (9, 6, WHITE),
                (3, 11, BLACK),
                (11, 3, WHITE),
            ],
            vec![
                (0, 0, BLACK),
                (1, 0, BLACK),
                (2, 0, WHITE),
                (14, 14, WHITE),
                (13, 14, BLACK),
                (12, 13, WHITE),
                (0, 14, BLACK),
                (14, 0, WHITE),
            ],
            vec![
                (4, 4, BLACK),
                (5, 5, BLACK),
                (6, 6, EMPTY),
                (7, 7, WHITE),
                (8, 8, BLACK),
                (10, 4, WHITE),
                (9, 5, WHITE),
                (8, 6, BLACK),
                (6, 8, WHITE),
            ],
        ];

        for stones in boards {
            let mut board = Board::new();
            for (x, y, side) in stones {
                if side != EMPTY {
                    board.grid_rows_mut()[y][x] = side;
                }
            }

            for x in 0..BOARD_SIZE {
                for y in 0..BOARD_SIZE {
                    if board.grid_rows()[y][x] != EMPTY {
                        continue;
                    }
                    for side in [BLACK, WHITE] {
                        board.grid_rows_mut()[y][x] = side;
                        for freestyle in [true, false] {
                            for direction in [HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP] {
                                let (pivot, point_index) = match direction {
                                    HORIZONTAL => (x, y),
                                    VERTICAL => (y, x),
                                    DIAGONAL_DOWN => (x + y, y),
                                    DIAGONAL_UP => (BOARD_SIZE - 1 - y + x, BOARD_SIZE - 1 - y),
                                    _ => unreachable!(),
                                };
                                let full = shape_raw_from_board_python(
                                    &board,
                                    pivot,
                                    direction,
                                    point_index,
                                    freestyle,
                                )
                                .unwrap();
                                let point = shape_raw_from_board_point_python(
                                    &board,
                                    pivot,
                                    direction,
                                    point_index,
                                    freestyle,
                                )
                                .unwrap();
                                assert_eq!(
                                    point, full,
                                    "x={x} y={y} side={side} direction={direction} freestyle={freestyle}"
                                );
                            }
                        }
                        board.grid_rows_mut()[y][x] = EMPTY;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub cells: Vec<i32>,
}

impl Line {
    pub fn from_board(board: &Board, pivot: usize, direction: i32) -> Result<Self, PatternError> {
        Ok(Self {
            cells: extract_cells_python(board.grid_rows(), board.size(), pivot, direction)?,
        })
    }

    pub fn shape(&self, point_index: usize, freestyle: bool) -> PackedShape {
        PackedShape {
            raw: self.shape_raw(point_index, freestyle),
        }
    }

    pub fn shape_raw(&self, point_index: usize, freestyle: bool) -> i32 {
        shape_raw_from_cells_python(&self.cells, point_index, freestyle)
    }

    pub fn a3pb(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(3));
        let xmax = usize::min(BOARD_SIZE - 2, p);
        for i in xmin..=xmax {
            let num1 = self.cells[i] + self.cells[i + 1] + self.cells[i + 2] + self.cells[i + 3];
            let num2 = self.cells[i] * self.cells[i + 1] * self.cells[i + 2] * self.cells[i + 3];
            if num1 != 3 * x0 || num2 != 0 {
                continue;
            }
            let mut shape = (self.cells[i] << 3)
                + (self.cells[i + 1] << 2)
                + (self.cells[i + 2] << 1)
                + self.cells[i + 3];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x0E {
                if self.cells[i - 1] == i32::from(EMPTY)
                    && self.cells[i - 2] != x0
                    && self.cells[i + 4] != x0
                {
                    if self.cells[i - 2] == i32::from(EMPTY)
                        && self.cells[i + 4] == i32::from(EMPTY)
                    {
                        return comc(1, i - 1, i + 3);
                    }
                    if self.cells[i - 2] == i32::from(EMPTY) {
                        return comd(1, i - 1, i - 2, i + 3);
                    }
                    if self.cells[i + 4] == i32::from(EMPTY) {
                        return comd(1, i + 3, i - 1, i + 4);
                    }
                }
            }
            if shape == 0x0D
                && self.cells[i - 1] == i32::from(EMPTY)
                && self.cells[i + 4] == i32::from(EMPTY)
            {
                return comd(1, i + 2, i - 1, i + 4);
            }
            if shape == 0x0B
                && self.cells[i - 1] == i32::from(EMPTY)
                && self.cells[i + 4] == i32::from(EMPTY)
            {
                return comd(1, i + 1, i - 1, i + 4);
            }
        }
        0
    }

    pub fn a4(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(3));
        let xmax = usize::min(BOARD_SIZE - 2, p);
        for i in xmin..=xmax {
            if self.cells[i] + self.cells[i + 1] + self.cells[i + 2] + self.cells[i + 3] != 4 * x0 {
                continue;
            }
            if self.cells[i - 1] == i32::from(EMPTY) && self.cells[i + 4] == i32::from(EMPTY) {
                return 1;
            }
        }
        0
    }

    pub fn a6(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        if self.cells[p] != i32::from(BLACK) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(5));
        let xmax = usize::min(BOARD_SIZE - 4, p);
        for i in xmin..=xmax {
            if self.cells[i]
                + self.cells[i + 1]
                + self.cells[i + 2]
                + self.cells[i + 3]
                + self.cells[i + 4]
                + self.cells[i + 5]
                == 6
            {
                return 1;
            }
        }
        0
    }

    pub fn a5(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(4));
        let xmax = usize::min(BOARD_SIZE - 3, p);
        for i in xmin..=xmax {
            if self.cells[i]
                + self.cells[i + 1]
                + self.cells[i + 2]
                + self.cells[i + 3]
                + self.cells[i + 4]
                == 5 * x0
            {
                return 1;
            }
        }
        0
    }

    pub fn b4(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(4));
        let xmax = usize::min(BOARD_SIZE - 3, p);
        for i in xmin..=xmax {
            if (0..5).map(|k| self.cells[i + k]).sum::<i32>() != 4 * x0 {
                continue;
            }
            let mut shape = (self.cells[i] << 4)
                + (self.cells[i + 1] << 3)
                + (self.cells[i + 2] << 2)
                + (self.cells[i + 3] << 1)
                + self.cells[i + 4];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x1E || shape == 0x0F {
                return 1;
            }
            if shape == 0x1D {
                if i <= BOARD_SIZE - 7
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && self.cells[i + 8] == x0
                    && p == i + 4
                {
                    return 2;
                }
                return 1;
            }
            if shape == 0x1B {
                if i <= BOARD_SIZE - 6
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && (p == i + 4 || p == i + 3)
                {
                    return 2;
                }
                return 1;
            }
            if shape == 0x17 {
                if i <= BOARD_SIZE - 5
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && (p == i + 4 || p == i + 3 || p == i + 2)
                {
                    return 2;
                }
                return 1;
            }
        }
        0
    }

    pub fn b4p(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(4));
        let xmax = usize::min(BOARD_SIZE - 3, p);
        for i in xmin..=xmax {
            if (0..5).map(|k| self.cells[i + k]).sum::<i32>() != 4 * x0 {
                continue;
            }
            let mut shape = (self.cells[i] << 4)
                + (self.cells[i + 1] << 3)
                + (self.cells[i + 2] << 2)
                + (self.cells[i + 3] << 1)
                + self.cells[i + 4];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x1E {
                if self.cells[i - 1] == i32::from(EMPTY) {
                    return comc(1, i - 1, i + 4);
                }
                return comb(1, i + 4);
            }
            if shape == 0x1D {
                if i <= BOARD_SIZE - 7
                    && self.cells[i + 5] == x0
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && p == i + 4
                    && self.cells[i + 3] == i32::from(EMPTY)
                {
                    return comc(1, i + 3, i + 5);
                }
                if self.cells[i + 3] == i32::from(EMPTY) {
                    return comb(1, i + 3);
                }
            }
            if shape == 0x1B {
                if i <= BOARD_SIZE - 6
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && self.cells[i + 7] == x0
                    && (p == i + 4 || p == i + 3)
                    && self.cells[i + 2] == i32::from(EMPTY)
                {
                    return comc(1, i + 2, i + 5);
                }
                if self.cells[i + 2] == i32::from(EMPTY) {
                    return comb(1, i + 2);
                }
            }
            if shape == 0x17 {
                if i <= BOARD_SIZE - 5
                    && self.cells[i + 5] == i32::from(EMPTY)
                    && self.cells[i + 6] == x0
                    && (p == i + 4 || p == i + 3 || p == i + 2)
                    && self.cells[i + 1] == i32::from(EMPTY)
                {
                    return comc(1, i + 1, i + 5);
                }
                if self.cells[i + 1] == i32::from(EMPTY) {
                    return comb(1, i + 1);
                }
            }
            if shape == 0x0F {
                if self.cells[i + 5] == i32::from(EMPTY) {
                    return comc(1, i, i + 5);
                }
                return comb(1, i);
            }
        }
        0
    }

    pub fn a3(&self, point_index: usize) -> i32 {
        let p = point_index + 2;
        let x0 = self.cells[p];
        if x0 == i32::from(EMPTY) {
            return 0;
        }
        let xmin = usize::max(2, p.saturating_sub(3));
        let xmax = usize::min(BOARD_SIZE - 2, p);
        for i in xmin..=xmax {
            let num1 = self.cells[i] + self.cells[i + 1] + self.cells[i + 2] + self.cells[i + 3];
            let num2 = self.cells[i] * self.cells[i + 1] * self.cells[i + 2] * self.cells[i + 3];
            if num1 != 3 * x0 || num2 != 0 {
                continue;
            }
            let mut shape = (self.cells[i] << 3)
                + (self.cells[i + 1] << 2)
                + (self.cells[i + 2] << 1)
                + self.cells[i + 3];
            if x0 == i32::from(WHITE) {
                shape = -shape;
            }
            if shape == 0x0E {
                if self.cells[i - 1] == i32::from(EMPTY)
                    && self.cells[i - 2] != x0
                    && self.cells[i + 4] != x0
                {
                    if self.cells[i - 2] == i32::from(EMPTY)
                        && self.cells[i + 4] == i32::from(EMPTY)
                    {
                        return comc(1, i - 1, i + 3);
                    }
                    if self.cells[i - 2] == i32::from(EMPTY) {
                        return comb(1, i - 1);
                    }
                    if self.cells[i + 4] == i32::from(EMPTY) {
                        return comb(1, i + 3);
                    }
                }
            }
            if shape == 0x0D
                && self.cells[i - 1] == i32::from(EMPTY)
                && self.cells[i + 4] == i32::from(EMPTY)
            {
                return comb(1, i + 2);
            }
            if shape == 0x0B
                && self.cells[i - 1] == i32::from(EMPTY)
                && self.cells[i + 4] == i32::from(EMPTY)
            {
                return comb(1, i + 1);
            }
        }
        0
    }
}

fn extract_cells_python(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    size: usize,
    pivot: usize,
    direction: i32,
) -> Result<Vec<i32>, PatternError> {
    let mut cells = vec![SENTINEL; size + 4];
    fill_cells_python(&mut cells, grid, size, pivot, direction)?;
    Ok(cells)
}

fn fill_cells_python(
    cells: &mut [i32],
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    size: usize,
    pivot: usize,
    direction: i32,
) -> Result<(), PatternError> {
    match direction {
        HORIZONTAL => {
            for y in 0..size {
                cells[y + 2] = i32::from(grid[y][pivot]);
            }
        }
        VERTICAL => {
            let row = &grid[pivot];
            for x in 0..size {
                cells[x + 2] = i32::from(row[x]);
            }
        }
        DIAGONAL_DOWN => {
            if pivot < size {
                for i in 0..=pivot {
                    cells[i + 2] = i32::from(grid[i][pivot - i]);
                }
            } else {
                let start = pivot - size + 1;
                for i in start..size {
                    cells[i + 2] = i32::from(grid[i][pivot - i]);
                }
            }
        }
        DIAGONAL_UP => {
            if pivot < size {
                for i in 0..=pivot {
                    cells[i + 2] = i32::from(grid[size - 1 - i][pivot - i]);
                }
            } else {
                let start = pivot - size + 1;
                for i in start..size {
                    cells[i + 2] = i32::from(grid[size - 1 - i][pivot - i]);
                }
            }
        }
        _ => {
            return Err(PatternError::InvalidDirection(direction));
        }
    }
    Ok(())
}

fn comb(x: usize, y: usize) -> i32 {
    ((x as i32) << 8) | (y as i32 - 2)
}

fn comc(x: usize, y: usize, z: usize) -> i32 {
    comb(comb(x, y) as usize, z)
}

fn comd(x: usize, y: usize, z: usize, w: usize) -> i32 {
    comb(comc(x, y, z) as usize, w)
}
