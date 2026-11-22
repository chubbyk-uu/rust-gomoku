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

#[cfg(test)]
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

pub(crate) fn shape_raw_from_board_point_hypothetical(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    direction: i32,
    side: i8,
    freestyle: bool,
) -> Result<i32, PatternError> {
    match direction {
        HORIZONTAL => Ok(shape_raw_from_hypothetical_offsets(
            grid, x, y, 0, 1, side, freestyle,
        )),
        VERTICAL => Ok(shape_raw_from_hypothetical_offsets(
            grid, x, y, 1, 0, side, freestyle,
        )),
        DIAGONAL_DOWN => Ok(shape_raw_from_hypothetical_offsets(
            grid, x, y, -1, 1, side, freestyle,
        )),
        DIAGONAL_UP => Ok(shape_raw_from_hypothetical_offsets(
            grid, x, y, -1, -1, side, freestyle,
        )),
        _ => Err(PatternError::InvalidDirection(direction)),
    }
}

/// Maps a 5-bit forward own-stone mask to the `ssp` bits the old walk produced:
/// a stone at forward offset `k+1` contributed `16 >> k`, i.e. bit `k` maps to
/// bit `4 - k`. Precomputed so the hot reader avoids a per-bit loop.
const REVERSE5: [i32; 32] = {
    let mut table = [0_i32; 32];
    let mut mask = 0_usize;
    while mask < 32 {
        let mut reversed = 0_i32;
        let mut k = 0;
        while k < 5 {
            if mask & (1 << k) != 0 {
                reversed |= 1 << (4 - k);
            }
            k += 1;
        }
        table[mask] = reversed;
        mask += 1;
    }
    table
};

/// Build the blocker and own-stone bitmasks over offsets `1..=5` from `(x, y)`
/// along `(dx, dy)` with `dx, dy ∈ {-1, 0, 1}`. Bit `k` corresponds to offset
/// `k + 1`. An off-board square is a blocker (never EMPTY nor a stone), matching
/// the old `SENTINEL` walk. The fixed 5-iteration loop has no data-dependent
/// early-exit, so the result is computed branchlessly from the masks.
#[inline(always)]
fn directional_masks(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    dx: isize,
    dy: isize,
    stone: i32,
) -> (u32, u32) {
    let mut blocker = 0_u32;
    let mut own = 0_u32;
    for k in 0..5_isize {
        let offset = k + 1;
        let xx = x as isize + dx * offset;
        let yy = y as isize + dy * offset;
        let on_board =
            xx >= 0 && yy >= 0 && (xx as usize) < BOARD_SIZE && (yy as usize) < BOARD_SIZE;
        let value = if on_board {
            i32::from(grid[yy as usize][xx as usize])
        } else {
            SENTINEL
        };
        let bit = 1_u32 << k;
        own |= u32::from(value == stone) << k;
        blocker |= u32::from(value != stone && value != i32::from(EMPTY)) * bit;
    }
    (blocker, own)
}

#[inline(always)]
fn shape_raw_from_hypothetical_offsets(
    grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    dx: isize,
    dy: isize,
    side: i8,
    freestyle: bool,
) -> i32 {
    let stone = i32::from(side);
    if stone != i32::from(BLACK) && stone != i32::from(WHITE) {
        return 0;
    }

    // Forward (dx, dy) and backward (-dx, -dy) blocker/own masks over offsets
    // 1..=5. `sj`/`si` are the distance to the first blocker (capped at 5 when
    // none); `ssp` collects own stones before that blocker, mapped to the same
    // bit positions the old walk used (forward reversed via REVERSE5, backward
    // shifted up by 5).
    let (blocker_fwd, own_fwd) = directional_masks(grid, x, y, dx, dy, stone);
    let (blocker_bwd, own_bwd) = directional_masks(grid, x, y, -dx, -dy, stone);

    let row = usize::from(side == BLACK && !freestyle);
    shape_raw_from_masks(blocker_fwd, own_fwd, blocker_bwd, own_bwd, row)
}

#[inline(always)]
fn shape_raw_from_masks(
    blocker_fwd: u32,
    own_fwd: u32,
    blocker_bwd: u32,
    own_bwd: u32,
    row: usize,
) -> i32 {
    let sj = blocker_fwd.trailing_zeros().min(5);
    let si = blocker_bwd.trailing_zeros().min(5);
    let fwd_ssp = REVERSE5[(own_fwd & ((1_u32 << sj) - 1)) as usize];
    let bwd_ssp = ((own_bwd & ((1_u32 << si) - 1)) as i32) << 5;
    let mut ssp = fwd_ssp | bwd_ssp;
    ssp >>= 5 - sj;

    let table_index = (1 << si) * ((1 << sj) + 62) - 63 + ssp;
    let trt = shape_table_lookup(row, table_index as usize);
    ((trt & 0xF0) << 12) | (trt & 0xF)
}

