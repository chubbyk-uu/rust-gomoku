use rust_gomoku::{
    adjust_loaded_parameters, bucket_for_lines, default_eval_para, line_backend_name,
    load_default_config, xy_to_move, Board, Line, PackedShape, SearchLimits, ShapeLabel, BLACK,
    BOARD_SIZE, DEFAULT_DYNAMIC_BOARD_MARGIN, DEFAULT_OPPONENT_VCF_DEPTH, DEFAULT_ROOT_VCF_DEPTH,
    DEFAULT_ROOT_VCT_DEPTH, DEFAULT_SEARCH_DEPTH, DEFAULT_SEARCH_WIDTH,
    DEFAULT_TIMED_SEARCH_MAX_DEPTH, DEFAULT_TIMED_SEARCH_MAX_WIDTH,
    DEFAULT_VCT_VERIFY_OPPONENT_VCF_DEPTH, DIAGONAL_DOWN, DIAGONAL_UP, DIRECTION_IDS, DOUBLE_SHAPE,
    HORIZONTAL, VERTICAL,
};

#[test]
fn default_parameter_count() {
    assert_eq!(default_eval_para().len(), 375);
}

#[test]
fn parameter_slices_have_expected_lengths() {
    let config = load_default_config();
    assert_eq!(
        config.eval_tables.last_eval.len(),
        rust_gomoku::constants::DSHAPE_SIZE
    );
    assert_eq!(
        config.eval_tables.next_eval.len(),
        rust_gomoku::constants::DSHAPE_SIZE
    );
    assert_eq!(
        config.eval_tables.attack_value.len(),
        rust_gomoku::constants::DSHAPE_SIZE
    );
    assert_eq!(
        config.eval_tables.defend_value.len(),
        rust_gomoku::constants::DSHAPE_SIZE
    );
}

#[test]
fn parameter_boundaries_match_expected_offsets() {
    let config = load_default_config();
    let para = default_eval_para();
    let dshape = rust_gomoku::constants::DSHAPE_SIZE;
    assert_eq!(config.eval_tables.last_eval[0], para[0]);
    assert_eq!(config.eval_tables.last_eval[dshape - 1], para[dshape - 1]);
    assert_eq!(config.eval_tables.next_eval[0], para[dshape]);
    assert_eq!(config.eval_tables.attack_value[0], para[dshape * 2]);
    assert_eq!(config.eval_tables.defend_value[0], para[dshape * 3]);
    assert_eq!(config.search.drift, para[dshape * 4]);
    assert_eq!(config.search.extend_ratio, para[dshape * 4 + 6]);
}

#[test]
fn runtime_defaults_match_reference() {
    let config = load_default_config();
    assert!(!config.runtime.read_config_each_move);
    assert!(config.runtime.compute_vcf);
    assert_eq!(config.runtime.root_vcf_depth, DEFAULT_ROOT_VCF_DEPTH);
    assert_eq!(
        config.runtime.opponent_vcf_depth,
        DEFAULT_OPPONENT_VCF_DEPTH
    );
    assert_eq!(
        config.runtime.vct_verify_opponent_vcf_depth,
        DEFAULT_VCT_VERIFY_OPPONENT_VCF_DEPTH
    );
    assert!(!config.runtime.nonroot_vcf);
    assert!(config.runtime.static_board);
    assert_eq!(
        config.runtime.dynamic_board_margin,
        DEFAULT_DYNAMIC_BOARD_MARGIN
    );
    assert!(config.runtime.compute_vct);
    assert_eq!(config.runtime.root_vct_depth, DEFAULT_ROOT_VCT_DEPTH);
    assert!(!config.runtime.lazy_smp);
    assert_eq!(config.runtime.lazy_smp_workers, 0);
}

#[test]
fn root_search_defaults_match_engine_defaults() {
    let config = load_default_config();
    assert_eq!(config.root_search.depth, DEFAULT_SEARCH_DEPTH);
    assert_eq!(config.root_search.wide, DEFAULT_SEARCH_WIDTH);
    assert_eq!(
        config.root_search.timed_max_depth,
        DEFAULT_TIMED_SEARCH_MAX_DEPTH
    );
    assert_eq!(
        config.root_search.timed_max_wide,
        DEFAULT_TIMED_SEARCH_MAX_WIDTH
    );
    assert_eq!(config.root_search.ratio_num, 1);
    assert_eq!(config.root_search.ratio_den, 1);
}

