//! Shape lookup table loaded from the reference table text for exact alignment.

// Generated plain-text static data extracted from the vendored reference text.
const SHAPE_TABLE_SOURCE: &str = include_str!("../../data/static/shape_table.txt");

/// One row per (freestyle vs. black-renju) shape table.
const SHAPE_TABLE_ROWS: usize = 2;
/// Entries per row in `data/static/shape_table.txt`.
const SHAPE_TABLE_COLS: usize = 3969;

const fn parse_shape_table_u8() -> [[u8; SHAPE_TABLE_COLS]; SHAPE_TABLE_ROWS] {
    let bytes = SHAPE_TABLE_SOURCE.as_bytes();
    let mut table = [[0_u8; SHAPE_TABLE_COLS]; SHAPE_TABLE_ROWS];
    let mut i = 0_usize;
    let mut row = 0_usize;
    let mut col = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\r' {
            i += 1;
        } else if b == b'\n' {
            if col > 0 {
                assert!(col == SHAPE_TABLE_COLS, "shape table row length mismatch");
                row += 1;
                col = 0;
            }
            i += 1;
        } else if b >= b'0' && b <= b'9' {
            let mut val = 0_u32;
            while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
                val = val * 10 + (bytes[i] - b'0') as u32;
                i += 1;
            }
            assert!(val <= u8::MAX as u32, "shape table value exceeds u8");
            assert!(row < SHAPE_TABLE_ROWS, "shape table has too many rows");
            assert!(col < SHAPE_TABLE_COLS, "shape table row has too many cols");
            table[row][col] = val as u8;
            col += 1;
        } else {
            panic!("unexpected byte in shape_table.txt");
        }
    }
    if col > 0 {
        assert!(
            col == SHAPE_TABLE_COLS,
            "shape table final row length mismatch"
        );
        row += 1;
    }
    assert!(row == SHAPE_TABLE_ROWS, "shape table row count mismatch");
    table
}

/// Compile-time `.rodata` compact `u8` shape lookup table (`7.9 KiB` vs `31.8 KiB`
/// heap-backed `i32` table), eliminating `LazyLock` atomic checks and heap
/// pointer indirection on the hottest eval path.
pub static SHAPE_TABLE: [[u8; SHAPE_TABLE_COLS]; SHAPE_TABLE_ROWS] = parse_shape_table_u8();

#[inline(always)]
pub fn shape_table_lookup(row: usize, index: usize) -> i32 {
    i32::from(SHAPE_TABLE[row][index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_shape_table_matches_runtime_parse() {
        let rows: Vec<Vec<i32>> = SHAPE_TABLE_SOURCE
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                l.split_whitespace()
                    .map(|t| t.parse::<i32>().expect("valid i32"))
                    .collect()
            })
            .collect();
        assert_eq!(rows.len(), SHAPE_TABLE_ROWS);
        for (r, row) in rows.iter().enumerate() {
            assert_eq!(row.len(), SHAPE_TABLE_COLS);
            for (c, &expected) in row.iter().enumerate() {
                assert_eq!(shape_table_lookup(r, c), expected);
            }
        }
    }
}
