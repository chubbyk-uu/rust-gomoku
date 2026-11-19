//! Command-line Gomocup engine entrypoint.

use std::io::{self, BufRead, Write};

use rust_gomoku::{load_default_config, GomocupProtocol, SearchLimits, MAX_TT_BUCKET_BITS};

fn main() {
    let args = parse_args().unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    let mut config = load_default_config();
    if let Some(root_profile) = args.root_profile {
        config.runtime.root_profile = root_profile;
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
    tt_bits: Option<u32>,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut depth = None;
    let mut width = None;
    let mut root_profile = None;
    let mut tt_bits = None;
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
            "--tt-bits" => {
                let Some(value) = args.next() else {
                    return Err("--tt-bits requires an integer".to_string());
                };
                let bits = value
                    .parse::<u32>()
                    .map_err(|_| format!("invalid --tt-bits: {value}"))?;
                if bits > MAX_TT_BUCKET_BITS {
                    return Err(format!(
                        "--tt-bits must be between 0 and {MAX_TT_BUCKET_BITS}"
                    ));
                }
                tt_bits = Some(bits);
            }
            "--profile" | "--fast-history-ordering" | "--no-fast-history-ordering" => {
                return Err(format!("{arg} was removed with the Fast profile"));
            }
            _ if arg.starts_with("--profile=") => {
                return Err("--profile was removed with the Fast profile".to_string());
            }
            _ => {}
        }
    }
    Ok(CliArgs {
        depth,
        width,
        root_profile,
        tt_bits,
    })
}
