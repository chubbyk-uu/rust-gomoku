//! Emit Rust probe output for a fixed differential test case.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use rust_gomoku::{
    load_default_config, move_to_xy, xy_to_move, Board, RootSearcher, SearchLimits,
    VCTAndMemoCollisionSample, VCTDepthStats, VCTStats,
};

#[derive(Debug, Deserialize)]
struct DiffCase {
    name: String,
    moves: Vec<[i32; 2]>,
    #[serde(default = "default_first_side")]
    first_side: i8,
    #[serde(default)]
    limits: CaseLimits,
    #[serde(default)]
    runtime: CaseRuntime,
}

#[derive(Debug, Default, Deserialize)]
struct CaseLimits {
    max_depth: Option<i32>,
    root_width: Option<usize>,
    node_limit: Option<usize>,
    time_limit_ms: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct CaseRuntime {
    compute_vcf: Option<bool>,
    root_vcf_depth: Option<i32>,
    opponent_vcf_depth: Option<i32>,
    vct_verify_opponent_vcf_depth: Option<i32>,
    vcf_multi_reply: Option<bool>,
    nonroot_vcf: Option<bool>,
    compute_vct: Option<bool>,
    root_vct_depth: Option<i32>,
    vct_strict_and_memo_key: Option<bool>,
    overlap_vct_alphabeta: Option<bool>,
    static_board: Option<bool>,
    dynamic_board_margin: Option<i32>,
}

fn default_first_side() -> i8 {
    1
}

#[derive(Serialize)]
struct ProbeOutput {
    name: String,
    board: BoardSummary,
    root: RootSummary,
}

#[derive(Serialize)]
struct BoardSummary {
    side_to_move: i8,
    winner: i8,
    move_count: usize,
    zobrist_key: String,
}

#[derive(Serialize)]
struct RootSummary {
    #[serde(rename = "move")]
    move_xy: [usize; 2],
    score: i32,
    depth: i32,
    nodes: usize,
    trace: TraceSummary,
}

#[derive(Serialize)]
struct TraceSummary {
    used_vcf: bool,
    vcf_found: bool,
    used_vct: bool,
    vct_triggered: bool,
    vct_found: bool,
    vct_move: Option<[usize; 2]>,
    vct_accepted: bool,
    vct_reject_reason: Option<String>,
    vct_ms: Option<f64>,
    vct_stats: Option<VctStatsSummary>,
    alphabeta_ms: Option<f64>,
    overlap_used: bool,
    overlap_ab_ms: Option<f64>,
    overlap_ab_cancelled: bool,
    overlap_wait_ms: Option<f64>,
    tt_snapshot_ms: Option<f64>,
    tactical_path: String,
    root_profiles: Vec<RootDepthProfileSummary>,
}

#[derive(Serialize)]
struct VctStatsSummary {
    depth_limit: i32,
    depth_completed: i32,
    elapsed_ms: f64,
    or_nodes: usize,
    and_nodes: usize,
    memo_exact_hits: usize,
    memo_shallow_found_hits: usize,
    memo_shallow_solved_hits: usize,
    attacks_generated: usize,
    defenses_generated: usize,
    max_attack_count: usize,
    max_defense_count: usize,
    and_memo_context_observations: usize,
    and_memo_context_collisions: usize,
    and_memo_context_collision_keys: usize,
    and_memo_context_collision_samples: Vec<VctAndMemoCollisionSampleSummary>,
    depth_stats: Vec<VctDepthStatsSummary>,
}

#[derive(Serialize)]
struct VctAndMemoCollisionSampleSummary {
    observed_depth: i32,
    current_depth: i32,
    board_key: String,
    observed_signature: String,
    current_signature: String,
    attack_move: [usize; 2],
    attack_level: u8,
    defenses: Vec<[usize; 2]>,
}

#[derive(Serialize)]
struct VctDepthStatsSummary {
    depth: i32,
    elapsed_ms: f64,
    found: bool,
    solved: bool,
    or_nodes: usize,
    and_nodes: usize,
    memo_exact_hits: usize,
    memo_shallow_found_hits: usize,
    memo_shallow_solved_hits: usize,
    attacks_generated: usize,
    defenses_generated: usize,
    max_attack_count: usize,
    max_defense_count: usize,
    and_memo_context_observations: usize,
    and_memo_context_collisions: usize,
    and_memo_context_collision_keys: usize,
}

#[derive(Serialize)]
struct RootDepthProfileSummary {
    depth: i32,
    score: i32,
    best_move: Option<[usize; 2]>,
    nodes: usize,
    elapsed_ms: f64,
    stopped: bool,
    candidates: Vec<RootCandidateProfileSummary>,
}

#[derive(Serialize)]
struct RootCandidateProfileSummary {
    index: usize,
    #[serde(rename = "move")]
    move_xy: [usize; 2],
    order_score: i32,
    self_attack: i32,
    opp_attack: i32,
    depthdown: f64,
    atdown: i32,
    attempt_depth: f64,
    score: i32,
    nodes: usize,
    elapsed_ms: f64,
    alpha_before: i32,
    alpha_after: i32,
    beta: i32,
    reason: String,
    zero_window_nodes: usize,
    zero_window_elapsed_ms: f64,
    full_window_nodes: usize,
    full_window_elapsed_ms: f64,
    pvs_research: bool,
}

fn vct_depth_stats_summary(stats: &VCTDepthStats) -> VctDepthStatsSummary {
    VctDepthStatsSummary {
        depth: stats.depth,
        elapsed_ms: stats.elapsed_us as f64 / 1000.0,
        found: stats.found,
        solved: stats.solved,
        or_nodes: stats.or_nodes,
        and_nodes: stats.and_nodes,
        memo_exact_hits: stats.memo_exact_hits,
        memo_shallow_found_hits: stats.memo_shallow_found_hits,
        memo_shallow_solved_hits: stats.memo_shallow_solved_hits,
        attacks_generated: stats.attacks_generated,
        defenses_generated: stats.defenses_generated,
        max_attack_count: stats.max_attack_count,
        max_defense_count: stats.max_defense_count,
        and_memo_context_observations: stats.and_memo_context_observations,
        and_memo_context_collisions: stats.and_memo_context_collisions,
        and_memo_context_collision_keys: stats.and_memo_context_collision_keys,
    }
}

fn vct_and_memo_collision_sample_summary(
    sample: &VCTAndMemoCollisionSample,
) -> VctAndMemoCollisionSampleSummary {
    let attack_move = move_to_xy(sample.attack_move)
        .map(|(x, y)| [x, y])
        .unwrap_or([usize::MAX, usize::MAX]);
    let defenses = sample
        .defenses
        .iter()
        .filter_map(|&move_| move_to_xy(move_).ok())
        .map(|(x, y)| [x, y])
        .collect();
    VctAndMemoCollisionSampleSummary {
        observed_depth: sample.observed_depth,
        current_depth: sample.current_depth,
        board_key: sample.board_key.to_string(),
        observed_signature: sample.observed_signature.to_string(),
        current_signature: sample.current_signature.to_string(),
        attack_move,
        attack_level: sample.attack_level,
        defenses,
    }
}

fn vct_stats_summary(stats: &VCTStats) -> VctStatsSummary {
    VctStatsSummary {
        depth_limit: stats.depth_limit,
        depth_completed: stats.depth_completed,
        elapsed_ms: stats.elapsed_us as f64 / 1000.0,
        or_nodes: stats.or_nodes,
        and_nodes: stats.and_nodes,
        memo_exact_hits: stats.memo_exact_hits,
        memo_shallow_found_hits: stats.memo_shallow_found_hits,
        memo_shallow_solved_hits: stats.memo_shallow_solved_hits,
        attacks_generated: stats.attacks_generated,
        defenses_generated: stats.defenses_generated,
        max_attack_count: stats.max_attack_count,
        max_defense_count: stats.max_defense_count,
        and_memo_context_observations: stats.and_memo_context_observations,
        and_memo_context_collisions: stats.and_memo_context_collisions,
        and_memo_context_collision_keys: stats.and_memo_context_collision_keys,
        and_memo_context_collision_samples: stats
            .and_memo_context_collision_samples
            .iter()
            .map(vct_and_memo_collision_sample_summary)
            .collect(),
        depth_stats: stats
            .depth_stats
            .iter()
            .map(vct_depth_stats_summary)
            .collect(),
    }
}

fn main() {
    let args = parse_args();
    let text = fs::read_to_string(&args.case_path).expect("case file is readable");
    let case: DiffCase = serde_json::from_str(&text).expect("case file is valid JSON");
    let output = run_case(case, args.root_profile);
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("probe output serialises")
    );
}

