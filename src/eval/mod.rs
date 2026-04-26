//! Evaluation modules aligned with the classic reference.

pub mod caches;
pub mod global_eval;
pub mod local;

pub use caches::{caches_backend_name, EvalCaches, EvalSnapshot};
pub use global_eval::{
    evaluate_board, evaluate_board_main, evaluate_board_main_cached, evaluate_board_main_scan,
    evaluate_last5_branch, evaluate_next43_branch, find_last5_target, global_eval_backend_name,
};
pub use local::{
    attack_level, compute_bucket_and_attack, compute_direction_shape, eval_value_last,
    eval_value_next, local_backend_name, move_value, recompute_all, recompute_point_caches,
    value_wide_compute,
};
