//! Command-line Gomocup engine entrypoint.

use std::io::{self, BufRead, Write};

use rust_gomoku::{GomocupProtocol, SearchLimits};

const DEFAULT_DEPTH: i32 = 6;
const DEFAULT_WIDTH: usize = 20;

fn main() {
    let (depth, width) = parse_args();
    let mut protocol = GomocupProtocol::new(
        None,
        Some(SearchLimits {
            max_depth: depth,
            root_width: width,
            ..SearchLimits::default()
        }),
    );

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        for response in protocol.handle_line(&line) {
            writeln!(stdout, "{response}").expect("stdout write succeeds");
            stdout.flush().expect("stdout flush succeeds");
        }
        if protocol.ended {
            break;
        }
    }
}

fn parse_args() -> (i32, usize) {
    let mut depth = DEFAULT_DEPTH;
    let mut width = DEFAULT_WIDTH;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--depth" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<i32>().ok()) {
                    depth = value;
                }
            }
            "--width" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<usize>().ok()) {
                    width = value;
                }
            }
            _ => {}
        }
    }
    (depth, width)
}
