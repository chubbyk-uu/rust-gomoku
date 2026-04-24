use rust_gomoku::constants::{HASHF_ALPHA, HASHF_BETA, HASHF_EXACT};
use rust_gomoku::{xy_to_move, TTEntry, TranspositionTable};

#[test]
fn tt_exact_hit_returns_value_and_best_move() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 5,
        value: 123,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 9,
        best_move: Some(77),
    });
    let result = table.probe(5, 4, -10, 10);
    assert!(result.hit);
    assert_eq!(result.value, Some(123));
    assert_eq!(result.best_move, Some(77));
}

#[test]
fn tt_alpha_entry_can_shrink_window() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 5,
        value: 20,
        flag: HASHF_ALPHA,
        depth: 4,
        priority: 9,
        best_move: Some(11),
    });
    let result = table.probe(5, 4, 0, 100);
    assert!(!result.hit);
    assert!(result.has_window);
    assert_eq!(result.window_alpha, 0);
    assert_eq!(result.window_beta, 21);
    assert_eq!(result.best_move, Some(11));
}

#[test]
fn tt_beta_entry_cuts_when_value_exceeds_beta() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 5,
        value: 80,
        flag: HASHF_BETA,
        depth: 4,
        priority: 9,
        best_move: Some(33),
    });
    let result = table.probe(5, 4, 0, 50);
    assert!(result.hit);
    assert_eq!(result.value, Some(80));
    assert_eq!(result.best_move, None);
}

#[test]
fn tt_alpha_entry_returns_unknown_with_best_move_when_no_cut() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 5,
        value: 80,
        flag: HASHF_ALPHA,
        depth: 4,
        priority: 9,
        best_move: Some(17),
    });
    let result = table.probe(5, 5, 0, 100);
    assert!(!result.hit);
    assert!(!result.has_window);
    assert_eq!(result.value, None);
    assert_eq!(result.best_move, Some(17));
}

#[test]
fn tt_beta_entry_narrows_alpha_window_when_no_cut() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 5,
        value: 80,
        flag: HASHF_BETA,
        depth: 4,
        priority: 9,
        best_move: Some(17),
    });
    let result = table.probe(5, 4, 0, 100);
    assert!(!result.hit);
    assert!(result.has_window);
    assert_eq!(result.window_alpha, 80);
    assert_eq!(result.window_beta, 100);
    assert_eq!(result.value, None);
    assert_eq!(result.best_move, Some(17));
}

#[test]
fn tt_beta_entry_alpha_already_above_value_keeps_alpha() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 5,
        value: 30,
        flag: HASHF_BETA,
        depth: 4,
        priority: 9,
        best_move: Some(17),
    });
    let result = table.probe(5, 4, 50, 100);
    assert!(result.has_window);
    assert_eq!(result.window_alpha, 50);
    assert_eq!(result.window_beta, 100);
}

#[test]
fn tt_second_slot_checked_when_first_has_insufficient_depth() {
    let table = TranspositionTable::new(1);
    table.store(TTEntry {
        key: 2,
        value: 99,
        flag: HASHF_EXACT,
        depth: 1,
        priority: 100,
        best_move: Some(5),
    });
    table.store(TTEntry {
        key: 2,
        value: 42,
        flag: HASHF_EXACT,
        depth: 6,
        priority: 10,
        best_move: Some(7),
    });
    let result = table.probe(2, 5, -200, 200);
    assert!(result.hit);
    assert_eq!(result.value, Some(42));
    assert_eq!(result.best_move, Some(7));
}

#[test]
fn tt_prefers_second_slot_when_first_has_higher_priority() {
    let table = TranspositionTable::new(1);
    table.store(TTEntry {
        key: 2,
        value: 10,
        flag: HASHF_EXACT,
        depth: 1,
        priority: 100,
        best_move: Some(1),
    });
    table.store(TTEntry {
        key: 4,
        value: 20,
        flag: HASHF_EXACT,
        depth: 1,
        priority: 10,
        best_move: Some(2),
    });
    assert_eq!(table.probe(2, 1, -5, 5).value, Some(10));
    assert_eq!(table.probe(4, 1, -5, 5).value, Some(20));
}

