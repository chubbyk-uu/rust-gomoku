//! Emit Rust probe output for a fixed differential test case.

use std::fs;
use std::path::PathBuf;

use rust_gomoku::{load_default_config, move_to_xy, xy_to_move, Board, RootSearcher, SearchLimits};

#[derive(Debug)]
struct DiffCase {
    name: String,
    moves: Vec<(usize, usize)>,
    first_side: i8,
    max_depth: i32,
    root_width: usize,
    compute_vcf: bool,
    compute_vct: bool,
    root_vct_depth: i32,
    static_board: bool,
}

fn main() {
    let path = parse_case_path();
    let text = fs::read_to_string(&path).expect("case file is readable");
    let case = parse_case(&text);
    println!("{}", run_case(&case));
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

fn run_case(case: &DiffCase) -> String {
    let mut board = Board::with_side_to_move(case.first_side).expect("first_side is valid");
    for &(x, y) in &case.moves {
        let move_ = xy_to_move(x, y).expect("case move is in range");
        board.play(move_, None).expect("case move is legal");
    }

    let mut config = load_default_config();
    config.runtime.compute_vcf = case.compute_vcf;
    config.runtime.compute_vct = case.compute_vct;
    config.runtime.root_vct_depth = case.root_vct_depth;
    config.runtime.static_board = case.static_board;

    let limits = SearchLimits {
        max_depth: case.max_depth,
        root_width: case.root_width,
        ..SearchLimits::default()
    };
    let mut searcher = RootSearcher::new(config);
    let result = searcher.search(&mut board, Some(limits));
    let (mx, my) = move_to_xy(result.move_).expect("root move is valid");
    let trace = searcher.last_trace.clone().unwrap_or_default();
    let vct_move = trace
        .vct_move
        .and_then(|m| move_to_xy(m).ok())
        .map(|(x, y)| format!("[{},{}]", x, y))
        .unwrap_or_else(|| "null".to_string());
    let reason = trace
        .vct_reject_reason
        .map(|value| format!("\"{}\"", escape_json(value)))
        .unwrap_or_else(|| "null".to_string());

    format!(
        concat!(
            "{{\n",
            "  \"name\": \"{}\",\n",
            "  \"board\": {{\n",
            "    \"side_to_move\": {},\n",
            "    \"winner\": {},\n",
            "    \"move_count\": {},\n",
            "    \"zobrist_key\": \"{}\"\n",
            "  }},\n",
            "  \"root\": {{\n",
            "    \"move\": [{}, {}],\n",
            "    \"score\": {},\n",
            "    \"depth\": {},\n",
            "    \"nodes\": {},\n",
            "    \"trace\": {{\n",
            "      \"used_vcf\": {},\n",
            "      \"vcf_found\": {},\n",
            "      \"used_vct\": {},\n",
            "      \"vct_triggered\": {},\n",
            "      \"vct_found\": {},\n",
            "      \"vct_move\": {},\n",
            "      \"vct_accepted\": {},\n",
            "      \"vct_reject_reason\": {},\n",
            "      \"tactical_path\": \"{}\"\n",
            "    }}\n",
            "  }}\n",
            "}}"
        ),
        escape_json(&case.name),
        board.side_to_move(),
        board.winner(),
        board.move_count(),
        board.zobrist_key(),
        mx,
        my,
        result.score,
        result.depth,
        result.nodes,
        trace.used_vcf,
        trace.vcf_found,
        trace.used_vct,
        trace.vct_triggered,
        trace.vct_found,
        vct_move,
        trace.vct_accepted,
        reason,
        escape_json(trace.tactical_path)
    )
}

fn parse_case(text: &str) -> DiffCase {
    DiffCase {
        name: parse_string(text, "name").expect("case has name"),
        moves: parse_moves(text),
        first_side: parse_i64(text, "first_side").unwrap_or(1) as i8,
        max_depth: parse_i64(text, "max_depth").unwrap_or(6) as i32,
        root_width: parse_i64(text, "root_width").unwrap_or(20) as usize,
        compute_vcf: parse_bool(text, "compute_vcf").unwrap_or(true),
        compute_vct: parse_bool(text, "compute_vct").unwrap_or(true),
        root_vct_depth: parse_i64(text, "root_vct_depth").unwrap_or(4).max(0) as i32,
        static_board: parse_bool(text, "static_board").unwrap_or(true),
    }
}

fn parse_string(text: &str, key: &str) -> Option<String> {
    let marker = format!("\"{}\"", key);
    let start = text.find(&marker)?;
    let after_key = &text[start + marker.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let value = after_colon.strip_prefix('"')?;
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

fn parse_i64(text: &str, key: &str) -> Option<i64> {
    let marker = format!("\"{}\"", key);
    let start = text.find(&marker)?;
    let after_key = &text[start + marker.len()..];
    let colon = after_key.find(':')?;
    let chars = after_key[colon + 1..].trim_start().chars();
    let mut raw = String::new();
    for c in chars {
        if c == '-' || c.is_ascii_digit() {
            raw.push(c);
        } else if !raw.is_empty() {
            break;
        }
    }
    raw.parse().ok()
}

fn parse_bool(text: &str, key: &str) -> Option<bool> {
    let marker = format!("\"{}\"", key);
    let start = text.find(&marker)?;
    let after_key = &text[start + marker.len()..];
    let colon = after_key.find(':')?;
    let value = after_key[colon + 1..].trim_start();
    if value.starts_with("true") {
        Some(true)
    } else if value.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_moves(text: &str) -> Vec<(usize, usize)> {
    let marker = "\"moves\"";
    let start = text.find(marker).expect("case has moves");
    let after_key = &text[start + marker.len()..];
    let colon = after_key.find(':').expect("moves has colon");
    let mut chars = after_key[colon + 1..].chars().peekable();
    let mut depth = 0_i32;
    let mut nums = Vec::new();
    let mut raw = String::new();
    while let Some(c) = chars.next() {
        match c {
            '[' => depth += 1,
            ']' => {
                if !raw.is_empty() {
                    nums.push(raw.parse::<usize>().expect("move number parses"));
                    raw.clear();
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '-' | '0'..='9' => raw.push(c),
            _ => {
                if !raw.is_empty() {
                    nums.push(raw.parse::<usize>().expect("move number parses"));
                    raw.clear();
                }
            }
        }
    }
    nums.chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
