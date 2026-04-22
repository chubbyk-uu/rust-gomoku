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

    for (offset, mask) in (1..=5).zip(forward_masks) {
        let value = cells[p + offset];
        if value == i32::from(EMPTY) {
            continue;
        }
        if value == stone {
            ssp |= mask;
        } else {
            sj = (offset - 1) as i32;
            forward_blocked = true;
            break;
        }
    }
    if !forward_blocked {
        sj = 5;
    }

    for (offset, mask) in (1..=5).zip(backward_masks) {
        let value = cells[p - offset];
        if value == i32::from(EMPTY) {
            continue;
        }
        if value == stone {
            ssp |= mask;
        } else {
            si = (offset - 1) as i32;
            backward_blocked = true;
            break;
        }
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
    Ok(cells)
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
