//! Emit Rust probe output for a fixed differential test case.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use rust_gomoku::{load_default_config, move_to_xy, xy_to_move, Board, RootSearcher, SearchLimits};

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
    nonroot_vcf: Option<bool>,
    compute_vct: Option<bool>,
    root_vct_depth: Option<i32>,
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
    score: i32,
    nodes: usize,
    elapsed_ms: f64,
    alpha_before: i32,
    alpha_after: i32,
    beta: i32,
    reason: String,
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
    if let Some(v) = case.runtime.nonroot_vcf {
        config.runtime.nonroot_vcf = v;
    }
    if let Some(v) = case.runtime.compute_vct {
        config.runtime.compute_vct = v;
    }
    if let Some(v) = case.runtime.root_vct_depth {
        config.runtime.root_vct_depth = v.max(0);
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
                                    score: candidate.score,
                                    nodes: candidate.nodes,
                                    elapsed_ms: candidate.elapsed_us as f64 / 1000.0,
                                    alpha_before: candidate.alpha_before,
                                    alpha_after: candidate.alpha_after,
                                    beta: candidate.beta,
                                    reason: candidate.reason.to_string(),
                                }
                            })
                            .collect(),
                    })
                    .collect(),
            },
        },
    }
}