pub(crate) const PACKED_LINE_COUNT: usize = 88;

const fn build_packed_on_board_masks() -> [u32; PACKED_LINE_COUNT] {
    let mut masks = [0_u32; PACKED_LINE_COUNT];
    let full = ((1_u32 << BOARD_SIZE) - 1) << 5;
    let mut i = 0;
    while i < BOARD_SIZE {
        masks[i] = full;
        masks[BOARD_SIZE + i] = full;
        i += 1;
    }
    let mut d = 0;
    while d < BOARD_SIZE * 2 - 1 {
        let lo = if d >= BOARD_SIZE {
            d + 1 - BOARD_SIZE
        } else {
            0
        };
        let hi = if d < BOARD_SIZE { d } else { BOARD_SIZE - 1 };
        let len = hi - lo + 1;
        let mask = ((1_u32 << len) - 1) << (lo + 5);
        masks[30 + d] = mask;
        masks[59 + d] = mask;
        d += 1;
    }
    masks
}

pub(crate) const PACKED_ON_BOARD_MASKS: [u32; PACKED_LINE_COUNT] = build_packed_on_board_masks();

/// Masks for one of the four lines intersecting a changed cell. Five padding
/// bits on each end let a shift extract any cell's complete radius-five window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackedLine {
    stones: [u32; 2],
    on_board: u32,
}

impl PackedLine {
    #[inline(always)]
    pub(crate) fn from_stones(stones: [u32; 2], line_idx: usize) -> Self {
        Self {
            stones,
            on_board: PACKED_ON_BOARD_MASKS[line_idx],
        }
    }

    #[cfg(any(debug_assertions, test))]
    #[inline(always)]
    pub(crate) fn through(
        grid: &[[i8; BOARD_SIZE]; BOARD_SIZE],
        x: usize,
        y: usize,
        dx: isize,
        dy: isize,
    ) -> Self {
        // Line indices agree with the engine's directional pattern coordinates.
        let anchor = match dy {
            1 => y,
            -1 => BOARD_SIZE - 1 - y,
            _ => x,
        } as isize;
        let origin_x = x as isize - anchor * dx;
        let origin_y = y as isize - anchor * dy;
        let mut line = Self {
            stones: [0; 2],
            on_board: 0,
        };
        for i in 0..BOARD_SIZE {
            let xx = (origin_x + i as isize * dx) as usize;
            let yy = (origin_y + i as isize * dy) as usize;
            if xx < BOARD_SIZE && yy < BOARD_SIZE {
                let bit = 1_u32 << (i + 5);
                let cell = grid[yy][xx];
                line.stones[0] |= u32::from(cell == BLACK) << (i + 5);
                line.stones[1] |= u32::from(cell == WHITE) << (i + 5);
                line.on_board |= bit;
            }
        }
        line
    }

    #[inline(always)]
    pub(crate) fn neighbor_counts(&self, index: usize, radius: u32) -> (u32, u32) {
        // The second color encountered stops the original dirty-point scan.
        // Off-board bits stop it at the edge, independently of stone colors.
        let forward = |mask: u32| (mask >> (index + 6)).trailing_zeros();
        let backward = |mask: u32| ((mask >> index) & 31).leading_zeros() - 27;
        let forward_count = forward(self.stones[0])
            .max(forward(self.stones[1]))
            .min(forward(!self.on_board))
            .min(radius);
        let backward_count = backward(self.stones[0])
            .max(backward(self.stones[1]))
            .min(backward(!self.on_board))
            .min(radius);
        (forward_count, backward_count)
    }

