//! Emit rule-aware move-generation diagnostics for fixed positions.

use std::fs;
use std::path::PathBuf;

use rust_gomoku::{
    diagnose_candidates, load_default_config, move_to_xy, recompute_all_for_rule, xy_to_move,
    Board, CandidateDiagnostic, EvalCaches, ForbiddenKind, RuleSet, BLACK, WHITE,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct ProbeCase {
    #[serde(default = "unnamed")]
    name: String,
    moves: Vec<CaseMove>,
    side_to_move: Option<i8>,
    #[serde(default = "default_rule")]
    rule: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CaseMove {
    Pair([usize; 2]),
    Object { x: usize, y: usize, side: i8 },
}

#[derive(Debug)]
struct Args {
    case_path: Option<PathBuf>,
    case_file: Option<PathBuf>,
}

#[derive(Serialize)]
struct ProbeOutput {
    name: String,
    rule: &'static str,
    side_to_move: &'static str,
    move_count: usize,
    covered_count: usize,
    single_forcing: bool,
    hostile_threat: bool,
    win_priority: bool,
    ordered_candidates: Vec<[usize; 2]>,
    points: Vec<PointOutput>,
}

#[derive(Serialize)]
struct PointOutput {
    point: [usize; 2],
    move_value: f64,
    adjusted_move_value: f64,
    self_attack: i32,
    opponent_attack: i32,
    requires_full_detector: bool,
    forbidden_kind: Option<&'static str>,
    rule_legal: bool,
    retained: bool,
    order_score: Option<f64>,
    final_rank: Option<usize>,
    rejection_reason: Option<&'static str>,
}

fn unnamed() -> String {
    "unnamed".to_string()
}

fn default_rule() -> String {
    "renju".to_string()
}

fn side_name(side: i8) -> &'static str {
    if side == BLACK {
        "black"
    } else {
        "white"
    }
}

fn forbidden_name(kind: ForbiddenKind) -> &'static str {
    match kind {
        ForbiddenKind::None => "none",
        ForbiddenKind::DoubleThree => "double_three",
        ForbiddenKind::DoubleFour => "double_four",
        ForbiddenKind::Overline => "overline",
    }
}

fn point_output(point: &CandidateDiagnostic) -> PointOutput {
    let (x, y) = move_to_xy(point.move_).expect("diagnostic move stays valid");
    PointOutput {
        point: [x, y],
        move_value: point.move_value,
        adjusted_move_value: point.adjusted_move_value,
        self_attack: point.self_attack,
        opponent_attack: point.opp_attack,
        requires_full_detector: point.requires_full_detector,
        forbidden_kind: point.forbidden_kind.map(forbidden_name),
        rule_legal: point.rule_legal,
        retained: point.retained,
        order_score: point.order_score,
        final_rank: point.final_rank,
        rejection_reason: point.rejection_reason,
    }
}

fn usage() -> ! {
    eprintln!("usage: renju_candidate_probe (--case <case.json> | --case-file <cases.jsonl>)");
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let mut result = Args {
        case_path: None,
        case_file: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--case" => result.case_path = args.next().map(PathBuf::from),
            "--case-file" => result.case_file = args.next().map(PathBuf::from),
            _ => usage(),
        }
    }
    if result.case_path.is_some() == result.case_file.is_some() {
        usage();
    }
    result
}

fn expected_side(index: usize) -> i8 {
    if index % 2 == 0 {
        BLACK
    } else {
        WHITE
    }
}

fn run_case(case: ProbeCase) -> Result<ProbeOutput, String> {
    let rule = case.rule.parse::<RuleSet>()?;
    let mut board = Board::new();
    for (index, case_move) in case.moves.iter().enumerate() {
        let side = expected_side(index);
        let (x, y) = match case_move {
            CaseMove::Pair([x, y]) => (*x, *y),
            CaseMove::Object {
                x,
                y,
                side: declared,
            } => {
                if *declared != side {
                    return Err(format!(
                        "{}: move {} declares side {}, expected {}",
                        case.name,
                        index + 1,
                        declared,
                        side
                    ));
                }
                (*x, *y)
            }
        };
        let move_ = xy_to_move(x, y)
            .map_err(|_| format!("{}: move {} is out of bounds", case.name, index + 1))?;
        board
            .play(move_, Some(side))
            .map_err(|err| format!("{}: move {} is illegal: {err:?}", case.name, index + 1))?;
    }
    if case
        .side_to_move
        .is_some_and(|side| side != board.side_to_move())
    {
        return Err(format!(
            "{}: side_to_move does not match the move sequence",
            case.name
        ));
    }

    let mut config = load_default_config();
    config.rule_set = rule;
    let mut caches = EvalCaches::new();
    recompute_all_for_rule(&mut board, &mut caches, rule);
    let diagnostics = diagnose_candidates(&board, &caches, board.side_to_move(), &config);
    let ordered_candidates = diagnostics
        .ordered_candidates
        .iter()
        .map(|candidate| {
            let (x, y) = move_to_xy(candidate.move_).expect("candidate move stays valid");
            [x, y]
        })
        .collect();

    Ok(ProbeOutput {
        name: case.name,
        rule: rule.as_str(),
        side_to_move: side_name(board.side_to_move()),
        move_count: board.move_count(),
        covered_count: diagnostics.covered_count,
        single_forcing: diagnostics.single_forcing,
        hostile_threat: diagnostics.hostile_threat,
        win_priority: diagnostics.win_priority,
        ordered_candidates,
        points: diagnostics.points.iter().map(point_output).collect(),
    })
}

fn parse_jsonl(path: &PathBuf, text: &str) -> Result<Vec<ProbeCase>, String> {
    let mut cases = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        cases.push(serde_json::from_str(line).map_err(|err| {
            format!(
                "{}:{}: invalid JSON fixture: {err}",
                path.display(),
                line_index + 1
            )
        })?);
    }
    Ok(cases)
}

fn main() {
    let args = parse_args();
    let path = args.case_path.as_ref().or(args.case_file.as_ref()).unwrap();
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        eprintln!("failed to read {}: {err}", path.display());
        std::process::exit(1);
    });
    let cases = if args.case_file.is_some() {
        parse_jsonl(path, &text)
    } else {
        serde_json::from_str(&text)
            .map(|case| vec![case])
            .map_err(|err| format!("{}: invalid JSON case: {err}", path.display()))
    }
    .unwrap_or_else(|err| {
        eprintln!("{err}");
        std::process::exit(1);
    });

    for case in cases {
        let output = run_case(case).unwrap_or_else(|err| {
            eprintln!("{err}");
            std::process::exit(1);
        });
        println!(
            "{}",
            serde_json::to_string(&output).expect("probe output serializes")
        );
    }
}
