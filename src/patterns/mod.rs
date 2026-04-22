//! Pattern recognition modules aligned with the classic reference.

pub mod buckets;
pub mod line;
pub mod shape_table;
pub mod shapes;

pub use buckets::{bucket_for_lines, DOUBLE_SHAPE};
pub use line::{line_backend_name, shape_raw_from_cells_python, Line, PatternError};
pub use shapes::{
    PackedShape, ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, DIRECTION_IDS, HORIZONTAL, VERTICAL,
};
