//! Pure Renju forbidden-move detection.
//!
//! This module is intentionally detached from `Board::is_legal_move`, move
//! generation, and search. It classifies a hypothetical black move under Renju
//! rules while preserving the current freestyle engine behavior.

use crate::board::{move_to_xy, xy_to_move, Board, BoardError};
use crate::constants::{BLACK, BOARD_SIZE, EMPTY};
use crate::types::{is_valid_side, Move, Side};

const DIRECTIONS: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];
const NO_EXTRA_BLACK: usize = usize::MAX;

#[derive(Clone, Copy)]
struct DirectionalLine {
    cells: [Side; BOARD_SIZE],
    xs: [usize; BOARD_SIZE],
    ys: [usize; BOARD_SIZE],
    len: usize,
    anchor: usize,
}

impl DirectionalLine {
    fn from_grid(
        grid: &[[Side; BOARD_SIZE]; BOARD_SIZE],
        x: usize,
        y: usize,
        (dx, dy): (isize, isize),
    ) -> Self {
        let mut start_x = x;
        let mut start_y = y;
        while let Some((nx, ny)) = step(start_x, start_y, -dx, -dy) {
            start_x = nx;
            start_y = ny;
        }

        let mut cells = [EMPTY; BOARD_SIZE];
        let mut xs = [0; BOARD_SIZE];
        let mut ys = [0; BOARD_SIZE];
        let mut len = 0;
        let mut anchor = 0;
        let mut cx = start_x;
        let mut cy = start_y;
        loop {
            cells[len] = grid[cy][cx];
            xs[len] = cx;
            ys[len] = cy;
            if cx == x && cy == y {
                anchor = len;
            }
            len += 1;
            let Some((nx, ny)) = step(cx, cy, dx, dy) else {
                break;
            };
            cx = nx;
            cy = ny;
        }

        Self {
            cells,
            xs,
            ys,
            len,
            anchor,
        }
    }

    #[inline(always)]
    fn cell_with_extra(&self, index: usize, extra1: usize, extra2: usize) -> Side {
        if index == extra1 || index == extra2 {
            BLACK
        } else {
            self.cells[index]
        }
    }

    fn black_run_bounds_with_extra(
        &self,
        index: usize,
        extra1: usize,
        extra2: usize,
    ) -> Option<(usize, usize)> {
        if index >= self.len || self.cell_with_extra(index, extra1, extra2) != BLACK {
            return None;
        }
        let mut start = index;
        while start > 0 && self.cell_with_extra(start - 1, extra1, extra2) == BLACK {
            start -= 1;
        }
        let mut end = index;
        while end + 1 < self.len && self.cell_with_extra(end + 1, extra1, extra2) == BLACK {
            end += 1;
        }
        Some((start, end))
    }

    fn has_exact_five(&self) -> bool {
        self.black_run_bounds_with_extra(self.anchor, NO_EXTRA_BLACK, NO_EXTRA_BLACK)
            .is_some_and(|(start, end)| end - start + 1 == 5)
    }

    fn has_overline(&self) -> bool {
        self.black_run_bounds_with_extra(self.anchor, NO_EXTRA_BLACK, NO_EXTRA_BLACK)
            .is_some_and(|(start, end)| end - start + 1 >= 6)
    }

    fn count_four_shapes_through(&self) -> usize {
        if self.len < 5 {
            return 0;
        }
        let start_min = self.anchor.saturating_sub(4);
        let start_max = usize::min(self.anchor, self.len.saturating_sub(5));
        let mut shapes = [[usize::MAX; 4]; 5];
        let mut shape_count = 0;

        for start in start_min..=start_max {
            let end = start + 5;
            let mut empty = None;
            let mut black_count = 0;
            let mut black_shape = [usize::MAX; 4];
            let mut valid_window = true;

            for position in start..end {
                match self.cells[position] {
                    BLACK => {
                        if black_count < 4 {
                            black_shape[black_count] = position;
                        }
                        black_count += 1;
                    }
                    EMPTY => {
                        if empty.replace(position).is_some() {
                            valid_window = false;
                            break;
                        }
                    }
                    _ => {
                        valid_window = false;
                        break;
                    }
                }
            }

            if !valid_window || black_count != 4 || empty.is_none() {
                continue;
            }
            let left_open = start == 0 || self.cells[start - 1] != BLACK;
            let right_open = end == self.len || self.cells[end] != BLACK;
            if !left_open || !right_open {
                continue;
            }
            if !shapes[..shape_count].contains(&black_shape) {
                shapes[shape_count] = black_shape;
                shape_count += 1;
            }
        }

        shape_count
    }

