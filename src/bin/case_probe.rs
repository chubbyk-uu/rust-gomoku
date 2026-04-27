//! Emit a single-search probe for a match-case JSON object.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use rust_gomoku::{
    load_config_for_profile, move_to_xy, xy_to_move, Board, EngineProfile, RootSearcher,
    SearchLimits, TranspositionTable, VCTAndMemoCollisionSample, VCTDepthStats, VCTStats, BLACK,
    WHITE,
};

#[derive(Debug, Deserialize)]
struct MatchCase {
    name: Option<String>,
    moves: Vec<CaseMove>,
    side_to_move: Option<CaseSide>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CaseMove {
    Pair([i32; 2]),
    Object {
        x: i32,
        y: i32,
        side: Option<CaseSide>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
enum CaseSide {
    Text(CaseSideText),
    Value(i8),
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CaseSideText {
    Black,
    White,
}

impl CaseSide {
    fn to_side(self) -> Result<i8, String> {
        match self {
            Self::Text(CaseSideText::Black) | Self::Value(BLACK) => Ok(BLACK),
            Self::Text(CaseSideText::White) | Self::Value(WHITE) => Ok(WHITE),
            Self::Value(other) => Err(format!("invalid side value: {other}")),
        }
    }
}

#[derive(Debug)]
struct ProbeArgs {
    case_path: Option<PathBuf>,
    case_json: Option<String>,
    profile: EngineProfile,
    depth: Option<i32>,
    width: Option<usize>,
    tt_bits: Option<u32>,
    root_profile: bool,
    vct_strict_and_memo_key: Option<bool>,
}

#[derive(Serialize)]
struct ProbeOutput {
    case_name: String,
    tags: Vec<String>,
    prefix_plies: usize,
    side_to_move: String,
    profile: String,
    tt_bits: Option<u32>,
    limits: LimitsOutput,
    result: ResultOutput,
    trace: TraceOutput,
}

#[derive(Serialize)]
struct LimitsOutput {
    max_depth: i32,
    root_width: usize,
}

#[derive(Serialize)]
struct ResultOutput {
    #[serde(rename = "move")]
    move_xy: [usize; 2],
    score: i32,
    depth: i32,
    nodes: usize,
    elapsed_ms: f64,
}

#[derive(Serialize)]
struct TraceOutput {
    used_vcf: bool,
    vcf_found: bool,
    used_vct: bool,
    vct_triggered: bool,
    vct_found: bool,
    tactical_path: String,
    vct_ms: Option<f64>,
    vct_stats: Option<VctStatsOutput>,
    alphabeta_ms: Option<f64>,
    root_profile_depths: usize,
}

#[derive(Serialize)]
struct VctStatsOutput {
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
    and_memo_context_collision_samples: Vec<VctAndMemoCollisionSampleOutput>,
    depth_stats: Vec<VctDepthStatsOutput>,
}

#[derive(Serialize)]
struct VctAndMemoCollisionSampleOutput {
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
struct VctDepthStatsOutput {
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

fn vct_depth_stats_output(stats: &VCTDepthStats) -> VctDepthStatsOutput {
    VctDepthStatsOutput {
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

fn vct_and_memo_collision_sample_output(
    sample: &VCTAndMemoCollisionSample,
) -> VctAndMemoCollisionSampleOutput {
    let attack_move = move_to_xy(sample.attack_move)
        .map(|(x, y)| [x, y])
        .unwrap_or([usize::MAX, usize::MAX]);
    let defenses = sample
        .defenses
        .iter()
        .filter_map(|&move_| move_to_xy(move_).ok())
        .map(|(x, y)| [x, y])
        .collect();
    VctAndMemoCollisionSampleOutput {
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

fn vct_stats_output(stats: &VCTStats) -> VctStatsOutput {
    VctStatsOutput {
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
            .map(vct_and_memo_collision_sample_output)
            .collect(),
        depth_stats: stats
            .depth_stats
            .iter()
            .map(vct_depth_stats_output)
            .collect(),
    }
}

fn main() {
    let args = parse_args().unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    let case = read_case(&args).unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    let output = run_case(case, &args).unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(1);
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("probe output serialises")
    );
}

fn parse_args() -> Result<ProbeArgs, String> {
    let mut args = std::env::args().skip(1);
    let mut case_path = None;
    let mut case_json = None;
    let mut profile = EngineProfile::Base;
    let mut depth = None;
    let mut width = None;
    let mut tt_bits = None;
    let mut root_profile = false;
    let mut vct_strict_and_memo_key = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--case" => {
                case_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--case requires a path".to_string())?,
                ));
            }
            "--case-json" => {
                case_json = Some(
                    args.next()
                        .ok_or_else(|| "--case-json requires a JSON object".to_string())?,
                );
            }
            "--profile" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--profile requires base or fast".to_string())?;
                profile = value.parse::<EngineProfile>()?;
            }
            "--depth" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--depth requires an integer".to_string())?;
                depth = Some(
                    value
                        .parse::<i32>()
                        .map_err(|_| format!("invalid --depth: {value}"))?,
                );
            }
            "--width" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--width requires an integer".to_string())?;
                width = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --width: {value}"))?,
                );
            }
            "--tt-bits" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--tt-bits requires an integer".to_string())?;
                tt_bits = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid --tt-bits: {value}"))?,
                );
            }
            "--root-profile" => {
                root_profile = true;
            }
            "--vct-strict-and-memo-key" => {
                vct_strict_and_memo_key = Some(true);
            }
            "--no-vct-strict-and-memo-key" => {
                vct_strict_and_memo_key = Some(false);
            }
            _ => {}
        }
    }
    if case_path.is_some() == case_json.is_some() {
        return Err("provide exactly one of --case or --case-json".to_string());
    }
    Ok(ProbeArgs {
        case_path,
        case_json,
        profile,
        depth,
        width,
        tt_bits,
        root_profile,
        vct_strict_and_memo_key,
    })
}

