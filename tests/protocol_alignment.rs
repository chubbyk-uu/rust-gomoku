use rust_gomoku::{
    EngineProfile, GomocupProtocol, SearchLimits, DEFAULT_DYNAMIC_BOARD_MARGIN,
    DEFAULT_OPPONENT_VCF_DEPTH, DEFAULT_OVERLAP_VCT_ALPHABETA, DEFAULT_ROOT_PROFILE,
    DEFAULT_ROOT_VCF_DEPTH, DEFAULT_ROOT_VCT_DEPTH, DEFAULT_SEARCH_DEPTH, DEFAULT_SEARCH_WIDTH,
    DEFAULT_TIMED_SEARCH_MAX_DEPTH, DEFAULT_TIMED_SEARCH_MAX_WIDTH, DEFAULT_VCF_MULTI_REPLY,
    DEFAULT_VCT_STRICT_AND_MEMO_KEY, DEFAULT_VCT_VERIFY_OPPONENT_VCF_DEPTH,
};

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
    let move_response = response.last().expect("response has final move");
    let parts: Vec<_> = move_response.split(',').collect();
    assert_eq!(parts.len(), 2);
    let x: usize = parts[0].parse().unwrap();
    let y: usize = parts[1].parse().unwrap();
    assert!(x < 15);
    assert!(y < 15);
}

fn assert_last_xy(response: &[String]) {
    let last = response.last().expect("response has final move");
    let parts: Vec<_> = last.split(',').collect();
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
fn protocol_info_dynamic_board_margin_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO dynamic_board_margin 2");
    assert_eq!(proto.config.runtime.dynamic_board_margin, 2);
}

#[test]
fn protocol_info_dynamic_board_margin_negative_clamps_to_zero() {
    let mut proto = proto();
    proto.handle_line("INFO dynamic_board_margin -1");
    assert_eq!(proto.config.runtime.dynamic_board_margin, 0);
}

#[test]
fn protocol_info_compute_vcf_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO compute_vcf 0");
    assert!(!proto.config.runtime.compute_vcf);
}

#[test]
fn protocol_info_vcf_depths_update_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO root_vcf_depth 9");
    proto.handle_line("INFO opponent_vcf_depth 6");
    proto.handle_line("INFO vct_verify_opponent_vcf_depth 3");
    assert_eq!(proto.config.runtime.root_vcf_depth, 9);
    assert_eq!(proto.config.runtime.opponent_vcf_depth, 6);
    assert_eq!(proto.config.runtime.vct_verify_opponent_vcf_depth, 3);
}

#[test]
fn protocol_info_nonroot_vcf_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO nonroot_vcf 1");
    assert!(proto.config.runtime.nonroot_vcf);
    proto.handle_line("INFO nonroot_vcf 0");
    assert!(!proto.config.runtime.nonroot_vcf);
}

#[test]
fn protocol_info_vcf_multi_reply_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO vcf_multi_reply 1");
    assert!(proto.config.runtime.vcf_multi_reply);
    proto.handle_line("INFO vcf_multi_reply 0");
    assert!(!proto.config.runtime.vcf_multi_reply);
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
fn protocol_info_vct_strict_and_memo_key_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO vct_strict_and_memo_key 1");
    assert!(proto.config.runtime.vct_strict_and_memo_key);
    proto.handle_line("INFO vct_strict_and_memo_key 0");
    assert!(!proto.config.runtime.vct_strict_and_memo_key);
}

#[test]
fn protocol_info_overlap_vct_alphabeta_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO overlap_vct_alphabeta 1");
    assert!(proto.config.runtime.overlap_vct_alphabeta);
    proto.handle_line("INFO overlap_vct_alphabeta 0");
    assert!(!proto.config.runtime.overlap_vct_alphabeta);
}

#[test]
fn protocol_info_fast_history_ordering_updates_runtime() {
    let mut proto = proto();
    assert!(!proto.config.runtime.fast_history_ordering);
    proto.handle_line("INFO fast_history_ordering 1");
    assert!(proto.config.runtime.fast_history_ordering);
    proto.handle_line("INFO fast_history_ordering 0");
    assert!(!proto.config.runtime.fast_history_ordering);
}

#[test]
fn protocol_info_tt_bits_updates_engine_option() {
    let mut proto = proto();
    proto.handle_line("INFO tt_bits 24");
    assert_eq!(proto.tt_bits, Some(24));
}

#[test]
fn protocol_info_profile_updates_config_profile() {
    let mut proto = proto();
    assert_eq!(proto.config.profile, EngineProfile::Base);
    proto.handle_line("INFO profile fast");
    assert_eq!(proto.config.profile, EngineProfile::Fast);
    assert!(proto.config.runtime.vcf_multi_reply);
    assert!(proto.config.runtime.fast_history_ordering);
    proto.handle_line("INFO profile classic");
    assert_eq!(proto.config.profile, EngineProfile::Base);
    assert_eq!(
        proto.config.runtime.vcf_multi_reply,
        DEFAULT_VCF_MULTI_REPLY
    );
    assert!(!proto.config.runtime.fast_history_ordering);
}

#[test]
fn protocol_info_root_profile_updates_runtime() {
    let mut proto = proto();
    proto.handle_line("INFO root_profile 1");
    assert!(proto.config.runtime.root_profile);
    proto.handle_line("INFO root_profile 0");
    assert!(!proto.config.runtime.root_profile);
}