    fn open_three_gain_indices(&self) -> ([usize; 2], usize) {
        let mut gains = [NO_EXTRA_BLACK; 2];
        let mut len = 0;

        let mut index = self.anchor;
        for _ in 0..4 {
            if index == 0 {
                break;
            }
            index -= 1;
            match self.cells[index] {
                EMPTY => {
                    gains[len] = index;
                    len += 1;
                    break;
                }
                BLACK => {}
                _ => break,
            }
        }

        index = self.anchor;
        for _ in 0..4 {
            if index + 1 >= self.len {
                break;
            }
            index += 1;
            match self.cells[index] {
                EMPTY => {
                    gains[len] = index;
                    len += 1;
                    break;
                }
                BLACK => {}
                _ => break,
            }
        }

        (gains, len)
    }

    fn has_open_four_through_gain(&self, gain: usize) -> bool {
        let mut winning_extensions = 0;
        let start = self.anchor.saturating_sub(5);
        let end = usize::min(self.len, self.anchor + 6);

        for extension in start..end {
            if extension == self.anchor || extension == gain || self.cells[extension] != EMPTY {
                continue;
            }
            let Some((run_start, run_end)) =
                self.black_run_bounds_with_extra(extension, gain, extension)
            else {
                continue;
            };
            if run_end - run_start + 1 == 5
                && run_start <= self.anchor
                && self.anchor <= run_end
                && run_start <= gain
                && gain <= run_end
            {
                winning_extensions += 1;
                if winning_extensions >= 2 {
                    return true;
                }
            }
        }

        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleSet {
    Freestyle,
    Renju,
}

impl RuleSet {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Freestyle => "freestyle",
            Self::Renju => "renju",
        }
    }
}

