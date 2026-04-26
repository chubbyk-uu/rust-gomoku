//! Command-line Gomocup engine entrypoint.

use std::io::{self, BufRead, Write};

use rust_gomoku::{load_default_config, GomocupProtocol, SearchLimits};

fn main() {
    let (depth_override, width_override, root_profile) = parse_args();
    let mut config = load_default_config();
    if let Some(root_profile) = root_profile {
        config.runtime.root_profile = root_profile;
    }
    let search_limits = if depth_override.is_some() || width_override.is_some() {
        let fixed = SearchLimits::fixed_from_config(&config);
        Some(SearchLimits {
            max_depth: depth_override.unwrap_or(fixed.max_depth),
            root_width: width_override.unwrap_or(fixed.root_width),
            ..SearchLimits::default()
        })
    } else {
        None
    };
    let mut protocol = GomocupProtocol::new(Some(config), search_limits);

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

fn parse_args() -> (Option<i32>, Option<usize>, Option<bool>) {
    let mut depth = None;
    let mut width = None;
    let mut root_profile = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--depth" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<i32>().ok()) {
                    depth = Some(value);
                }
            }
            "--width" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<usize>().ok()) {
                    width = Some(value);
                }
            }
            "--root-profile" => {
                root_profile = Some(true);
            }
            "--no-root-profile" => {
                root_profile = Some(false);
            }
            _ => {}
        }
    }
    (depth, width, root_profile)
}