#[test]
fn protocol_root_profile_emits_messages_before_move_when_enabled() {
    let mut proto = proto();
    proto.handle_line("START 15");
    proto.handle_line("INFO root_profile 1");
    let response = proto.handle_line("TURN 7,7");
    assert!(response
        .iter()
        .any(|line| line.starts_with("MESSAGE root_profile ")));
    assert!(response
        .iter()
        .any(|line| line.starts_with("MESSAGE root_candidate ")));
    assert_last_xy(&response);
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
fn protocol_info_negative_time_values_are_ignored() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_turn 500");
    proto.handle_line("INFO timeout_match 600");
    proto.handle_line("INFO timeout_turn -1");
    proto.handle_line("INFO timeout_match -2");
    proto.handle_line("INFO time_left -3");
    assert_eq!(proto.timeout_turn_ms, Some(500.0));
    assert_eq!(proto.time_left_ms, Some(600.0));
}

#[test]
fn protocol_info_non_finite_time_values_are_ignored() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_turn 500");
    proto.handle_line("INFO timeout_match 600");
    proto.handle_line("INFO timeout_turn NaN");
    proto.handle_line("INFO timeout_match inf");
    proto.handle_line("INFO time_left -inf");
    assert_eq!(proto.timeout_turn_ms, Some(500.0));
    assert_eq!(proto.time_left_ms, Some(600.0));
}

#[test]
fn protocol_without_time_control_uses_fixed_search_defaults() {
    let proto = GomocupProtocol::default();
    let limits = proto.current_search_limits();
    assert_eq!(limits.max_depth, DEFAULT_SEARCH_DEPTH);
    assert_eq!(limits.root_width, DEFAULT_SEARCH_WIDTH as usize);
    assert_eq!(limits.time_limit_ms, None);
}

#[test]
fn protocol_time_control_uses_timed_search_caps() {
    let mut proto = GomocupProtocol::default();
    proto.handle_line("INFO timeout_turn 5000");
    let limits = proto.current_search_limits();
    assert_eq!(limits.max_depth, DEFAULT_TIMED_SEARCH_MAX_DEPTH);
    assert_eq!(limits.root_width, DEFAULT_TIMED_SEARCH_MAX_WIDTH as usize);
    assert_eq!(limits.time_limit_ms, Some(5000.0));
}

#[test]
fn protocol_explicit_search_limits_keep_depth_width_under_time_control() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_turn 5000");
    let limits = proto.current_search_limits();
    assert_eq!(limits.max_depth, 2);
    assert_eq!(limits.root_width, 8);
    assert_eq!(limits.time_limit_ms, Some(5000.0));
}

#[test]
fn protocol_info_invalid_numeric_values_are_ignored() {
    let mut proto = proto();
    proto.handle_line("INFO timeout_turn 500");
    proto.handle_line("INFO timeout_turn foo");
    proto.handle_line("INFO time_left bar");
    proto.handle_line("INFO max_node baz");
    proto.handle_line("INFO compute_vcf qux");
    proto.handle_line("INFO root_vcf_depth nope");
    proto.handle_line("INFO opponent_vcf_depth nope");
    proto.handle_line("INFO vct_verify_opponent_vcf_depth nope");
    proto.handle_line("INFO vcf_multi_reply nope");
    proto.handle_line("INFO nonroot_vcf nope");
    proto.handle_line("INFO compute_vct nope");
    proto.handle_line("INFO root_vct_depth nope");
    proto.handle_line("INFO vct_strict_and_memo_key nope");
    proto.handle_line("INFO overlap_vct_alphabeta nope");
    proto.handle_line("INFO profile nope");
    proto.handle_line("INFO root_profile nope");
    proto.handle_line("INFO static zed");
    proto.handle_line("INFO dynamic_board_margin hmm");
    assert_eq!(proto.timeout_turn_ms, Some(500.0));
    assert_eq!(proto.time_left_ms, None);
    assert_eq!(proto.node_limit, None);
    assert!(proto.config.runtime.compute_vcf);
    assert_eq!(proto.config.runtime.root_vcf_depth, DEFAULT_ROOT_VCF_DEPTH);
    assert_eq!(
        proto.config.runtime.opponent_vcf_depth,
        DEFAULT_OPPONENT_VCF_DEPTH
    );
    assert_eq!(
        proto.config.runtime.vct_verify_opponent_vcf_depth,
        DEFAULT_VCT_VERIFY_OPPONENT_VCF_DEPTH
    );
    assert_eq!(
        proto.config.runtime.vcf_multi_reply,
        DEFAULT_VCF_MULTI_REPLY
    );
    assert!(!proto.config.runtime.nonroot_vcf);
    assert!(proto.config.runtime.compute_vct);
    assert_eq!(proto.config.runtime.root_vct_depth, DEFAULT_ROOT_VCT_DEPTH);
    assert_eq!(
        proto.config.runtime.vct_strict_and_memo_key,
        DEFAULT_VCT_STRICT_AND_MEMO_KEY
    );
    assert_eq!(
        proto.config.runtime.overlap_vct_alphabeta,
        DEFAULT_OVERLAP_VCT_ALPHABETA
    );
    assert_eq!(proto.config.runtime.root_profile, DEFAULT_ROOT_PROFILE);
    assert_eq!(proto.config.profile, EngineProfile::Base);
    assert!(proto.config.runtime.static_board);
    assert_eq!(
        proto.config.runtime.dynamic_board_margin,
        DEFAULT_DYNAMIC_BOARD_MARGIN
    );
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