fn read_case(args: &ProbeArgs) -> Result<MatchCase, String> {
    let text = if let Some(path) = &args.case_path {
        fs::read_to_string(path).map_err(|err| format!("failed to read {path:?}: {err}"))?
    } else {
        args.case_json
            .clone()
            .ok_or_else(|| "missing case JSON".to_string())?
    };
    serde_json::from_str(&text).map_err(|err| format!("invalid case JSON: {err}"))
}

fn side_for_index(index: usize) -> i8 {
    if index % 2 == 0 {
        BLACK
    } else {
        WHITE
    }
}

fn side_name(side: i8) -> &'static str {
    if side == BLACK {
        "black"
    } else {
        "white"
    }
}

fn move_xy(move_: &CaseMove) -> (i32, i32, Option<CaseSide>) {
    match move_ {
        CaseMove::Pair([x, y]) => (*x, *y, None),
        CaseMove::Object { x, y, side } => (*x, *y, *side),
    }
}

fn run_case(case: MatchCase, args: &ProbeArgs) -> Result<ProbeOutput, String> {
    let mut board = Board::new();
    for (index, move_) in case.moves.iter().enumerate() {
        let (x, y, side) = move_xy(move_);
        let expected_side = side_for_index(index);
        if let Some(side) = side {
            let parsed = side.to_side()?;
            if parsed != expected_side {
                return Err(format!("move {} side does not alternate", index + 1));
            }
        }
        let move_ = xy_to_move(x as usize, y as usize)
            .map_err(|_| format!("move {} is out of bounds: [{x},{y}]", index + 1))?;
        board
            .play(move_, Some(expected_side))
            .map_err(|err| format!("move {} is illegal: {err:?}", index + 1))?;
    }
    let expected_side = side_for_index(case.moves.len());
    if let Some(side) = case.side_to_move {
        let parsed = side.to_side()?;
        if parsed != expected_side {
            return Err(format!(
                "side_to_move {} does not match {} moves",
                side_name(parsed),
                case.moves.len()
            ));
        }
    }

    let mut config = load_config_for_profile(args.profile);
    config.runtime.root_profile = args.root_profile;
    if let Some(vct_strict_and_memo_key) = args.vct_strict_and_memo_key {
        config.runtime.vct_strict_and_memo_key = vct_strict_and_memo_key;
    }
    let default_limits = SearchLimits::fixed_from_config(&config);
    let limits = SearchLimits {
        max_depth: args.depth.unwrap_or(default_limits.max_depth),
        root_width: args.width.unwrap_or(default_limits.root_width),
        ..SearchLimits::default()
    };
    let mut searcher = if let Some(bits) = args.tt_bits {
        RootSearcher::with_tt(config, TranspositionTable::new(bits))
    } else {
        RootSearcher::new(config)
    };
    let start = Instant::now();
    let result = searcher.search(&mut board, Some(limits));
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let (mx, my) =
        move_to_xy(result.move_).map_err(|err| format!("invalid result move: {err:?}"))?;
    let trace = searcher.last_trace.clone().unwrap_or_default();

    Ok(ProbeOutput {
        case_name: case.name.unwrap_or_else(|| "unnamed".to_string()),
        tags: case.tags,
        prefix_plies: board.move_count(),
        side_to_move: side_name(board.side_to_move()).to_string(),
        profile: args.profile.as_str().to_string(),
        tt_bits: args.tt_bits,
        limits: LimitsOutput {
            max_depth: limits.max_depth,
            root_width: limits.root_width,
        },
        result: ResultOutput {
            move_xy: [mx, my],
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
            elapsed_ms,
        },
        trace: TraceOutput {
            used_vcf: trace.used_vcf,
            vcf_found: trace.vcf_found,
            used_vct: trace.used_vct,
            vct_triggered: trace.vct_triggered,
            vct_found: trace.vct_found,
            tactical_path: trace.tactical_path.to_string(),
            vct_ms: trace.vct_ms,
            vct_stats: trace.vct_stats.as_ref().map(vct_stats_output),
            alphabeta_ms: trace.alphabeta_ms,
            root_profile_depths: trace.root_profiles.len(),
        },
    })
}
