use std::env;
use std::fs;
use std::process;

use rust_gomoku::{classify_forbidden_stones, ForbiddenKind, RuleSet, BLACK};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    name: String,
    moves: Vec<Stone>,
    candidate: Point,
}

#[derive(Deserialize)]
struct Stone {
    x: usize,
    y: usize,
    side: i8,
}

#[derive(Deserialize)]
struct Point {
    x: usize,
    y: usize,
}

fn kind_name(kind: ForbiddenKind) -> &'static str {
    match kind {
        ForbiddenKind::None => "none",
        ForbiddenKind::DoubleThree => "double_three",
        ForbiddenKind::DoubleFour => "double_four",
        ForbiddenKind::Overline => "overline",
    }
}

fn usage() -> ! {
    eprintln!("usage: renju_rule_probe --case-file <fixtures.jsonl>");
    process::exit(2);
}

fn parse_case_file_arg() -> String {
    let mut args = env::args().skip(1);
    let Some(flag) = args.next() else {
        usage();
    };
    if flag != "--case-file" {
        usage();
    }
    let Some(path) = args.next() else {
        usage();
    };
    if args.next().is_some() {
        usage();
    }
    path
}

fn main() {
    let path = parse_case_file_arg();
    let text = fs::read_to_string(&path).unwrap_or_else(|err| {
        eprintln!("failed to read {path}: {err}");
        process::exit(1);
    });

    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fixture: Fixture = serde_json::from_str(line).unwrap_or_else(|err| {
            eprintln!("{}:{}: invalid JSON fixture: {err}", path, line_index + 1);
            process::exit(1);
        });
        let stones: Vec<_> = fixture
            .moves
            .iter()
            .map(|stone| (stone.x, stone.y, stone.side))
            .collect();
        let kind = classify_forbidden_stones(
            &stones,
            (fixture.candidate.x, fixture.candidate.y),
            BLACK,
            RuleSet::Renju,
        )
        .unwrap_or_else(|err| {
            eprintln!("{}: classify failed: {err:?}", fixture.name);
            process::exit(1);
        });
        println!("{} {}", fixture.name, kind_name(kind));
    }
}
