use rust_gomoku::{caches_backend_name, EvalCaches, BOARD_SIZE};

#[test]
fn eval_caches_start_with_zeroed_storage() {
    let caches = EvalCaches::new();
    assert!(!caches.initialized);
    assert_eq!(caches.board_shadow.len(), BOARD_SIZE);
    assert_eq!(caches.shape_cache.len(), 2);
    assert_eq!(caches.board_shadow[0][0], 0);
    assert_eq!(caches.shape_cache[0][0][0], [0, 0, 0, 0]);
    assert_eq!(caches.value_cache[1][BOARD_SIZE - 1][BOARD_SIZE - 1], 0);
}

#[test]
fn caches_backend_name_is_supported() {
    assert!(matches!(caches_backend_name(), "python" | "cython"));
}

#[test]
fn eval_caches_reset_restores_zero_state() {
    let mut caches = EvalCaches::new();
    caches.initialized = true;
    caches.board_shadow[0][0] = 1;
    caches.shape_cache[0][0][0][0] = 123;
    caches.value_cache[1][0][0] = 9;
    caches.attack_cache[1][0][0] = 7;
    caches.reset();
    assert!(!caches.initialized);
    assert_eq!(caches.board_shadow[0][0], 0);
    assert_eq!(caches.shape_cache[0][0][0], [0, 0, 0, 0]);
    assert_eq!(caches.value_cache[1][0][0], 0);
    assert_eq!(caches.attack_cache[1][0][0], 0);
}

#[test]
fn eval_caches_snapshot_restore_roundtrip() {
    let mut caches = EvalCaches::new();
    caches.initialized = true;
    caches.board_shadow[0][0] = 1;
    caches.set_shape_value(0, 1, 1, 2, 99);
    let snapshot = caches.snapshot();
    caches.board_shadow[0][0] = 0;
    caches.set_shape_value(0, 1, 1, 2, 0);
    caches.restore_snapshot(&snapshot);
    assert!(caches.initialized);
    assert_eq!(caches.board_shadow[0][0], 1);
    assert_eq!(caches.shape_cache[0][1][1][2], 99);
}
