use std::path::Path;

use rust_gomoku::board::{move_to_xy, xy_to_move, Board};
use rust_gomoku::config::load_default_config;
use rust_gomoku::constants::INF;
use rust_gomoku::eval::{recompute_all_for_rule, EvalCaches};
use rust_gomoku::nnue::mix9svq::FcWeight;
use rust_gomoku::protocol::GomocupProtocol;
use rust_gomoku::rules::RuleSet;
use rust_gomoku::search::{
    AlphaBetaSearcher, RootSearcher, SearchLimits, SearchOptions, SearchStats, TranspositionTable,
};
use rust_gomoku::threats::DfpnSearcher;

const NNUE_DIR: &str = "opponent/rapfi-networks/mix9svq";

fn build_board(moves: &[(usize, usize)], rule: RuleSet) -> Board {
    let mut board = Board::new();
    for &(x, y) in moves {
        let mv = xy_to_move(x, y).expect("valid coord");
        board
            .play_for_rule(mv, None, rule)
            .expect("valid move in regression case");
    }
    board
}

fn has_local_weights() -> bool {
    Path::new(NNUE_DIR).exists()
}

/// Review Item 1 (P1): 4-way NNUE TT must isolate 1T integer-ply and MT 1/64-ply
/// depth units when a single `TranspositionTable` is shared across modes.
#[test]
fn review_item1_four_way_tt_isolates_1t_and_mt_depth_units() {
    if !has_local_weights() {
        return;
    }
    let moves = [(7, 7), (8, 8), (9, 5), (6, 8)];
    let tt = TranspositionTable::try_new_with_layout(15, true).expect("4-way TT allocates");

    let mut cfg_mt = load_default_config();
    cfg_mt.rule_set = RuleSet::Renju;
    cfg_mt.runtime.use_nnue = true;
    cfg_mt.runtime.nnue_dir = Some(NNUE_DIR.to_string());
    cfg_mt.runtime.compute_vcf = false;
    cfg_mt.runtime.compute_vct = false;
    cfg_mt.runtime.threads = 4;

    let mut cfg_1t = cfg_mt.clone();
    cfg_1t.runtime.threads = 1;

    // 1) MT mode writes depth 1 (encoded as 64 sub-plies).
    let mut s_mt = AlphaBetaSearcher::with_tt(cfg_mt, tt.clone());
    let mut b_mt = build_board(&moves, RuleSet::Renju);
    let mut c_mt = EvalCaches::new();
    recompute_all_for_rule(&mut b_mt, &mut c_mt, RuleSet::Renju);
    let side_mt = b_mt.side_to_move();
    let mut st_mt = SearchStats::default();
    let _ = s_mt.search(
        &mut b_mt,
        &mut c_mt,
        side_mt,
        1.0,
        -INF,
        INF,
        10,
        &mut st_mt,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );

    // 2) 1T mode probes the same shared TT at depth 3; must NOT treat MT depth 1 as 64.
    let mut s_1t = AlphaBetaSearcher::with_tt(cfg_1t.clone(), tt.clone());
    let mut b_1t = build_board(&moves, RuleSet::Renju);
    let mut c_1t = EvalCaches::new();
    recompute_all_for_rule(&mut b_1t, &mut c_1t, RuleSet::Renju);
    let side_1t = b_1t.side_to_move();
    let mut st_1t = SearchStats::default();
    let (sc_1t, mv_1t) = s_1t.search(
        &mut b_1t,
        &mut c_1t,
        side_1t,
        3.0,
        -INF,
        INF,
        10,
        &mut st_1t,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );

    // 3) Cold 1T reference at depth 3.
    let mut s_cold = AlphaBetaSearcher::with_tt(
        cfg_1t.clone(),
        TranspositionTable::try_new_with_layout(15, true).expect("cold 4-way TT"),
    );
    let mut b_cold = build_board(&moves, RuleSet::Renju);
    let mut c_cold = EvalCaches::new();
    recompute_all_for_rule(&mut b_cold, &mut c_cold, RuleSet::Renju);
    let side_cold = b_cold.side_to_move();
    let mut st_cold = SearchStats::default();
    let (sc_cold, mv_cold) = s_cold.search(
        &mut b_cold,
        &mut c_cold,
        side_cold,
        3.0,
        -INF,
        INF,
        10,
        &mut st_cold,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );

    assert_eq!(mv_1t, mv_cold);
    assert_eq!(sc_1t, sc_cold);
    assert!(
        st_1t.nodes > 1,
        "1T d3 must not falsely cut off on MT d1 (got {} vs cold {})",
        st_1t.nodes,
        st_cold.nodes
    );

    // 4) Same-mode repeat search MUST still hit the 4-way TT immediately.
    let mut b_rep = build_board(&moves, RuleSet::Renju);
    let mut c_rep = EvalCaches::new();
    recompute_all_for_rule(&mut b_rep, &mut c_rep, RuleSet::Renju);
    let side_rep = b_rep.side_to_move();
    let mut st_rep = SearchStats::default();
    let (sc_rep, mv_rep) = s_1t.search(
        &mut b_rep,
        &mut c_rep,
        side_rep,
        3.0,
        -INF,
        INF,
        10,
        &mut st_rep,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );
    assert_eq!(mv_rep, mv_cold);
    assert_eq!(sc_rep, sc_cold);
    assert_eq!(st_rep.nodes, 1);
}

