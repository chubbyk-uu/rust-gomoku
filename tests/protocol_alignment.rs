use rust_gomoku::{GomocupProtocol, SearchLimits};

fn proto() -> GomocupProtocol {
    GomocupProtocol::new(
        None,
        Some(SearchLimits {
            max_depth: 2,
            root_width: 8,
            ..SearchLimits::default()
        }),
    )
}

fn assert_xy(response: &[String]) {
    assert_eq!(response.len(), 1);
    let parts: Vec<_> = response[0].split(',').collect();
    assert_eq!(parts.len(), 2);
    let x: usize = parts[0].parse().unwrap();
    let y: usize = parts[1].parse().unwrap();
    assert!(x < 15);
    assert!(y < 15);
}

#[test]
fn protocol_start_accepts_15() {
    let mut proto = proto();
    assert_eq!(proto.handle_line("START 15"), ["OK"]);
}

#[test]
fn protocol_start_rejects_other_sizes() {
    let mut proto = proto();
    assert_eq!(proto.handle_line("START 20"), ["ERROR Size error."]);
}

#[test]
fn protocol_start_rejects_non_numeric_size() {
    let mut proto = proto();
    assert_eq!(proto.handle_line("START foo"), ["ERROR Size error."]);
}

#[test]
fn protocol_rectstart_rejects_non_numeric_size() {
    let mut proto = proto();
    assert_eq!(proto.handle_line("RECTSTART 15,foo"), ["ERROR Size error."]);
}

#[test]
fn protocol_begin_returns_center_move() {
    let mut proto = proto();
    proto.handle_line("START 15");
    assert_eq!(proto.handle_line("BEGIN"), ["7,7"]);
}

#[test]
fn protocol_turn_returns_move() {
    let mut proto = proto();
    proto.handle_line("START 15");
    let response = proto.handle_line("TURN 7,7");
    assert_xy(&response);
}

#[test]
fn protocol_turn_rejects_illegal_repeat_move() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("TURN 7,7");
    assert_eq!(proto.handle_line("TURN 7,7"), ["ERROR Illegal move."]);
}

#[test]
fn protocol_turn_rejects_non_numeric_coordinates() {
    let mut proto = proto();
    proto.handle_line("START 15");
    assert_eq!(proto.handle_line("TURN a,b"), ["ERROR Turn format error."]);
}

#[test]
fn protocol_turn_rejects_out_of_range_coordinates() {
    let mut proto = proto();
    proto.handle_line("START 15");
    assert_eq!(
        proto.handle_line("TURN 15,15"),
        ["ERROR Turn format error."]
    );
}

#[test]
fn protocol_about_returns_metadata() {
    let mut proto = proto();
    let response = proto.handle_line("ABOUT");
    assert_eq!(response.len(), 1);
    assert!(response[0].contains("rust_gomoku"));
}

#[test]
fn protocol_takeback_undoes_last_move() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("BEGIN");
    assert_eq!(proto.board.move_count(), 1);
    assert_eq!(proto.handle_line("TAKEBACK"), ["OK"]);
    assert_eq!(proto.board.move_count(), 0);
}

#[test]
fn protocol_takeback_on_empty_board_returns_ok() {
    let mut proto = proto();
    proto.handle_line("START 15");
    assert_eq!(proto.handle_line("TAKEBACK"), ["OK"]);
}

#[test]
fn protocol_board_mode_reconstructs_position() {
    let mut proto = proto();
    proto.handle_line("START 15");
    assert_eq!(proto.handle_line("BOARD"), Vec::<String>::new());
    assert_eq!(proto.handle_line("7,7,1"), Vec::<String>::new());
    assert_eq!(proto.handle_line("6,7,2"), Vec::<String>::new());
    let response = proto.handle_line("DONE");
    assert_xy(&response);
}

#[test]
fn protocol_board_mode_rejects_non_numeric_triplet() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("BOARD");
    assert_eq!(proto.handle_line("7,7,foo"), ["ERROR Board format error."]);
}