impl std::str::FromStr for RuleSet {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.to_ascii_lowercase().as_str() {
            "0" | "freestyle" | "free" => Ok(Self::Freestyle),
            "4" | "renju" => Ok(Self::Renju),
            _ => Err(format!("unknown rule set: {raw}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForbiddenKind {
    None,
    DoubleThree,
    DoubleFour,
    Overline,
}

impl ForbiddenKind {
    pub fn is_forbidden(self) -> bool {
        self != Self::None
    }
}

pub fn classify_forbidden_move(
    board: &Board,
    move_: Move,
    side: Side,
    rule: RuleSet,
) -> Result<ForbiddenKind, BoardError> {
    let (x, y) = move_to_xy(move_)?;
    if !is_valid_side(side) {
        return Err(BoardError::InvalidSide(side));
    }
    if board.at(x, y)? != EMPTY {
        return Err(BoardError::IllegalMove(move_));
    }
    if rule != RuleSet::Renju || side != BLACK {
        return Ok(ForbiddenKind::None);
    }

    let mut grid = *board.grid_rows();
    Ok(classify_black_move_on_grid(&mut grid, x, y))
}

pub fn classify_forbidden_stones(
    stones: &[(usize, usize, Side)],
    candidate: (usize, usize),
    side: Side,
    rule: RuleSet,
) -> Result<ForbiddenKind, BoardError> {
    let candidate_move = xy_to_move(candidate.0, candidate.1)?;
    if !is_valid_side(side) {
        return Err(BoardError::InvalidSide(side));
    }

    let mut grid = [[EMPTY; BOARD_SIZE]; BOARD_SIZE];
    for &(x, y, stone_side) in stones {
        if !is_valid_side(stone_side) {
            return Err(BoardError::InvalidSide(stone_side));
        }
        let move_ = xy_to_move(x, y)?;
        if grid[y][x] != EMPTY {
            return Err(BoardError::IllegalMove(move_));
        }
        grid[y][x] = stone_side;
    }

    if grid[candidate.1][candidate.0] != EMPTY {
        return Err(BoardError::IllegalMove(candidate_move));
    }
    if rule != RuleSet::Renju || side != BLACK {
        return Ok(ForbiddenKind::None);
    }
    Ok(classify_black_move_on_grid(
        &mut grid,
        candidate.0,
        candidate.1,
    ))
}

fn classify_black_move_on_grid(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
) -> ForbiddenKind {
    debug_assert_eq!(grid[y][x], EMPTY);
    grid[y][x] = BLACK;
    let kind = classify_placed_black(grid, x, y);
    grid[y][x] = EMPTY;
    kind
}

fn classify_placed_black(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
) -> ForbiddenKind {
    if has_exact_five(grid, x, y) {
        return ForbiddenKind::None;
    }
    if has_overline(grid, x, y) {
        return ForbiddenKind::Overline;
    }
    if count_four_directions(grid, x, y) >= 2 {
        return ForbiddenKind::DoubleFour;
    }
    if count_true_open_three_directions(grid, x, y) >= 2 {
        return ForbiddenKind::DoubleThree;
    }
    ForbiddenKind::None
}

fn is_legal_black_gain(grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE], x: usize, y: usize) -> bool {
    debug_assert_eq!(grid[y][x], EMPTY);
    grid[y][x] = BLACK;
    let legal = classify_placed_black(grid, x, y) == ForbiddenKind::None;
    grid[y][x] = EMPTY;
    legal
}

fn has_exact_five(grid: &[[Side; BOARD_SIZE]; BOARD_SIZE], x: usize, y: usize) -> bool {
    DIRECTIONS
        .into_iter()
        .any(|dir| DirectionalLine::from_grid(grid, x, y, dir).has_exact_five())
}

fn has_overline(grid: &[[Side; BOARD_SIZE]; BOARD_SIZE], x: usize, y: usize) -> bool {
    DIRECTIONS
        .into_iter()
        .any(|dir| DirectionalLine::from_grid(grid, x, y, dir).has_overline())
}

fn count_four_directions(grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE], x: usize, y: usize) -> usize {
    DIRECTIONS
        .into_iter()
        .map(|dir| count_four_shapes_through(grid, x, y, dir))
        .sum()
}

fn count_true_open_three_directions(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
) -> usize {
    DIRECTIONS
        .into_iter()
        .filter(|&dir| is_true_open_three_direction(grid, x, y, dir))
        .count()
}

fn has_four_through(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    dir: (isize, isize),
) -> bool {
    count_four_shapes_through(grid, x, y, dir) > 0
}

fn count_four_shapes_through(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    dir: (isize, isize),
) -> usize {
    DirectionalLine::from_grid(grid, x, y, dir).count_four_shapes_through()
}

fn is_true_open_three_direction(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    dir: (isize, isize),
) -> bool {
    if has_four_through(grid, x, y, dir) {
        return false;
    }

    let line = DirectionalLine::from_grid(grid, x, y, dir);
    let (gains, gain_count) = line.open_three_gain_indices();
    for &gain in &gains[..gain_count] {
        let gx = line.xs[gain];
        let gy = line.ys[gain];
        if line.has_open_four_through_gain(gain) && is_legal_black_gain(grid, gx, gy) {
            return true;
        }
    }
    false
}

fn step(x: usize, y: usize, dx: isize, dy: isize) -> Option<(usize, usize)> {
    let nx = x as isize + dx;
    let ny = y as isize + dy;
    if nx < 0 || ny < 0 || nx >= BOARD_SIZE as isize || ny >= BOARD_SIZE as isize {
        return None;
    }
    Some((nx as usize, ny as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::xy_to_move;
    use crate::constants::WHITE;
    use serde::Deserialize;
    use std::collections::BTreeSet;

    #[derive(Deserialize)]
    struct Fixture {
        name: String,
        moves: Vec<Stone>,
        candidate: Point,
        expected: String,
    }

    #[derive(Deserialize)]
    struct Stone {
        x: usize,
        y: usize,
        side: Side,
    }

    #[derive(Deserialize)]
    struct Point {
        x: usize,
        y: usize,
    }

    fn expected_kind(raw: &str) -> ForbiddenKind {
        match raw {
            "none" => ForbiddenKind::None,
            "double_three" => ForbiddenKind::DoubleThree,
            "double_four" => ForbiddenKind::DoubleFour,
            "overline" => ForbiddenKind::Overline,
            other => panic!("unknown expected forbidden kind {other}"),
        }
    }

    fn board_from_fixture(fixture: &Fixture) -> Board {
        let mut board = Board::new();
        for stone in &fixture.moves {
            assert!(matches!(stone.side, BLACK | WHITE), "{}", fixture.name);
            assert_eq!(
                board.grid_rows()[stone.y][stone.x],
                EMPTY,
                "duplicate stone in {} at {},{}",
                fixture.name,
                stone.x,
                stone.y
            );
            board.grid_rows_mut()[stone.y][stone.x] = stone.side;
        }
        board
    }

    const EXHAUSTION_DIRS: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

    // Lay the enumerated 1-D line along `dir` through the board centre, so the
    // same exhaustion also exercises the vertical/diagonal coordinate transforms
    // (step/offset/contiguous_segment), not just the horizontal row.
    fn line_grid_along(
        line: &[Side; BOARD_SIZE],
        dir: (isize, isize),
    ) -> [[Side; BOARD_SIZE]; BOARD_SIZE] {
        let center = (BOARD_SIZE / 2) as isize;
        let (dx, dy) = dir;
        let mut grid = [[EMPTY; BOARD_SIZE]; BOARD_SIZE];
        for (position, &side) in line.iter().enumerate() {
            if side == EMPTY {
                continue;
            }
            let offset = position as isize - center;
            let x = center + offset * dx;
            let y = center + offset * dy;
            // Non-empty cells only exist inside the enumerated window, which
            // stays in-bounds for all four directions.
            grid[y as usize][x as usize] = side;
        }
        grid
    }

    fn enumerated_line(width: usize, mut code: usize) -> [Side; BOARD_SIZE] {
        assert!(width % 2 == 1);
        let center = BOARD_SIZE / 2;
        let half_width = width / 2;
        let mut line = [EMPTY; BOARD_SIZE];
        line[center] = BLACK;

        for position in (center - half_width)..=(center + half_width) {
            if position == center {
                continue;
            }
            line[position] = match code % 3 {
                0 => EMPTY,
                1 => BLACK,
                _ => WHITE,
            };
            code /= 3;
        }
        line
    }

    fn slow_center_run_len(line: &[Side; BOARD_SIZE]) -> usize {
        let center = BOARD_SIZE / 2;
        let mut start = center;
        while start > 0 && line[start - 1] == BLACK {
            start -= 1;
        }
        let mut end = center;
        while end + 1 < BOARD_SIZE && line[end + 1] == BLACK {
            end += 1;
        }
        end - start + 1
    }

    fn slow_has_exact_five(line: &[Side; BOARD_SIZE]) -> bool {
        slow_center_run_len(line) == 5
    }

    fn slow_has_overline(line: &[Side; BOARD_SIZE]) -> bool {
        slow_center_run_len(line) >= 6
    }

    fn slow_four_shape_count(line: &[Side; BOARD_SIZE]) -> usize {
        let center = BOARD_SIZE / 2;
        let mut shapes = BTreeSet::new();

        for start in 0..=(BOARD_SIZE - 5) {
            let end = start + 5;
            if !(start <= center && center < end) {
                continue;
            }

            let mut black_count = 0;
            let mut empty = None;
            let mut black_shape = [usize::MAX; 4];
            let mut valid_window = true;
            for (shape_index, &side) in line[start..end].iter().enumerate() {
                let position = start + shape_index;
                match side {
                    BLACK => {
                        if black_count < 4 {
                            black_shape[black_count] = position;
                        }
                        black_count += 1;
                    }
                    EMPTY => {
                        if empty.replace(position).is_some() {
                            valid_window = false;
                            break;
                        }
                    }
                    _ => {
                        valid_window = false;
                        break;
                    }
                }
            }

            if !valid_window || black_count != 4 || empty.is_none() {
                continue;
            }

            let left_is_open = start == 0 || line[start - 1] != BLACK;
            let right_is_open = end == BOARD_SIZE || line[end] != BLACK;
            if left_is_open && right_is_open {
                shapes.insert(black_shape);
            }
        }

        shapes.len()
    }

    fn assert_line_exhaustion_matches_slow_reference(width: usize) {
        let center = BOARD_SIZE / 2;
        let cases = 3usize.pow((width - 1) as u32);
        for code in 0..cases {
            let line = enumerated_line(width, code);
            // The slow reference is purely 1-D, so the expected values are the
            // same for every direction the line is laid along.
            let expected_exact_five = slow_has_exact_five(&line);
            let expected_overline = slow_has_overline(&line);
            let expected_four = slow_four_shape_count(&line);

            for dir in EXHAUSTION_DIRS {
                let mut grid = line_grid_along(&line, dir);

                assert_eq!(
                    has_exact_five(&grid, center, center),
                    expected_exact_five,
                    "width={width} code={code} dir={dir:?} line={line:?} exact-five mismatch"
                );
                assert_eq!(
                    has_overline(&grid, center, center),
                    expected_overline,
                    "width={width} code={code} dir={dir:?} line={line:?} overline mismatch"
                );
                assert_eq!(
                    count_four_shapes_through(&mut grid, center, center, dir),
                    expected_four,
                    "width={width} code={code} dir={dir:?} line={line:?} four-count mismatch"
                );
            }
        }
    }

    #[test]
    fn hand_fixtures_match_oracle_expectations() {
        let text = include_str!("../../cases/renju/forbidden_hand_cases.jsonl");
        for (line_index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fixture: Fixture = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("line {} fixture parses: {err}", line_index + 1));
            let board = board_from_fixture(&fixture);
            let move_ = xy_to_move(fixture.candidate.x, fixture.candidate.y)
                .expect("fixture candidate stays in range");
            let actual = classify_forbidden_move(&board, move_, BLACK, RuleSet::Renju)
                .unwrap_or_else(|err| panic!("{} classify failed: {err:?}", fixture.name));
            assert_eq!(actual, expected_kind(&fixture.expected), "{}", fixture.name);
        }
    }

    #[test]
    fn freestyle_and_white_moves_are_never_forbidden() {
        let mut board = Board::new();
        board.grid_rows_mut()[7][4] = BLACK;
        board.grid_rows_mut()[7][5] = BLACK;
        board.grid_rows_mut()[7][6] = BLACK;
        board.grid_rows_mut()[7][8] = BLACK;
        board.grid_rows_mut()[7][9] = BLACK;
        let move_ = xy_to_move(7, 7).unwrap();

        assert_eq!(
            classify_forbidden_move(&board, move_, BLACK, RuleSet::Freestyle).unwrap(),
            ForbiddenKind::None
        );
        assert_eq!(
            classify_forbidden_move(&board, move_, WHITE, RuleSet::Renju).unwrap(),
            ForbiddenKind::None
        );
    }

    #[test]
    fn invalid_side_is_rejected() {
        let board = Board::new();
        let move_ = xy_to_move(7, 7).unwrap();
        assert_eq!(
            classify_forbidden_move(&board, move_, 2, RuleSet::Renju),
            Err(BoardError::InvalidSide(2))
        );
        assert_eq!(
            classify_forbidden_stones(&[], (7, 7), 2, RuleSet::Renju),
            Err(BoardError::InvalidSide(2))
        );
    }

    #[test]
    fn line_exhaustion_width_9_matches_slow_reference() {
        assert_line_exhaustion_matches_slow_reference(9);
    }

    #[test]
    fn line_exhaustion_width_11_matches_slow_reference() {
        assert_line_exhaustion_matches_slow_reference(11);
    }
}
