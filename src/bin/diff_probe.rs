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
    compute_vct: Option<bool>,
    root_vct_depth: Option<i32>,
    static_board: Option<bool>,
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
    tactical_path: String,
}

fn main() {
    let path = parse_case_path();
    let text = fs::read_to_string(&path).expect("case file is readable");
    let case: DiffCase = serde_json::from_str(&text).expect("case file is valid JSON");
    let output = run_case(case);
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("probe output serialises")
    );
}

fn parse_case_path() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--case" {
            return args.next().expect("--case requires a path").into();
        }
    }
    eprintln!("usage: diff_probe --case <case.json>");
    std::process::exit(2);
}

fn run_case(case: DiffCase) -> ProbeOutput {
    let mut board = Board::with_side_to_move(case.first_side).expect("first_side is valid");
    for [x, y] in &case.moves {
        let move_ = xy_to_move(*x as usize, *y as usize).expect("case move is in range");
        board.play(move_, None).expect("case move is legal");
    }

    let mut config = load_default_config();
    if let Some(v) = case.runtime.compute_vcf {
        config.runtime.compute_vcf = v;
    }
    if let Some(v) = case.runtime.compute_vct {
        config.runtime.compute_vct = v;
    }
    if let Some(v) = case.runtime.root_vct_depth {
        config.runtime.root_vct_depth = v.max(0);
    }
    if let Some(v) = case.runtime.static_board {
        config.runtime.static_board = v;
    }

    let limits = SearchLimits {
        max_depth: case.limits.max_depth.unwrap_or(config.root_search.depth),
        root_width: case
            .limits
            .root_width
            .unwrap_or(config.root_search.wide as usize),
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
                tactical_path: trace.tactical_path.to_string(),
            },
        },
    }
}