#[test]
fn protocol_board_mode_rejects_out_of_range_coordinates() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("BOARD");
    assert_eq!(proto.handle_line("15,0,1"), ["ERROR Board format error."]);
}

#[test]
fn protocol_board_mode_reconstructs_interleaved_color_order_as_expected() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("BOARD");
    for line in ["7,7,1", "5,5,2", "6,7,1", "5,6,2"] {
        assert_eq!(proto.handle_line(line), Vec::<String>::new());
    }
    proto.handle_line("DONE");
    let moves: Vec<_> = proto
        .board
        .move_history()
        .iter()
        .take(4)
        .map(|entry| (entry.move_ % 15, entry.move_ / 15, entry.side))
        .collect();
    assert_eq!(moves, vec![(7, 7, 1), (5, 5, -1), (6, 7, 1), (5, 6, -1)]);
}

#[test]
fn protocol_board_sfn_equals_opn_minus_one_plays_as_white() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("BOARD");
    proto.handle_line("7,7,2");
    let response = proto.handle_line("DONE");
    assert_xy(&response);
    assert_eq!(proto.board.move_count(), 2);
    assert_eq!(proto.board.move_history()[0].side, 1);
}

#[test]
fn protocol_info_static_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO static 0");
    assert!(!proto.config.runtime.static_board);
}

#[test]
fn protocol_info_compute_vcf_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO compute_vcf 0");
    assert!(!proto.config.runtime.compute_vcf);
}

#[test]
fn protocol_info_compute_vct_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO compute_vct 0");
    assert!(!proto.config.runtime.compute_vct);
}

#[test]
fn protocol_info_root_vct_depth_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO root_vct_depth 9");
    assert_eq!(proto.config.runtime.root_vct_depth, 9);
}

#[test]
fn protocol_info_root_vct_depth_negative_clamps_to_zero() {
    let mut proto = proto();
    proto.handle_line("INFO root_vct_depth -3");
    assert_eq!(proto.config.runtime.root_vct_depth, 0);
}

#[test]
fn protocol_info_max_node_zero_means_unlimited() {
    let mut proto = proto();
    proto.handle_line("INFO max_node 0");
    assert_eq!(proto.node_limit, None);
}

#[test]
fn protocol_info_timeout_turn_zero_matches_expected_floor() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_turn 0");
    assert_eq!(proto.timeout_turn_ms, Some(200.0));
}

#[test]
fn protocol_info_timeout_match_zero_matches_expected_large_default() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_match 0");
    assert_eq!(proto.time_left_ms, Some(99_999_999.0));
}

#[test]
fn protocol_info_invalid_numeric_values_are_ignored() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_turn 500");
    proto.handle_line("INFO timeout_turn foo");
    proto.handle_line("INFO time_left bar");
    proto.handle_line("INFO max_node baz");
    proto.handle_line("INFO compute_vcf qux");
    proto.handle_line("INFO compute_vct nope");
    proto.handle_line("INFO root_vct_depth nope");
    proto.handle_line("INFO static zed");
    assert_eq!(proto.timeout_turn_ms, Some(500.0));
    assert_eq!(proto.time_left_ms, None);
    assert_eq!(proto.node_limit, None);
    assert!(proto.config.runtime.compute_vcf);
    assert!(proto.config.runtime.compute_vct);
    assert_eq!(proto.config.runtime.root_vct_depth, 8);
    assert!(proto.config.runtime.static_board);
}

#[test]
fn protocol_unknown_command_silently_ignored_as_expected() {
    let mut proto = proto();
    assert_eq!(proto.handle_line("FOOBAR"), Vec::<String>::new());
    assert_eq!(proto.handle_line("XYZZY 123"), Vec::<String>::new());
}

#[test]
fn protocol_end_marks_protocol_ended() {
    let mut proto = proto();
    assert_eq!(proto.handle_line("END"), Vec::<String>::new());
    assert!(proto.ended);
}
