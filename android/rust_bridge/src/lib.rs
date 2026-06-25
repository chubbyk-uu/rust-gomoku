//! Android JNI bridge crate.
//!
//! Phase 3 only proves packaging of the optimized Rust engine. JNI request
//! dispatch is implemented in Phase 4.

#[no_mangle]
pub extern "C" fn rust_gomoku_android_board_size() -> u32 {
    rust_gomoku::BOARD_SIZE as u32
}
