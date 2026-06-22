//! Pure Renju forbidden-move detection.
//!
//! This module is intentionally detached from `Board::is_legal_move`, move
//! generation, and search. It classifies a hypothetical black move under Renju
//! rules while preserving the current freestyle engine behavior.

use crate::board::{move_to_xy, xy_to_move, Board, BoardError};
use crate::constants::{BLACK, BOARD_SIZE, EMPTY};
use crate::types::{is_valid_side, Move, Side};

const DIRECTIONS: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleSet {
    Freestyle,
    Renju,
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
        .any(|dir| exact_five_segment(grid, x, y, dir).is_some())
}

fn has_overline(grid: &[[Side; BOARD_SIZE]; BOARD_SIZE], x: usize, y: usize) -> bool {
    DIRECTIONS
        .into_iter()
        .any(|dir| contiguous_segment(grid, x, y, dir).len() >= 6)
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
    let coords = line_coords(x, y, dir);
    if coords.len() < 5 {
        return 0;
    }
    let Some(anchor_index) = coords.iter().position(|&point| point == (x, y)) else {
        return 0;
    };
    let start_min = anchor_index.saturating_sub(4);
    let start_max = usize::min(anchor_index, coords.len().saturating_sub(5));
    let mut shapes: Vec<[(usize, usize); 4]> = Vec::new();

    for start in start_min..=start_max {
        let window = &coords[start..start + 5];
        let mut empty = None;
        let mut black_count = 0;
        let mut black_shape = [(usize::MAX, usize::MAX); 4];
        let mut valid_window = true;
        for &(wx, wy) in window {
            match grid[wy][wx] {
                BLACK => {
                    if black_count < 4 {
                        black_shape[black_count] = (wx, wy);
                    }
                    black_count += 1;
                }
                EMPTY => {
                    if empty.replace((wx, wy)).is_some() {
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
        if !valid_window || black_count != 4 || empty.is_none() || !window.contains(&(x, y)) {
            continue;
        }
        let (ex, ey) = empty.expect("empty was checked above");
        grid[ey][ex] = BLACK;
        let makes_exact_five = exact_five_segment(grid, ex, ey, dir)
            .is_some_and(|segment| segment.as_slice() == window);
        grid[ey][ex] = EMPTY;
        if makes_exact_five && !shapes.contains(&black_shape) {
            shapes.push(black_shape);
        }
    }

    shapes.len()
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

    for (gx, gy) in open_three_gain_candidates(grid, x, y, dir) {
        grid[gy][gx] = BLACK;
        let makes_open_four = has_open_four_through_gain(grid, x, y, gx, gy, dir);
        grid[gy][gx] = EMPTY;
        if makes_open_four && is_legal_black_gain(grid, gx, gy) {
            return true;
        }
    }
    false
}

fn has_open_four_through_gain(
    grid: &mut [[Side; BOARD_SIZE]; BOARD_SIZE],
    candidate_x: usize,
    candidate_y: usize,
    gain_x: usize,
    gain_y: usize,
    dir: (isize, isize),
) -> bool {
    let mut winning_extensions = 0;
    for (wx, wy) in nearby_line_coords(candidate_x, candidate_y, dir, 5) {
        if grid[wy][wx] != EMPTY {
            continue;
        }
        grid[wy][wx] = BLACK;
        let is_extension = exact_five_segment(grid, wx, wy, dir).is_some_and(|segment| {
            segment.contains(&(candidate_x, candidate_y)) && segment.contains(&(gain_x, gain_y))
        });
        grid[wy][wx] = EMPTY;
        if is_extension {
            winning_extensions += 1;
            if winning_extensions >= 2 {
                return true;
            }
        }
    }
    false
}

fn open_three_gain_candidates(
    grid: &[[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    (dx, dy): (isize, isize),
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (sx, sy) in [(dx, dy), (-dx, -dy)] {
        let mut cx = x;
        let mut cy = y;
        for _ in 0..4 {
            let Some((nx, ny)) = step(cx, cy, sx, sy) else {
                break;
            };
            match grid[ny][nx] {
                EMPTY => {
                    out.push((nx, ny));
                    break;
                }
                BLACK => {
                    cx = nx;
                    cy = ny;
                }
                _ => break,
            }
        }
    }
    out
}

fn exact_five_segment(
    grid: &[[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    dir: (isize, isize),
) -> Option<Vec<(usize, usize)>> {
    let segment = contiguous_segment(grid, x, y, dir);
    (segment.len() == 5).then_some(segment)
}

fn contiguous_segment(
    grid: &[[Side; BOARD_SIZE]; BOARD_SIZE],
    x: usize,
    y: usize,
    (dx, dy): (isize, isize),
) -> Vec<(usize, usize)> {
    if grid[y][x] != BLACK {
        return Vec::new();
    }

    let mut start_x = x;
    let mut start_y = y;
    while let Some((nx, ny)) = step(start_x, start_y, -dx, -dy) {
        if grid[ny][nx] != BLACK {
            break;
        }
        start_x = nx;
        start_y = ny;
    }

    let mut out = Vec::new();
    let mut cx = start_x;
    let mut cy = start_y;
    loop {
        out.push((cx, cy));
        let Some((nx, ny)) = step(cx, cy, dx, dy) else {
            break;
        };
        if grid[ny][nx] != BLACK {
            break;
        }
        cx = nx;
        cy = ny;
    }
    out
}

fn nearby_line_coords(
    x: usize,
    y: usize,
    (dx, dy): (isize, isize),
    radius: usize,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for distance in (1..=radius).rev() {
        if let Some(point) = offset(x, y, -dx, -dy, distance) {
            out.push(point);
        }
    }
    for distance in 1..=radius {
        if let Some(point) = offset(x, y, dx, dy, distance) {
            out.push(point);
        }
    }
    out
}

fn line_coords(x: usize, y: usize, (dx, dy): (isize, isize)) -> Vec<(usize, usize)> {
    let mut start_x = x;
    let mut start_y = y;
    while let Some((nx, ny)) = step(start_x, start_y, -dx, -dy) {
        start_x = nx;
        start_y = ny;
    }

    let mut out = Vec::new();
    let mut cx = start_x;
    let mut cy = start_y;
    loop {
        out.push((cx, cy));
        let Some((nx, ny)) = step(cx, cy, dx, dy) else {
            break;
        };
        cx = nx;
        cy = ny;
    }
    out
}

fn offset(x: usize, y: usize, dx: isize, dy: isize, distance: usize) -> Option<(usize, usize)> {
    let nx = x as isize + dx * distance as isize;
    let ny = y as isize + dy * distance as isize;
    if nx < 0 || ny < 0 || nx >= BOARD_SIZE as isize || ny >= BOARD_SIZE as isize {
        return None;
    }
    Some((nx as usize, ny as usize))
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
}
