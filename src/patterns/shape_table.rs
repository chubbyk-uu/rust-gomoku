//! Shape lookup table loaded from the reference table text for exact alignment.

use std::sync::LazyLock;

// Generated plain-text static data extracted from the vendored reference text.
const SHAPE_TABLE_SOURCE: &str = include_str!("../../data/static/shape_table.txt");

pub static SHAPE_TABLE: LazyLock<Vec<Vec<i32>>> = LazyLock::new(parse_shape_table_lines);

pub fn shape_table_lookup(row: usize, index: usize) -> i32 {
    SHAPE_TABLE[row][index]
}

fn parse_shape_table_lines() -> Vec<Vec<i32>> {
    SHAPE_TABLE_SOURCE
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_whitespace()
                .map(|token| {
                    token
                        .parse::<i32>()
                        .expect("shape table entry parses as i32")
                })
                .collect()
        })
        .collect()
}