/// Review Item 2 (P1): Hot rule switching (Freestyle <-> Renju) and concurrent
/// multi-rule sharing on the same TT must isolate TT scores and `static_eval`.
#[test]
fn review_item2_rule_and_weight_switches_isolate_tt_entries() {
    if !has_local_weights() {
        return;
    }
    let moves = [(7, 7), (8, 8), (9, 5), (6, 8)];
    let limits = SearchLimits {
        max_depth: 3,
        root_width: 10,
        node_limit: None,
        time_limit_ms: None,
    };

    let mut cfg_fs = load_default_config();
    cfg_fs.rule_set = RuleSet::Freestyle;
    cfg_fs.runtime.use_nnue = true;
    cfg_fs.runtime.nnue_dir = Some(NNUE_DIR.to_string());
    cfg_fs.runtime.compute_vcf = false;
    cfg_fs.runtime.compute_vct = false;
    cfg_fs.runtime.threads = 1;

    let mut cfg_rj = cfg_fs.clone();
    cfg_rj.rule_set = RuleSet::Renju;

    // Cold references for both rules.
    let mut cold_fs_rs = RootSearcher::with_tt(
        cfg_fs.clone(),
        TranspositionTable::try_new_with_layout(15, true).unwrap(),
    );
    let mut b_cold_fs = build_board(&moves, RuleSet::Freestyle);
    let cold_fs = cold_fs_rs.search(&mut b_cold_fs, Some(limits));

    let mut cold_rj_rs = RootSearcher::with_tt(
        cfg_rj.clone(),
        TranspositionTable::try_new_with_layout(15, true).unwrap(),
    );
    let mut b_cold_rj = build_board(&moves, RuleSet::Renju);
    let cold_rj = cold_rj_rs.search(&mut b_cold_rj, Some(limits));

    // Forward hot switch: Freestyle -> Renju, then reverse: Renju -> Freestyle.
    let shared_tt = TranspositionTable::try_new_with_layout(15, true).unwrap();
    let mut rs = RootSearcher::with_tt(cfg_fs.clone(), shared_tt.clone());
    let mut b1 = build_board(&moves, RuleSet::Freestyle);
    let r1 = rs.search(&mut b1, Some(limits));
    assert_eq!((r1.move_, r1.score), (cold_fs.move_, cold_fs.score));

    rs.config.rule_set = RuleSet::Renju;
    let mut b2 = build_board(&moves, RuleSet::Renju);
    let r2 = rs.search(&mut b2, Some(limits));
    assert_eq!((r2.move_, r2.score), (cold_rj.move_, cold_rj.score));

    // Reverse hot switch back to Freestyle.
    rs.config.rule_set = RuleSet::Freestyle;
    let mut b3 = build_board(&moves, RuleSet::Freestyle);
    let r3 = rs.search(&mut b3, Some(limits));
    assert_eq!((r3.move_, r3.score), (cold_fs.move_, cold_fs.score));

    // Two live searchers with different rules sharing the exact same TT.
    let mut live_fs = RootSearcher::with_tt(cfg_fs, shared_tt.clone());
    let mut live_rj = RootSearcher::with_tt(cfg_rj, shared_tt);
    let mut b4 = build_board(&moves, RuleSet::Freestyle);
    let mut b5 = build_board(&moves, RuleSet::Renju);
    let r4 = live_fs.search(&mut b4, Some(limits));
    let r5 = live_rj.search(&mut b5, Some(limits));
    assert_eq!((r4.move_, r4.score), (cold_fs.move_, cold_fs.score));
    assert_eq!((r5.move_, r5.score), (cold_rj.move_, cold_rj.score));
}