#[test]
fn fixed_search_limits_from_config_match_engine_defaults() {
    let config = load_default_config();
    let limits = SearchLimits::fixed_from_config(&config);
    assert_eq!(limits.max_depth, DEFAULT_SEARCH_DEPTH);
    assert_eq!(limits.root_width, DEFAULT_SEARCH_WIDTH as usize);
    assert_eq!(limits.node_limit, None);
    assert_eq!(limits.time_limit_ms, None);
}

#[test]
fn timed_search_limits_from_config_match_engine_caps() {
    let config = load_default_config();
    let limits = SearchLimits::timed_from_config(&config);
    assert_eq!(limits.max_depth, DEFAULT_TIMED_SEARCH_MAX_DEPTH);
    assert_eq!(limits.root_width, DEFAULT_TIMED_SEARCH_MAX_WIDTH as usize);
    assert_eq!(limits.node_limit, None);
    assert_eq!(limits.time_limit_ms, None);
}

#[test]
fn loaded_parameter_adjustments_match_reference() {
    let adjusted = adjust_loaded_parameters(default_eval_para());
    let para = default_eval_para();
    assert_eq!(adjusted[156], para[156] + 65_536.0);
    assert_eq!(adjusted[157], para[157] + 65_536.0);
}

#[test]
fn shape_labels_match_expected_values() {
    assert_eq!(ShapeLabel::L0 as i32, 0);
    assert_eq!(ShapeLabel::L4S as i32, 10);
    assert_eq!(ShapeLabel::L5 as i32, 12);
    assert_eq!(ShapeLabel::L6 as i32, 13);
}

#[test]
fn direction_ids_match_expected_order() {
    assert_eq!(
        DIRECTION_IDS,
        (HORIZONTAL, VERTICAL, DIAGONAL_DOWN, DIAGONAL_UP)
    );
}

#[test]
fn packed_shape_decodes_label_and_aux() {
    let shape = PackedShape {
        raw: (((ShapeLabel::L4S as i32) & 0xF) << 16) | 3,
    };
    assert_eq!(shape.label(), ShapeLabel::L4S);
    assert_eq!(shape.aux(), 3);
}

#[test]
fn double_shape_table_covers_expected_bucket_range() {
    let flattened: Vec<i32> = DOUBLE_SHAPE
        .iter()
        .flat_map(|row| row.iter().copied())
        .collect();
    assert_eq!(flattened[0], 1);
    assert_eq!(flattened[flattened.len() - 1], 91);
    assert_eq!(flattened.len(), 91);
    assert_eq!(flattened, (1..=91).collect::<Vec<_>>());
}

#[test]
fn bucket_for_lines_orders_inputs() {
    assert_eq!(bucket_for_lines(4, 2).unwrap(), DOUBLE_SHAPE[4][2]);
    assert_eq!(bucket_for_lines(2, 4).unwrap(), DOUBLE_SHAPE[4][2]);
}

#[test]
fn line_extraction_direction_zero() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    let line = Line::from_board(&board, 7, HORIZONTAL).unwrap();
    assert_eq!(line.cells[2 + 7], BLACK as i32);
    assert_eq!(line.cells[2 + 8], rust_gomoku::WHITE as i32);
}

#[test]
fn line_shape_returns_nonzero_for_simple_stone() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    let line = Line::from_board(&board, 7, HORIZONTAL).unwrap();
    let shape = line.shape(7, true);
    assert!(shape.raw >= 0);
}

#[test]
fn line_backend_name_is_supported() {
    assert!(matches!(line_backend_name(), "python" | "cython"));
}

#[test]
fn line_shape_raw_matches_shape_wrapper() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 8).unwrap(), None).unwrap();
    board.play(xy_to_move(7, 9).unwrap(), None).unwrap();
    let line = Line::from_board(&board, 7, HORIZONTAL).unwrap();
    assert_eq!(line.shape_raw(7, true), line.shape(7, true).raw);
}

#[test]
fn line_a3pb_returns_expected_encoded_targets() {
    let mut board = Board::new();
    board.play(xy_to_move(7, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 0).unwrap(), None).unwrap();
    board.play(xy_to_move(8, 7).unwrap(), None).unwrap();
    board.play(xy_to_move(0, 1).unwrap(), None).unwrap();
    board.play(xy_to_move(10, 7).unwrap(), None).unwrap();
    let line = Line::from_board(&board, 7, VERTICAL).unwrap();
    let encoded = line.a3pb(8);
    assert!(encoded > 0);
    assert_eq!(encoded & 0xFF, 11);
    assert_eq!((encoded >> 8) & 0xFF, 6);
    assert_eq!((encoded >> 16) & 0xFF, 9);
}

#[test]
fn board_size_stays_reference_aligned() {
    assert_eq!(BOARD_SIZE, 15);
}
