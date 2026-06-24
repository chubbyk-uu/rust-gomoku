//! Shape lookup table loaded from the reference table text for exact alignment.

use std::sync::LazyLock;

// Generated plain-text static data extracted from the vendored reference text.
const SHAPE_TABLE_SOURCE: &str = include_str!("../../data/static/shape_table.txt");

/// One row per (freestyle vs. black-renju) shape table.
const SHAPE_TABLE_ROWS: usize = 2;
/// Entries per row in `data/static/shape_table.txt`.
const SHAPE_TABLE_COLS: usize = 3969;

// Contiguous, heap-backed table so each lookup is a single `Box` deref plus a
// flat array index into `.rodata`-style storage, instead of the `LazyLock` +
// double `Vec` pointer-chase a `Vec<Vec<i32>>` would cost on every shape
// computation (this lookup is on the hottest eval path).
pub static SHAPE_TABLE: LazyLock<Box<[[i32; SHAPE_TABLE_COLS]; SHAPE_TABLE_ROWS]>> =
    LazyLock::new(parse_shape_table_lines);

pub fn shape_table_lookup(row: usize, index: usize) -> i32 {
    SHAPE_TABLE[row][index]
}

fn parse_shape_table_lines() -> Box<[[i32; SHAPE_TABLE_COLS]; SHAPE_TABLE_ROWS]> {
    let mut table = Box::new([[0_i32; SHAPE_TABLE_COLS]; SHAPE_TABLE_ROWS]);
    let mut rows = 0_usize;
    for (row_index, line) in SHAPE_TABLE_SOURCE
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        assert!(
            row_index < SHAPE_TABLE_ROWS,
            "shape table has more than {SHAPE_TABLE_ROWS} rows"
        );
        let mut cols = 0_usize;
        for (col_index, token) in line.split_whitespace().enumerate() {
            assert!(
                col_index < SHAPE_TABLE_COLS,
                "shape table row {row_index} has more than {SHAPE_TABLE_COLS} entries"
            );
            table[row_index][col_index] = token
                .parse::<i32>()
                .expect("shape table entry parses as i32");
            cols = col_index + 1;
        }
        assert_eq!(
            cols, SHAPE_TABLE_COLS,
            "shape table row {row_index} has {cols} entries, expected {SHAPE_TABLE_COLS}"
        );
        rows = row_index + 1;
    }
    assert_eq!(
        rows, SHAPE_TABLE_ROWS,
        "shape table has {rows} rows, expected {SHAPE_TABLE_ROWS}"
    );
    table
}