    #[inline(always)]
    pub(crate) fn hypothetical_shapes(&self, index: usize, renju: bool) -> [i32; 2] {
        let black_forward = self.stones[0] >> (index + 6) & 31;
        let white_forward = self.stones[1] >> (index + 6) & 31;
        let edge_forward = !self.on_board >> (index + 6) & 31;
        let black_backward = REVERSE5[(self.stones[0] >> index & 31) as usize] as u32;
        let white_backward = REVERSE5[(self.stones[1] >> index & 31) as usize] as u32;
        let edge_backward = REVERSE5[(!self.on_board >> index & 31) as usize] as u32;
        [
            shape_raw_from_masks(
                white_forward | edge_forward,
                black_forward,
                white_backward | edge_backward,
                black_backward,
                usize::from(renju),
            ),
            shape_raw_from_masks(
                black_forward | edge_forward,
                white_forward,
                black_backward | edge_backward,
                white_backward,
                0,
            ),
        ]
    }
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
                        for freestyle in [true, false] {
                            for direction in [HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP] {
                                let hypothetical = shape_raw_from_board_point_hypothetical(
                                    board.grid_rows(),
                                    x,
                                    y,
                                    direction,
                                    side,
                                    freestyle,
                                )
                                .unwrap();
                                let (pivot, point_index) = match direction {
                                    HORIZONTAL => (x, y),
                                    VERTICAL => (y, x),
                                    DIAGONAL_DOWN => (x + y, y),
                                    DIAGONAL_UP => (BOARD_SIZE - 1 - y + x, BOARD_SIZE - 1 - y),
                                    _ => unreachable!(),
                                };
                                board.grid_rows_mut()[y][x] = side;
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
                                board.grid_rows_mut()[y][x] = EMPTY;
                                assert_eq!(
                                    point, full,
                                    "x={x} y={y} side={side} direction={direction} freestyle={freestyle}"
                                );
                                assert_eq!(
                                    hypothetical, full,
                                    "x={x} y={y} side={side} direction={direction} freestyle={freestyle}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hypothetical_reader_matches_full_line_on_random_and_edge_boards() {
        // Stage 2 gate: the branchless `directional_masks` reader must equal the
        // full-line reference for every empty point, direction, side, and rule,
        // across many random boards (some very dense so windows are blocker- and
        // gap-rich, exercising the bitmask `trailing_zeros`/REVERSE5 paths) plus
        // saturated cells against every edge and corner.
        let mut state = 0xC0FF_EE15_600D_1234_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for trial in 0..120_u64 {
            let mut board = Board::new();
            let dense = trial % 4 == 0;
            for y in 0..BOARD_SIZE {
                for x in 0..BOARD_SIZE {
                    let roll = next() % if dense { 3 } else { 5 };
                    board.grid_rows_mut()[y][x] = match roll {
                        0 => BLACK,
                        1 => WHITE,
                        _ => EMPTY,
                    };
                }
            }

            for y in 0..BOARD_SIZE {
                for x in 0..BOARD_SIZE {
                    if board.grid_rows()[y][x] != EMPTY {
                        continue;
                    }
                    for side in [BLACK, WHITE] {
                        for freestyle in [true, false] {
                            for direction in [HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP] {
                                let hypothetical = shape_raw_from_board_point_hypothetical(
                                    board.grid_rows(),
                                    x,
                                    y,
                                    direction,
                                    side,
                                    freestyle,
                                )
                                .unwrap();
                                let (pivot, point_index) = match direction {
                                    HORIZONTAL => (x, y),
                                    VERTICAL => (y, x),
                                    DIAGONAL_DOWN => (x + y, y),
                                    DIAGONAL_UP => (BOARD_SIZE - 1 - y + x, BOARD_SIZE - 1 - y),
                                    _ => unreachable!(),
                                };
                                let (dx, dy) = match direction {
                                    HORIZONTAL => (0, 1),
                                    VERTICAL => (1, 0),
                                    DIAGONAL_DOWN => (-1, 1),
                                    DIAGONAL_UP => (-1, -1),
                                    _ => unreachable!(),
                                };
                                let line = PackedLine::through(board.grid_rows(), x, y, dx, dy);
                                let shapes = line.hypothetical_shapes(point_index, !freestyle);
                                assert_eq!(shapes[usize::from(side == WHITE)], hypothetical);
                                board.grid_rows_mut()[y][x] = side;
                                let full = shape_raw_from_board_python(
                                    &board,
                                    pivot,
                                    direction,
                                    point_index,
                                    freestyle,
                                )
                                .unwrap();
                                board.grid_rows_mut()[y][x] = EMPTY;
                                assert_eq!(
                                    hypothetical, full,
                                    "trial={trial} x={x} y={y} side={side} dir={direction} fs={freestyle}"
                                );
                            }
                        }
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
        scan_a4(&self.cells, point_index)
    }

    pub fn a6(&self, point_index: usize) -> i32 {
        scan_a6(&self.cells, point_index)
    }

    pub fn a5(&self, point_index: usize) -> i32 {
        scan_a5(&self.cells, point_index)
    }

    pub fn b4(&self, point_index: usize) -> i32 {
        scan_b4(&self.cells, point_index)
    }

    pub fn b4p(&self, point_index: usize) -> i32 {
        scan_b4p(&self.cells, point_index)
    }

    pub fn a3(&self, point_index: usize) -> i32 {
        scan_a3(&self.cells, point_index)
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

pub(crate) fn comb(x: usize, y: usize) -> i32 {
    ((x as i32) << 8) | (y as i32 - 2)
}

pub(crate) fn comc(x: usize, y: usize, z: usize) -> i32 {
    comb(comb(x, y) as usize, z)
}

fn comd(x: usize, y: usize, z: usize, w: usize) -> i32 {
    comb(comc(x, y, z) as usize, w)
}

// Shared directional-line shape scanners over a padded `&[i32]` cell buffer
// (two SENTINEL cells on each side; `point_index + 2` addresses the anchor).
// `Line` (Vec-backed, extracted from a board) and `ThreatLine` (fixed-array,
// incrementally maintained) both delegate here so the encoding lives once.

pub(crate) fn scan_a4(cells: &[i32], point_index: usize) -> i32 {
    let p = point_index + 2;
    let x0 = cells[p];
    if x0 == i32::from(EMPTY) {
        return 0;
    }
    let xmin = usize::max(2, p.saturating_sub(3));
    let xmax = usize::min(BOARD_SIZE - 2, p);
    for i in xmin..=xmax {
        if cells[i] + cells[i + 1] + cells[i + 2] + cells[i + 3] != 4 * x0 {
            continue;
        }
        if cells[i - 1] == i32::from(EMPTY) && cells[i + 4] == i32::from(EMPTY) {
            return 1;
        }
    }
    0
}

pub(crate) fn scan_a6(cells: &[i32], point_index: usize) -> i32 {
    let p = point_index + 2;
    if cells[p] != i32::from(BLACK) {
        return 0;
    }
    let xmin = usize::max(2, p.saturating_sub(5));
    let xmax = usize::min(BOARD_SIZE - 4, p);
    for i in xmin..=xmax {
        if cells[i] + cells[i + 1] + cells[i + 2] + cells[i + 3] + cells[i + 4] + cells[i + 5] == 6
        {
            return 1;
        }
    }
    0
}

pub(crate) fn scan_a5(cells: &[i32], point_index: usize) -> i32 {
    let p = point_index + 2;
    let x0 = cells[p];
    if x0 == i32::from(EMPTY) {
        return 0;
    }
    let xmin = usize::max(2, p.saturating_sub(4));
    let xmax = usize::min(BOARD_SIZE - 3, p);
    for i in xmin..=xmax {
        if cells[i] + cells[i + 1] + cells[i + 2] + cells[i + 3] + cells[i + 4] == 5 * x0 {
            return 1;
        }
    }
    0
}

pub(crate) fn scan_b4(cells: &[i32], point_index: usize) -> i32 {
    let p = point_index + 2;
    let x0 = cells[p];
    if x0 == i32::from(EMPTY) {
        return 0;
    }
    let xmin = usize::max(2, p.saturating_sub(4));
    let xmax = usize::min(BOARD_SIZE - 3, p);
    for i in xmin..=xmax {
        if (0..5).map(|k| cells[i + k]).sum::<i32>() != 4 * x0 {
            continue;
        }
        let mut shape = (cells[i] << 4)
            + (cells[i + 1] << 3)
            + (cells[i + 2] << 2)
            + (cells[i + 3] << 1)
            + cells[i + 4];
        if x0 == i32::from(WHITE) {
            shape = -shape;
        }
        if shape == 0x1E || shape == 0x0F {
            return 1;
        }
        if shape == 0x1D {
            if i <= BOARD_SIZE - 7
                && cells[i + 5] == i32::from(EMPTY)
                && cells[i + 6] == x0
                && cells[i + 7] == x0
                && cells[i + 8] == x0
                && p == i + 4
            {
                return 2;
            }
            return 1;
        }
        if shape == 0x1B {
            if i <= BOARD_SIZE - 6
                && cells[i + 5] == i32::from(EMPTY)
                && cells[i + 6] == x0
                && cells[i + 7] == x0
                && (p == i + 4 || p == i + 3)
            {
                return 2;
            }
            return 1;
        }
        if shape == 0x17 {
            if i <= BOARD_SIZE - 5
                && cells[i + 5] == i32::from(EMPTY)
                && cells[i + 6] == x0
                && (p == i + 4 || p == i + 3 || p == i + 2)
            {
                return 2;
            }
            return 1;
        }
    }
    0
}

pub(crate) fn scan_b4p(cells: &[i32], point_index: usize) -> i32 {
    let p = point_index + 2;
    let x0 = cells[p];
    if x0 == i32::from(EMPTY) {
        return 0;
    }
    let xmin = usize::max(2, p.saturating_sub(4));
    let xmax = usize::min(BOARD_SIZE - 3, p);
    for i in xmin..=xmax {
        if (0..5).map(|k| cells[i + k]).sum::<i32>() != 4 * x0 {
            continue;
        }
        let mut shape = (cells[i] << 4)
            + (cells[i + 1] << 3)
            + (cells[i + 2] << 2)
            + (cells[i + 3] << 1)
            + cells[i + 4];
        if x0 == i32::from(WHITE) {
            shape = -shape;
        }
        if shape == 0x1E {
            if cells[i - 1] == i32::from(EMPTY) {
                return comc(1, i - 1, i + 4);
            }
            return comb(1, i + 4);
        }
        if shape == 0x1D {
            if i <= BOARD_SIZE - 7
                && cells[i + 5] == i32::from(EMPTY)
                && cells[i + 6] == x0
                && cells[i + 7] == x0
                && cells[i + 8] == x0
                && p == i + 4
                && cells[i + 3] == i32::from(EMPTY)
            {
                return comc(1, i + 3, i + 5);
            }
            if cells[i + 3] == i32::from(EMPTY) {
                return comb(1, i + 3);
            }
        }
        if shape == 0x1B {
            if i <= BOARD_SIZE - 6
                && cells[i + 5] == i32::from(EMPTY)
                && cells[i + 6] == x0
                && cells[i + 7] == x0
                && (p == i + 4 || p == i + 3)
                && cells[i + 2] == i32::from(EMPTY)
            {
                return comc(1, i + 2, i + 5);
            }
            if cells[i + 2] == i32::from(EMPTY) {
                return comb(1, i + 2);
            }
        }
        if shape == 0x17 {
            if i <= BOARD_SIZE - 5
                && cells[i + 5] == i32::from(EMPTY)
                && cells[i + 6] == x0
                && (p == i + 4 || p == i + 3 || p == i + 2)
                && cells[i + 1] == i32::from(EMPTY)
            {
                return comc(1, i + 1, i + 5);
            }
            if cells[i + 1] == i32::from(EMPTY) {
                return comb(1, i + 1);
            }
        }
        if shape == 0x0F {
            if cells[i + 5] == i32::from(EMPTY) {
                return comc(1, i, i + 5);
            }
            return comb(1, i);
        }
    }
    0
}

pub(crate) fn scan_a3(cells: &[i32], point_index: usize) -> i32 {
    let p = point_index + 2;
    let x0 = cells[p];
    if x0 == i32::from(EMPTY) {
        return 0;
    }
    let xmin = usize::max(2, p.saturating_sub(3));
    let xmax = usize::min(BOARD_SIZE - 2, p);
    for i in xmin..=xmax {
        let num1 = cells[i] + cells[i + 1] + cells[i + 2] + cells[i + 3];
        let num2 = cells[i] * cells[i + 1] * cells[i + 2] * cells[i + 3];
        if num1 != 3 * x0 || num2 != 0 {
            continue;
        }
        let mut shape = (cells[i] << 3) + (cells[i + 1] << 2) + (cells[i + 2] << 1) + cells[i + 3];
        if x0 == i32::from(WHITE) {
            shape = -shape;
        }
        if shape == 0x0E {
            if cells[i - 1] == i32::from(EMPTY) && cells[i - 2] != x0 && cells[i + 4] != x0 {
                if cells[i - 2] == i32::from(EMPTY) && cells[i + 4] == i32::from(EMPTY) {
                    return comc(1, i - 1, i + 3);
                }
                if cells[i - 2] == i32::from(EMPTY) {
                    return comb(1, i - 1);
                }
                if cells[i + 4] == i32::from(EMPTY) {
                    return comb(1, i + 3);
                }
            }
        }
        if shape == 0x0D && cells[i - 1] == i32::from(EMPTY) && cells[i + 4] == i32::from(EMPTY) {
            return comb(1, i + 2);
        }
        if shape == 0x0B && cells[i - 1] == i32::from(EMPTY) && cells[i + 4] == i32::from(EMPTY) {
            return comb(1, i + 1);
        }
    }
    0
}