#[derive(Debug)]
struct ProbeArgs {
    case_path: PathBuf,
    root_profile: bool,
}

fn parse_args() -> ProbeArgs {
    let mut args = std::env::args().skip(1);
    let mut case_path = None;
    let mut root_profile = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--case" => {
                case_path = Some(args.next().expect("--case requires a path").into());
            }
            "--root-profile" => {
                root_profile = true;
            }
            _ => {}
        }
    }
    let Some(case_path) = case_path else {
        eprintln!("usage: diff_probe --case <case.json> [--root-profile]");
        std::process::exit(2);
    };
    ProbeArgs {
        case_path,
        root_profile,
    }
}

fn run_case(case: DiffCase, root_profile: bool) -> ProbeOutput {
    let mut board = Board::with_side_to_move(case.first_side).expect("first_side is valid");
    for [x, y] in &case.moves {
        let move_ = xy_to_move(*x as usize, *y as usize).expect("case move is in range");
        board.play(move_, None).expect("case move is legal");
    }

    let mut config = load_default_config();
    if let Some(v) = case.runtime.compute_vcf {
        config.runtime.compute_vcf = v;
    }
    if let Some(v) = case.runtime.root_vcf_depth {
        config.runtime.root_vcf_depth = v.max(0);
    }
    if let Some(v) = case.runtime.opponent_vcf_depth {
        config.runtime.opponent_vcf_depth = v.max(0);
    }
    if let Some(v) = case.runtime.vct_verify_opponent_vcf_depth {
        config.runtime.vct_verify_opponent_vcf_depth = v.max(0);
    }
    if let Some(v) = case.runtime.vcf_multi_reply {
        config.runtime.vcf_multi_reply = v;
    }
    if let Some(v) = case.runtime.nonroot_vcf {
        config.runtime.nonroot_vcf = v;
    }
    if let Some(v) = case.runtime.compute_vct {
        config.runtime.compute_vct = v;
    }
    if let Some(v) = case.runtime.root_vct_depth {
        config.runtime.root_vct_depth = v.max(0);
    }
    if let Some(v) = case.runtime.vct_strict_and_memo_key {
        config.runtime.vct_strict_and_memo_key = v;
    }
    if let Some(v) = case.runtime.overlap_vct_alphabeta {
        config.runtime.overlap_vct_alphabeta = v;
    }
    if let Some(v) = case.runtime.static_board {
        config.runtime.static_board = v;
    }
    if let Some(v) = case.runtime.dynamic_board_margin {
        config.runtime.dynamic_board_margin = v.max(0);
    }
    config.runtime.root_profile = root_profile;

    let default_limits = SearchLimits::fixed_from_config(&config);
    let limits = SearchLimits {
        max_depth: case.limits.max_depth.unwrap_or(default_limits.max_depth),
        root_width: case.limits.root_width.unwrap_or(default_limits.root_width),
        node_limit: case.limits.node_limit,
        time_limit_ms: case.limits.time_limit_ms,
    };

    let mut searcher = RootSearcher::new(config);
    let result = searcher.search(&mut board, Some(limits));
    let (mx, my) = move_to_xy(result.move_).expect("root move is valid");
    let trace = searcher.last_trace.clone().unwrap_or_default();

    ProbeOutput {
        name: case.name,
        board: BoardSummary {
            side_to_move: board.side_to_move(),
            winner: board.winner(),
            move_count: board.move_count(),
            zobrist_key: board.zobrist_key().to_string(),
        },
        root: RootSummary {
            move_xy: [mx, my],
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
            trace: TraceSummary {
                used_vcf: trace.used_vcf,
                vcf_found: trace.vcf_found,
                used_vct: trace.used_vct,
                vct_triggered: trace.vct_triggered,
                vct_found: trace.vct_found,
                vct_move: trace
                    .vct_move
                    .and_then(|m| move_to_xy(m).ok())
                    .map(|(x, y)| [x, y]),
                vct_accepted: trace.vct_accepted,
                vct_reject_reason: trace.vct_reject_reason.map(|s| s.to_string()),
                vct_ms: trace.vct_ms,
                vct_stats: trace.vct_stats.as_ref().map(vct_stats_summary),
                alphabeta_ms: trace.alphabeta_ms,
                overlap_used: trace.overlap_used,
                overlap_ab_ms: trace.overlap_ab_ms,
                overlap_ab_cancelled: trace.overlap_ab_cancelled,
                overlap_wait_ms: trace.overlap_wait_ms,
                tt_snapshot_ms: trace.tt_snapshot_ms,
                tactical_path: trace.tactical_path.to_string(),
                root_profiles: trace
                    .root_profiles
                    .iter()
                    .map(|profile| RootDepthProfileSummary {
                        depth: profile.depth,
                        score: profile.score,
                        best_move: profile
                            .best_move
                            .and_then(|m| move_to_xy(m).ok())
                            .map(|(x, y)| [x, y]),
                        nodes: profile.nodes,
                        elapsed_ms: profile.elapsed_us as f64 / 1000.0,
                        stopped: profile.stopped,
                        candidates: profile
                            .candidates
                            .iter()
                            .map(|candidate| {
                                let (x, y) =
                                    move_to_xy(candidate.move_).expect("profile move is valid");
                                RootCandidateProfileSummary {
                                    index: candidate.index,
                                    move_xy: [x, y],
                                    order_score: candidate.order_score,
                                    self_attack: candidate.self_attack,
                                    opp_attack: candidate.opp_attack,
                                    depthdown: candidate.depthdown_milli as f64 / 1000.0,
                                    atdown: candidate.atdown,
                                    attempt_depth: candidate.attempt_depth_milli as f64 / 1000.0,
                                    score: candidate.score,
                                    nodes: candidate.nodes,
                                    elapsed_ms: candidate.elapsed_us as f64 / 1000.0,
                                    alpha_before: candidate.alpha_before,
                                    alpha_after: candidate.alpha_after,
                                    beta: candidate.beta,
                                    reason: candidate.reason.to_string(),
                                    zero_window_nodes: candidate.zero_window_nodes,
                                    zero_window_elapsed_ms: candidate.zero_window_elapsed_us as f64
                                        / 1000.0,
                                    full_window_nodes: candidate.full_window_nodes,
                                    full_window_elapsed_ms: candidate.full_window_elapsed_us as f64
                                        / 1000.0,
                                    pvs_research: candidate.pvs_research,
                                }
                            })
                            .collect(),
                    })
                    .collect(),
            },
        },
    }
}