/// Review Item 4 (P2): df-pn depth boundary must detect blocking the opponent's
/// unique win5 while simultaneously creating a legal A4, without falsely proving
/// an A4 that leaves an opponent win5 unblocked.
#[test]
fn review_item4_dfpn_boundary_detects_block_win5_plus_a4_and_rejects_counterexample() {
    let moves = [
        (4, 7),
        (3, 7),
        (5, 7),
        (8, 8),
        (6, 7),
        (8, 9),
        (6, 8),
        (8, 10),
        (5, 9),
        (0, 0),
        (8, 11),
        (14, 14),
    ];
    let expected = xy_to_move(7, 7).unwrap();
    for rule in [RuleSet::Freestyle, RuleSet::Renju] {
        let board = build_board(&moves, rule);
        let mut dfpn = DfpnSearcher::default();
        let res = dfpn.search_for_rule(&board, board.side_to_move(), 2, rule);
        assert!(
            res.found,
            "{rule:?}: expected df-pn to prove win at max_ply=2"
        );
        assert_eq!(
            res.move_,
            Some(expected),
            "{rule:?}: expected winning move (7,7)"
        );
    }

    // Counterexample: same board except Black's blocker at (8,11) is moved to (1,1),
    // leaving (8,11) empty. After Black (7,7) B4 -> White forced (8,7), White has an
    // open-4 (8,7..10) with TWO win5 points (8,6) and (8,11). At the ply=2 boundary
    // OR node, Black (8,6) forms an A4 but leaves White's (8,11) win5 unblocked.
    let counter_moves = [
        (4, 7),
        (3, 7),
        (5, 7),
        (8, 8),
        (6, 7),
        (8, 9),
        (6, 8),
        (8, 10),
        (5, 9),
        (0, 0),
        (1, 1),
        (14, 14),
    ];
    for rule in [RuleSet::Freestyle, RuleSet::Renju] {
        let counter = build_board(&counter_moves, rule);
        let mut dfpn = DfpnSearcher::default();
        let res = dfpn.search_for_rule(&counter, counter.side_to_move(), 2, rule);
        assert!(
            !res.found,
            "{rule:?}: A4 leaving opponent win5 unblocked must NOT be proved at ply cap"
        );
        assert!(
            dfpn.stats.or_nodes >= 2 && dfpn.stats.and_nodes >= 1 && dfpn.stats.hit_depth_cap,
            "{rule:?}: expected boundary OR node to be visited, got stats={:?}",
            dfpn.stats
        );
    }
}