#[test]
fn tt_default_bucket_bits_match_reference() {
    let table = TranspositionTable::default();
    assert_eq!(table.bucket_mask, (1_u64 << 20) - 1);
}

#[test]
fn tt_allocates_fixed_bucket_count() {
    let table = TranspositionTable::new(4);
    assert_eq!(table.bucket_count(), 16);
    assert_eq!(table.bucket(3), [TTEntry::default(), TTEntry::default()]);
    table.store(TTEntry {
        key: 3,
        value: 10,
        flag: HASHF_EXACT,
        depth: 2,
        priority: 5,
        best_move: Some(7),
    });
    assert_eq!(table.bucket_count(), 16);
}

#[test]
fn tt_probe_leaves_empty_bucket_unchanged() {
    let table = TranspositionTable::new(4);
    let before = table.bucket(7);
    let result = table.probe(7, 1, -10, 10);
    assert!(!result.hit);
    assert_eq!(result.value, None);
    assert_eq!(result.best_move, None);
    assert_eq!(table.bucket(7), before);
}

#[test]
fn tt_probe_does_not_create_bucket_for_miss_in_existing_slot() {
    let table = TranspositionTable::new(2);
    table.store(TTEntry {
        key: 1,
        value: 10,
        flag: HASHF_EXACT,
        depth: 2,
        priority: 5,
        best_move: Some(7),
    });
    let before = table.bucket(5);
    let result = table.probe(5, 2, -10, 10);
    assert!(!result.hit);
    assert_eq!(result.value, None);
    assert_eq!(result.best_move, None);
    assert_eq!(table.bucket(5), before);
}

#[test]
fn tt_third_store_in_same_slot_replaces_second_slot_only() {
    let table = TranspositionTable::new(1);
    table.store(TTEntry {
        key: 2,
        value: 10,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 100,
        best_move: Some(1),
    });
    table.store(TTEntry {
        key: 4,
        value: 20,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 10,
        best_move: Some(2),
    });
    table.store(TTEntry {
        key: 6,
        value: 30,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 5,
        best_move: Some(3),
    });
    assert_eq!(table.probe(2, 4, -50, 50).value, Some(10));
    assert_eq!(table.probe(4, 4, -50, 50).value, None);
    let result = table.probe(6, 4, -50, 50);
    assert!(result.hit);
    assert_eq!(result.value, Some(30));
    assert_eq!(result.best_move, Some(3));
}

#[test]
fn tt_higher_priority_third_store_replaces_first_slot() {
    let table = TranspositionTable::new(1);
    table.store(TTEntry {
        key: 2,
        value: 10,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 100,
        best_move: Some(1),
    });
    table.store(TTEntry {
        key: 4,
        value: 20,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 10,
        best_move: Some(2),
    });
    table.store(TTEntry {
        key: 6,
        value: 30,
        flag: HASHF_EXACT,
        depth: 4,
        priority: 200,
        best_move: Some(3),
    });
    assert_eq!(table.probe(2, 4, -50, 50).value, None);
    assert_eq!(table.probe(4, 4, -50, 50).value, Some(20));
    let result = table.probe(6, 4, -50, 50);
    assert!(result.hit);
    assert_eq!(result.value, Some(30));
    assert_eq!(result.best_move, Some(3));
}

#[test]
fn tt_bucket_returns_stored_entries_for_masked_slot() {
    let table = TranspositionTable::new(2);
    let best_move = xy_to_move(7, 7).unwrap();
    table.store(TTEntry {
        key: 5,
        value: 10,
        flag: HASHF_EXACT,
        depth: 3,
        priority: 7,
        best_move: Some(best_move),
    });
    let bucket = table.bucket(5);
    assert!(bucket.iter().any(|entry| {
        entry.key == 5
            && entry.value == 10
            && entry.flag == HASHF_EXACT
            && entry.best_move == Some(best_move)
    }));
}

#[test]
fn tt_clone_shares_storage_for_lazy_smp() {
    let table = TranspositionTable::new(2);
    let cloned = table.clone();
    assert!(table.is_shared_with(&cloned));
    table.store(TTEntry {
        key: 5,
        value: 123,
        flag: HASHF_EXACT,
        depth: 3,
        priority: 7,
        best_move: Some(9),
    });
    assert_eq!(cloned.probe(5, 3, -10, 10).value, Some(123));
}
