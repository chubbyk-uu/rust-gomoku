//! Command-line Gomocup engine entrypoint.

use std::io::{self, BufRead, Write};

use rust_gomoku::{load_config_for_profile, EngineProfile, GomocupProtocol, SearchLimits};

fn main() {
    let args = parse_args().unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    let mut config = load_config_for_profile(args.profile);
    if let Some(root_profile) = args.root_profile {
        config.runtime.root_profile = root_profile;
    }
    if let Some(fast_history_ordering) = args.fast_history_ordering {
        config.runtime.fast_history_ordering = fast_history_ordering;
    }
    let search_limits = if args.depth.is_some() || args.width.is_some() {
        let fixed = SearchLimits::fixed_from_config(&config);
        Some(SearchLimits {
            max_depth: args.depth.unwrap_or(fixed.max_depth),
            root_width: args.width.unwrap_or(fixed.root_width),
            ..SearchLimits::default()
        })
    } else {
        None
    };
    let mut protocol = GomocupProtocol::new(Some(config), search_limits);
    protocol.tt_bits = args.tt_bits;

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct CliArgs {
    depth: Option<i32>,
    width: Option<usize>,
    root_profile: Option<bool>,
    fast_history_ordering: Option<bool>,
    tt_bits: Option<u32>,
    profile: EngineProfile,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut depth = None;
    let mut width = None;
    let mut root_profile = None;
    let mut fast_history_ordering = None;
    let mut tt_bits = None;
    let mut profile = EngineProfile::Base;
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
            "--fast-history-ordering" => {
                fast_history_ordering = Some(true);
            }
            "--no-fast-history-ordering" => {
                fast_history_ordering = Some(false);
            }
            "--tt-bits" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<u32>().ok()) {
                    tt_bits = Some(value);
                }
            }
            "--profile" => {
                let Some(value) = args.next() else {
                    return Err("--profile requires base or fast".to_string());
                };
                profile = value.parse::<EngineProfile>()?;
            }
            _ => {}
        }
    }
    Ok(CliArgs {
        depth,
        width,
        root_profile,
        fast_history_ordering,
        tt_bits,
        profile,
    })
}