/// Review Item 5 (P2): Explicit `root_vcf_depth = 0` and small positive depths
/// must be respected, and weight-load failure must keep root VCF, alpha-beta,
/// and Gomocup match-clock pacing on the same classical fallback policy.
#[test]
fn review_item5_explicit_vcf_depth_and_missing_weight_fallback_are_consistent() {
    let moves = [(7, 7), (0, 0), (8, 7), (0, 1), (9, 7), (0, 2)];
    let limits = SearchLimits {
        max_depth: 2,
        root_width: 10,
        node_limit: None,
        time_limit_ms: None,
    };

    for (use_nnue, dir, expect_nnue) in [
        (false, None, false),
        (true, Some("nonexistent/weights".to_string()), false),
    ] {
        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.use_nnue = use_nnue;
        cfg.runtime.nnue_dir = dir;
        cfg.runtime.compute_vcf = true;
        cfg.runtime.root_vcf_depth = 0;
        cfg.runtime.compute_vct = false;

        let mut rs = RootSearcher::with_tt(cfg, TranspositionTable::new(15));
        let mut b = build_board(&moves, RuleSet::Freestyle);
        let _ = rs.search(&mut b, Some(limits));
        assert_eq!(rs.has_nnue(), expect_nnue);
        assert_eq!(
            rs.last_trace.as_ref().map(|t| t.tactical_path),
            Some("alphabeta")
        );
    }

    if has_local_weights() {
        let mut cfg = load_default_config();
        cfg.rule_set = RuleSet::Freestyle;
        cfg.runtime.use_nnue = true;
        cfg.runtime.nnue_dir = Some(NNUE_DIR.to_string());
        cfg.runtime.compute_vcf = true;
        cfg.runtime.root_vcf_depth = 0;
        cfg.runtime.compute_vct = false;

        let mut rs = RootSearcher::with_tt(
            cfg,
            TranspositionTable::try_new_with_layout(15, true).unwrap(),
        );
        let mut b = build_board(&moves, RuleSet::Freestyle);
        let _ = rs.search(&mut b, Some(limits));
        assert!(rs.has_nnue());
        assert_eq!(
            rs.last_trace.as_ref().map(|t| t.tactical_path),
            Some("alphabeta"),
            "explicit root_vcf_depth=0 must disable root VCF even with active NNUE"
        );
    }

    // Explicit root_vcf_depth override must distinguish unset (auto 16 for NNUE)
    // from explicit 0, 7, 8, 9, 16.
    let cfg_default = load_default_config();
    assert_eq!(cfg_default.runtime.effective_root_vcf_depth(false), 8);
    assert_eq!(cfg_default.runtime.effective_root_vcf_depth(true), 16);
    for explicit in [0, 7, 8, 9, 16] {
        let mut cfg_explicit = load_default_config();
        cfg_explicit.runtime.set_root_vcf_depth(explicit);
        assert_eq!(
            cfg_explicit.runtime.effective_root_vcf_depth(true),
            explicit
        );
        assert_eq!(
            cfg_explicit.runtime.effective_root_vcf_depth(false),
            explicit
        );

        let mut proto = GomocupProtocol::default();
        proto.handle_line(&format!("INFO root_vcf_depth {explicit}"));
        assert_eq!(
            proto.config.runtime.effective_root_vcf_depth(true),
            explicit
        );
    }

    // Direct mutation of `root_vcf_depth` after `set_root_vcf_depth(0)` must stay consistent.
    let mut cfg_mutated = load_default_config();
    cfg_mutated.runtime.set_root_vcf_depth(0);
    cfg_mutated.runtime.root_vcf_depth = 7;
    assert_eq!(cfg_mutated.runtime.effective_root_vcf_depth(true), 7);
    assert_eq!(cfg_mutated.runtime.effective_root_vcf_depth(false), 7);

    // Even before START / while Loading, valid nnue_dir + time_left 1000 must pace
    // the turn slice (~24ms) instead of handing the full 1000ms match clock to move #1.
    if has_local_weights() {
        let mut proto_loading = GomocupProtocol::default();
        proto_loading.handle_line(&format!("INFO nnue_dir {NNUE_DIR}"));
        proto_loading.handle_line("INFO rule 4");
        proto_loading.handle_line("INFO time_left 1000");
        let slice = proto_loading
            .current_search_limits()
            .time_limit_ms
            .expect("paced match slice");
        assert!(
            (15.0..=40.0).contains(&slice),
            "expected paced slice ~24ms for time_left=1000 while NNUE loading/ready, got {slice}"
        );
    }

    // Gomocup current_search_limits() must fall back to classical match-clock
    // semantics when NNUE weights fail to load.
    let mut proto = GomocupProtocol::default();
    proto.handle_line("INFO nnue_dir nonexistent/weights");
    proto.handle_line("INFO timeout_match 60000");
    assert_eq!(proto.current_search_limits().time_limit_ms, Some(60000.0));
}

/// Follow-up Item 2 (P2): Missing NNUE weights under tight turn budget must
/// record prewarm failure, reuse the classical searcher & explicit `tt_bits`,
/// and hit the cached TT on TAKEBACK repeats.
#[test]
fn review_followup2_missing_nnue_reuses_classical_searcher_and_explicit_tt_bits() {
    let limits = SearchLimits {
        max_depth: 3,
        root_width: 10,
        node_limit: None,
        time_limit_ms: Some(100.0),
    };
    let mut proto = GomocupProtocol::new(None, Some(limits));
    proto.handle_line("START 15");
    proto.handle_line("INFO compute_vcf 0");
    proto.handle_line("INFO compute_vct 0");
    proto.handle_line("INFO tt_bits 15");
    proto.handle_line("INFO timeout_turn 100");
    proto.handle_line("INFO nnue_dir nonexistent/weights");
    // Give the single background prewarm attempt a brief moment to record Failed.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let r1 = proto.handle_line("TURN 7,7");
    let reply1 = r1.last().expect("move response").clone();
    assert!(reply1.contains(','));
    assert_eq!(proto.handle_line(&format!("TAKEBACK {reply1}")), vec!["OK"]);
    assert_eq!(proto.handle_line("TAKEBACK 7,7"), vec!["OK"]);

    let r2 = proto.handle_line("TURN 7,7");
    let reply2 = r2.last().expect("repeat move response").clone();
    assert_eq!(reply1, reply2);
    // Repeated search on the cached classical TT hits previous-generation entries
    // (`old > 0`) instead of recreating and discarding a fresh searcher on every turn.
    assert!(
        r2.iter()
            .any(|l| l.contains("tt_generation") && !l.contains("old=0")),
        "expected cached TT hit (old > 0) on repeat search after missing-NNUE fallback, got {r2:?}"
    );
}

/// Follow-up Item 3②: Explicit `--tt-bits` via `RootSearcher::try_with_tt_bits`
/// must allocate the same single 4-way TT plane as auto-sized `new_for_limits`.
#[test]
fn review_followup3_explicit_tt_bits_allocates_single_plane_for_nnue() {
    if !has_local_weights() {
        return;
    }
    let mut cfg = load_default_config();
    cfg.rule_set = RuleSet::Renju;
    cfg.runtime.use_nnue = true;
    cfg.runtime.nnue_dir = Some(NNUE_DIR.to_string());
    cfg.runtime.compute_vcf = false;
    cfg.runtime.compute_vct = false;
    let rs = RootSearcher::try_with_tt_bits(cfg, 15).expect("explicit 15-bit TT");
    assert!(rs.has_nnue());
    assert_eq!(rs.tt.allocated_bucket_bytes(), (1_usize << 15) * 32);
}

/// Follow-up Item 3: Missing NNUE weights must NOT allocate both 4-way and 2-way TT planes.
#[test]
fn review_followup3_missing_and_valid_nnue_allocate_single_tt_plane() {
    let moves = [(7, 7), (5, 5), (10, 7), (7, 4), (10, 6), (9, 6)];
    let limits = SearchLimits {
        max_depth: 2,
        root_width: 10,
        node_limit: None,
        time_limit_ms: None,
    };

    // Missing weights -> classical fallback -> single 2-way plane only (32 MiB for 1T default 20-bit).
    let mut cfg_missing = load_default_config();
    cfg_missing.rule_set = RuleSet::Freestyle;
    cfg_missing.runtime.use_nnue = true;
    cfg_missing.runtime.nnue_dir = Some("nonexistent/weights".to_string());
    cfg_missing.runtime.compute_vcf = false;
    cfg_missing.runtime.compute_vct = false;
    cfg_missing.runtime.threads = 1;
    let mut rs_missing = RootSearcher::new_for_limits(cfg_missing, limits);
    assert!(!rs_missing.has_nnue());
    let expected_bytes = rs_missing.tt.bucket_count() * 32;
    assert_eq!(rs_missing.tt.allocated_bucket_bytes(), expected_bytes);
    let mut b_missing = build_board(&moves, RuleSet::Freestyle);
    let _ = rs_missing.search(&mut b_missing, Some(limits));
    assert_eq!(
        rs_missing.tt.allocated_bucket_bytes(),
        expected_bytes,
        "missing-weights fallback must stay on single 2-way TT plane after search"
    );

    if has_local_weights() {
        let mut cfg_valid = load_default_config();
        cfg_valid.rule_set = RuleSet::Freestyle;
        cfg_valid.runtime.use_nnue = true;
        cfg_valid.runtime.nnue_dir = Some(NNUE_DIR.to_string());
        cfg_valid.runtime.compute_vcf = false;
        cfg_valid.runtime.compute_vct = false;
        cfg_valid.runtime.threads = 1;
        let mut rs_valid = RootSearcher::new_for_limits(cfg_valid, limits);
        assert!(rs_valid.has_nnue());
        let expected_valid_bytes = rs_valid.tt.bucket_count() * 32;
        assert_eq!(rs_valid.tt.allocated_bucket_bytes(), expected_valid_bytes);
        let mut b_valid = build_board(&moves, RuleSet::Freestyle);
        let _ = rs_valid.search(&mut b_valid, Some(limits));
        assert_eq!(
            rs_valid.tt.allocated_bucket_bytes(),
            expected_valid_bytes,
            "valid NNUE search must stay on single 4-way TT plane after search"
        );
    }
}

/// Review Item 6 (P2): NNUE searches allocate only the 4-way TT plane (no
/// duplicate 2-way allocation) and `fork_snapshot()` preserves 4-way entries
/// while isolating subsequent writes.
#[test]
fn review_item6_nnue_single_tt_allocation_and_four_way_snapshot_isolation() {
    if !has_local_weights() {
        return;
    }
    let moves = [(7, 7), (5, 5), (10, 7), (7, 4), (10, 6), (9, 6)];
    let mut cfg = load_default_config();
    cfg.rule_set = RuleSet::Freestyle;
    cfg.runtime.use_nnue = true;
    cfg.runtime.nnue_dir = Some(NNUE_DIR.to_string());
    cfg.runtime.compute_vcf = false;
    cfg.runtime.compute_vct = false;
    cfg.runtime.threads = 1;

    let expected_single_plane_bytes = (1_usize << 15) * 32; // 1 MiB for bits=15
    let tt = TranspositionTable::try_new_with_layout(15, true).unwrap();
    assert_eq!(tt.allocated_bucket_bytes(), expected_single_plane_bytes);

    let mut s = AlphaBetaSearcher::with_tt(cfg.clone(), tt.clone());
    let mut b1 = build_board(&moves, RuleSet::Freestyle);
    let mut c1 = EvalCaches::new();
    recompute_all_for_rule(&mut b1, &mut c1, RuleSet::Freestyle);
    let side1 = b1.side_to_move();
    let mut st1 = SearchStats::default();
    let (sc1, mv1) = s.search(
        &mut b1,
        &mut c1,
        side1,
        3.0,
        -INF,
        INF,
        10,
        &mut st1,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );
    // Searching with NNUE must NOT lazily allocate the unused 2-way plane.
    assert_eq!(tt.allocated_bucket_bytes(), expected_single_plane_bytes);

    let snap = tt.fork_snapshot();
    assert_eq!(snap.allocated_bucket_bytes(), expected_single_plane_bytes);
    assert!(!snap.is_shared_with(&tt));

    let mut s_snap = AlphaBetaSearcher::with_tt(cfg, snap);
    let mut b2 = build_board(&moves, RuleSet::Freestyle);
    let mut c2 = EvalCaches::new();
    recompute_all_for_rule(&mut b2, &mut c2, RuleSet::Freestyle);
    let side2 = b2.side_to_move();
    let mut st2 = SearchStats::default();
    let (sc2, mv2) = s_snap.search(
        &mut b2,
        &mut c2,
        side2,
        3.0,
        -INF,
        INF,
        10,
        &mut st2,
        SearchOptions {
            opo: 1,
            root: true,
            ..SearchOptions::default()
        },
    );
    assert_eq!(mv2, mv1);
    assert_eq!(sc2, sc1);
    assert_eq!(
        st2.nodes, 1,
        "snapshot must hit cached 4-way NNUE TT entries"
    );
    assert_eq!(move_to_xy(mv2.unwrap()).unwrap(), (9, 7));
}

/// Review Item 7 (P1 library API): `FcWeight::new` rejects mismatched weight/bias
/// lengths at the construction boundary so safe callers cannot hand malformed
/// slices to SIMD kernels.
#[test]
fn review_item7_fc_weight_validates_dimensions_at_construction() {
    assert!(FcWeight::<4, 32>::new(vec![0; 4 * 32], vec![0; 4]).is_ok());
    assert!(FcWeight::<4, 32>::new(vec![0; 4 * 32 - 1], vec![0; 4]).is_err());
    assert!(FcWeight::<4, 32>::new(vec![0; 4 * 32], vec![0; 3]).is_err());
}
