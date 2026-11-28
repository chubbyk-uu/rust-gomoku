//! Local browser GUI for playing against the engine.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(not(target_os = "windows"))]
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use rust_gomoku::{
    default_interactive_threads, load_default_config, nnue::resolve_default_nnue_dir,
    EngineSearchCompletion, GameController, GameMode, RuleSet, SearchClockMode, SearchDifficulty,
    Side, UiLanguage, BLACK, MAX_SEARCH_THREADS, WHITE,
};

const DEFAULT_ADDR: &str = "127.0.0.1:18080";
const BROWSER_EXIT_GRACE: Duration = Duration::from_secs(30);
const BROWSER_EXIT_CHECK_INTERVAL: Duration = Duration::from_secs(5);

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("warning: debug build is much slower; use `cargo run --release --bin gomoku_gui` for real play.");
    }
    let args = parse_args();
    let mut config = load_default_config();
    SearchDifficulty::default().apply_to_config(&mut config);
    let default_multi_threads = args
        .threads
        .filter(|&threads| threads > 1)
        .map(|threads| threads.clamp(1, MAX_SEARCH_THREADS))
        .unwrap_or_else(default_interactive_threads);
    config.runtime.threads = args
        .threads
        .map(|threads| threads.clamp(1, MAX_SEARCH_THREADS))
        .unwrap_or(default_multi_threads);
    // VCT/AB overlap forks a full TT snapshot and competes with ABDADA workers;
    // keep it enabled only for single-threaded desktop play.
    config.runtime.overlap_vct_alphabeta = config.runtime.threads == 1;
    if !args.no_nnue {
        if let Some(nnue_dir) = resolve_default_nnue_dir(args.nnue_dir.as_deref()) {
            let dir_str = nnue_dir.to_string_lossy().into_owned();
            eprintln!("gomoku gui NNUE weights dir: {dir_str}");
            config.runtime.nnue_dir = Some(dir_str);
            config.runtime.use_nnue = true;
        } else {
            eprintln!("gomoku gui NNUE weights not found; Pro mode will use classical fallback.");
        }
    }
    let initial_clock = if args.depth.is_some() {
        SearchClockMode::Untimed
    } else {
        SearchClockMode::Timed
    };
    if let Some(depth) = args.depth {
        config.root_search.depth = depth;
    }
    if let Some(width) = args.width {
        config.root_search.wide = width;
        config.root_search.timed_max_wide = width;
    }
    let state = Arc::new(Mutex::new(
        GameController::new_with_clock(config, initial_clock).with_nnue_history(),
    ));
    let browser_activity = Arc::new(Mutex::new(BrowserActivity::default()));
    start_browser_watchdog(Arc::clone(&browser_activity));
    let system_language = system_ui_language();
    let addr = args.addr;
    let listener = TcpListener::bind(&addr).expect("GUI server binds to local address");
    let url = browser_url(&addr);
    eprintln!("gomoku gui listening on {url}");
    if args.open_browser {
        if let Err(err) = open_browser(&url) {
            eprintln!("warning: failed to open the browser automatically: {err}");
        }
    }
    minimize_console_window();
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let state = Arc::clone(&state);
        let browser_activity = Arc::clone(&browser_activity);
        thread::spawn(move || {
            handle_client(
                stream,
                state,
                browser_activity,
                default_multi_threads,
                system_language,
            )
        });
    }
}

#[derive(Default)]
struct BrowserActivity {
    tabs: HashSet<String>,
    exit_after: Option<Instant>,
}

impl BrowserActivity {
    fn open(&mut self, tab: &str) {
        self.tabs.insert(tab.to_owned());
        self.exit_after = None;
    }

    fn close(&mut self, tab: &str, now: Instant) {
        if self.tabs.remove(tab) && self.tabs.is_empty() {
            self.exit_after = Some(now + BROWSER_EXIT_GRACE);
        }
    }

    fn should_exit(&self, now: Instant) -> bool {
        self.exit_after.is_some_and(|deadline| now >= deadline)
    }
}

fn start_browser_watchdog(browser_activity: Arc<Mutex<BrowserActivity>>) {
    thread::spawn(move || loop {
        thread::sleep(BROWSER_EXIT_CHECK_INTERVAL);
        let activity = browser_activity.lock().expect("browser activity lock");
        if activity.should_exit(Instant::now()) {
            eprintln!("all browser pages closed; exiting GUI server");
            std::process::exit(0);
        }
    });
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn minimize_console_window() {
    use std::ffi::c_void;

    const SW_MINIMIZE: i32 = 6;

    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut c_void;
        fn ShowWindow(hwnd: *mut c_void, n_cmd_show: i32) -> i32;
    }

    unsafe {
        let hwnd = GetConsoleWindow();
        if !hwnd.is_null() {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
    }
}

#[cfg(not(all(target_os = "windows", not(debug_assertions))))]
fn minimize_console_window() {}

struct GuiArgs {
    addr: String,
    depth: Option<i32>,
    width: Option<i32>,
    threads: Option<usize>,
    nnue_dir: Option<String>,
    no_nnue: bool,
    open_browser: bool,
}

fn parse_args() -> GuiArgs {
    let mut result = GuiArgs {
        addr: DEFAULT_ADDR.to_string(),
        depth: None,
        width: None,
        threads: None,
        nnue_dir: None,
        no_nnue: false,
        open_browser: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => {
                if let Some(value) = args.next() {
                    result.addr = value;
                }
            }
            "--depth" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<i32>().ok()) {
                    result.depth = Some(value.max(1));
                }
            }
            "--width" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<i32>().ok()) {
                    result.width = Some(value.max(1));
                }
            }
            "--threads" => {
                if let Some(value) = args.next().and_then(|value| value.parse::<usize>().ok()) {
                    result.threads = Some(value.clamp(1, MAX_SEARCH_THREADS));
                }
            }
            "--nnue-dir" => {
                if let Some(value) = args.next() {
                    result.nnue_dir = Some(value);
                }
            }
            "--no-nnue" => {
                result.no_nnue = true;
            }
            "--no-open-browser" => {
                result.open_browser = false;
            }
            _ => {}
        }
    }
    result
}

fn browser_url(addr: &str) -> String {
    if let Some(port) = addr.strip_prefix("0.0.0.0:") {
        format!("http://127.0.0.1:{port}")
    } else if let Some(port) = addr.strip_prefix("[::]:") {
        format!("http://[::1]:{port}")
    } else {
        format!("http://{addr}")
    }
}

#[cfg(not(target_os = "windows"))]
fn spawn_browser(program: &str, args: &[&str]) -> std::io::Result<()> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "windows")]
fn open_browser(url: &str) -> std::io::Result<()> {
    use std::ffi::c_void;
    use std::ptr::null;
    use std::ptr::null_mut;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_cmd: i32,
        ) -> *mut c_void;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
        fn CoUninitialize();
    }

    const COINIT_APARTMENTTHREADED: u32 = 2;
    const COINIT_DISABLE_OLE1DDE: u32 = 4;
    const SW_SHOWNORMAL: i32 = 1;
    let file = browser_url_utf16(url)?;
    let operation = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];

    // Shell handlers may require STA COM; balance both S_OK and S_FALSE.
    let initialized = unsafe {
        CoInitializeEx(
            null_mut(),
            COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
        )
    };
    if initialized < 0 {
        return Err(std::io::Error::other(format!(
            "browser COM initialization failed (HRESULT {:#010x})",
            initialized as u32
        )));
    }

    // Both UTF-16 buffers remain alive and terminated for the synchronous call.
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    unsafe { CoUninitialize() };
    shell_execute_result(result as isize)
}

#[cfg(any(target_os = "windows", test))]
fn browser_url_utf16(url: &str) -> std::io::Result<Vec<u16>> {
    if url.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "browser URL contains a null character",
        ));
    }
    Ok(url.encode_utf16().chain(std::iter::once(0)).collect())
}

#[cfg(any(target_os = "windows", test))]
fn shell_execute_result(result: isize) -> std::io::Result<()> {
    if result > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW failed to open browser (code {result})"
        )))
    }
}

#[cfg(target_os = "macos")]
fn open_browser(url: &str) -> std::io::Result<()> {
    spawn_browser("open", &[url])
}

#[cfg(target_os = "linux")]
fn open_browser(url: &str) -> std::io::Result<()> {
    let is_wsl = std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|release| release.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);
    if is_wsl && spawn_browser("cmd.exe", &["/C", "start", "", url]).is_ok() {
        return Ok(());
    }
    spawn_browser("xdg-open", &[url])
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn open_browser(_url: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "automatic browser opening is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn system_ui_language() -> UiLanguage {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }

    const LANG_CHINESE: u16 = 0x04;
    // The low ten bits of a Windows LANGID are its primary language ID.
    if unsafe { GetUserDefaultUILanguage() } & 0x03ff == LANG_CHINESE {
        UiLanguage::Chinese
    } else {
        UiLanguage::English
    }
}

#[cfg(not(windows))]
fn system_ui_language() -> UiLanguage {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .map_or(UiLanguage::English, |tag| UiLanguage::from_tag(&tag))
}

fn handle_client(
    mut stream: TcpStream,
    state: Arc<Mutex<GameController>>,
    browser_activity: Arc<Mutex<BrowserActivity>>,
    default_multi_threads: usize,
    system_language: UiLanguage,
) {
    let mut buffer = [0_u8; 8192];
    let Ok(read) = stream.read(&mut buffer) else {
        return;
    };
    if read == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buffer[..read]);
    let Some(first_line) = request.lines().next() else {
        return;
    };
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let (path, query) = split_target(target);
    let language = query_param(query, "lang").map_or(system_language, UiLanguage::from_tag);
    let response = match (method, path) {
        ("GET", "/") if system_language == UiLanguage::English => {
            html_response(&INDEX_HTML.replacen("<html lang=\"zh-CN\">", "<html lang=\"en\">", 1))
        }
        ("GET", "/") => html_response(INDEX_HTML),
        ("GET", "/ui_language.js") => response(
            200,
            "text/javascript; charset=utf-8",
            include_str!("../../android/app/src/main/assets/ui_language.js").to_string(),
        ),
        ("GET", "/state") => {
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        ("POST", "/browser/open") | ("POST", "/browser/close") => {
            if let Some(tab) = query_param(query, "tab").filter(|tab| !tab.is_empty()) {
                let mut activity = browser_activity.lock().expect("browser activity lock");
                if path == "/browser/open" {
                    activity.open(tab);
                } else {
                    activity.close(tab, Instant::now());
                }
            }
            text_response(200, "ok")
        }
        ("POST", "/new") => {
            let side = query_param(query, "side")
                .and_then(parse_side)
                .unwrap_or(BLACK);
            let rule = query_param(query, "rule")
                .and_then(|value| value.parse::<RuleSet>().ok())
                .unwrap_or(RuleSet::Freestyle);
            let mode = match query_param(query, "mode") {
                Some("two_player") => GameMode::TwoPlayer,
                _ => GameMode::VsEngine,
            };
            let clock_mode = query_param(query, "clock")
                .and_then(|value| value.parse::<SearchClockMode>().ok())
                .unwrap_or_default();
            {
                let mut game = state.lock().expect("state lock");
                game.new_game_with_clock(side, rule, mode, clock_mode);
            }
            if mode == GameMode::VsEngine && side == WHITE {
                maybe_start_engine(Arc::clone(&state));
            }
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        ("POST", "/play") => {
            let x = query_param(query, "x").and_then(|value| value.parse::<usize>().ok());
            let y = query_param(query, "y").and_then(|value| value.parse::<usize>().ok());
            let should_engine_move = {
                let mut game = state.lock().expect("state lock");
                if let (Some(x), Some(y)) = (x, y) {
                    game.play_human(x, y)
                } else {
                    game.set_localized_error("缺少坐标。", "The move is missing coordinates.");
                    false
                }
            };
            if should_engine_move {
                maybe_start_engine(Arc::clone(&state));
            }
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        ("POST", "/undo") => {
            state.lock().expect("state lock").undo_turn();
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        ("POST", "/difficulty") => {
            let difficulty = query_param(query, "value")
                .and_then(|value| value.parse::<SearchDifficulty>().ok());
            {
                let mut game = state.lock().expect("state lock");
                if let Some(difficulty) = difficulty {
                    game.set_difficulty(difficulty);
                } else {
                    game.set_localized_error(
                        "未知难度，请选择 beginner、normal、senior 或 master。",
                        "Unknown difficulty. Choose beginner, normal, senior or master.",
                    );
                }
            }
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        ("POST", "/time_limit") => {
            let ms = query_param(query, "ms").and_then(|value| value.parse::<u32>().ok());
            {
                let mut game = state.lock().expect("state lock");
                if let Some(ms) = ms {
                    game.set_time_limit_ms(ms);
                } else {
                    game.set_localized_error(
                        "缺少或无效的限时参数。",
                        "The time limit is missing or invalid.",
                    );
                }
            }
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        ("POST", "/threads") => {
            let enabled = match query_param(query, "enabled") {
                Some("1" | "true") => Some(true),
                Some("0" | "false") => Some(false),
                _ => None,
            };
            {
                let mut game = state.lock().expect("state lock");
                if let Some(enabled) = enabled {
                    let target_threads = if enabled { default_multi_threads } else { 1 };
                    game.set_threads(target_threads);
                    if !game.snapshot().engine_thinking {
                        game.set_overlap_vct_alphabeta(target_threads == 1);
                    }
                } else {
                    game.set_localized_error(
                        "无效的线程开关参数。",
                        "Invalid multithreading setting.",
                    );
                }
            }
            json_response(&state.lock().expect("state lock").snapshot_in(language))
        }
        _ => text_response(404, "not found"),
    };
    let _ = stream.write_all(response.as_bytes());
}

fn split_target(target: &str) -> (&str, &str) {
    target
        .split_once('?')
        .map_or((target, ""), |(path, query)| (path, query))
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn parse_side(value: &str) -> Option<Side> {
    match value {
        "black" | "1" => Some(BLACK),
        "white" | "-1" => Some(WHITE),
        _ => None,
    }
}

fn maybe_start_engine(state: Arc<Mutex<GameController>>) {
    let task = { state.lock().expect("state lock").prepare_engine_search() };
    let Some(task) = task else {
        return;
    };
    let generation = task.generation();
    let thread_state = Arc::clone(&state);
    let spawn_result = thread::Builder::new()
        .name("gomoku-gui-search".to_string())
        .spawn(move || {
            let completion = catch_unwind(AssertUnwindSafe(|| task.run())).unwrap_or_else(|_| {
                EngineSearchCompletion::failed_localized(
                    generation,
                    "引擎搜索线程异常退出。",
                    "The engine search thread exited unexpectedly.",
                )
            });
            thread_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .commit_engine_search(completion);
        });
    if spawn_result.is_err() {
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .commit_engine_search(EngineSearchCompletion::failed_localized(
                generation,
                "系统资源不足，无法启动引擎搜索线程。",
                "There are not enough system resources to start the engine search thread.",
            ));
    }
}

fn html_response(body: &str) -> String {
    response(200, "text/html; charset=utf-8", body.to_string())
}

fn json_response<T: Serialize>(value: &T) -> String {
    let body = serde_json::to_string(value).expect("state serializes");
    response(200, "application/json; charset=utf-8", body)
}

fn text_response(status: u16, body: &str) -> String {
    response(status, "text/plain; charset=utf-8", body.to_string())
}

fn response(status: u16, content_type: &str, body: String) -> String {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_activity_exits_only_after_the_last_page_closes() {
        let mut browser = BrowserActivity::default();
        let now = Instant::now();
        let late = now + BROWSER_EXIT_GRACE;
        browser.open("frozen");
        browser.open("visible");
        browser.close("visible", now);
        assert!(!browser.should_exit(late));
        browser.close("frozen", now);
        assert!(!browser.should_exit(now));
        assert!(browser.should_exit(late));
        browser.open("reloaded");
        assert!(!browser.should_exit(late));
    }

    #[test]
    fn browser_url_rewrites_unspecified_bind_addresses() {
        assert_eq!(browser_url("0.0.0.0:18080"), "http://127.0.0.1:18080");
        assert_eq!(browser_url("[::]:18080"), "http://[::1]:18080");
        assert_eq!(browser_url("127.0.0.1:18080"), "http://127.0.0.1:18080");
    }

    #[test]
    fn windows_browser_url_encoding_preserves_unicode_and_rejects_nulls() {
        let url = "http://127.0.0.1:18080/?name=\u{68cb}\u{1f600}&mode=renju";
        let encoded = browser_url_utf16(url).unwrap();
        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&encoded[..encoded.len() - 1]).unwrap(),
            url
        );
        assert_eq!(
            browser_url_utf16("http://127.0.0.1/\0ignored")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn windows_browser_shell_result_uses_documented_success_boundary() {
        for code in [0, 2, 5, 31, 32] {
            let error = shell_execute_result(code).unwrap_err();
            assert!(error.to_string().contains(&format!("code {code}")));
        }
        for code in [33, isize::MAX] {
            assert!(shell_execute_result(code).is_ok());
        }
    }

    #[test]
    fn desktop_gui_contains_difficulty_controls_and_result_dialog() {
        for marker in [
            "difficulty-select",
            "入门 · d2 / w10 · 经典",
            "普通 · d4 / w20 · 经典",
            "高级 · d6 / w30 · 经典 + VCF",
            "大师 · d8 / w40 · 经典 + VCF/VCT",
            "time-limit-select",
            "0.5 秒 (500 ms)",
            "1 秒 (1000 ms)",
            "2.5 秒 (2500 ms)",
            "5 秒 (5000 ms)",
            "selectClock('untimed')",
            "selectClock('timed')",
            "result-backdrop",
            "你赢了",
            "引擎获胜",
            "查看棋盘",
            "再来一局",
            "start-two-player",
            "开始双人对局",
            "selectMode('two_player')",
            "registerBrowserTab",
            "/browser/open",
            "/browser/close",
            "NNUE 局势估计",
            "NNUE (Mix9svq · FastEngine)",
        ] {
            assert!(INDEX_HTML.contains(marker), "missing marker: {marker}");
        }
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>rust_gomoku GUI</title>
  <style>
    :root {
      --ink: #20170e;
      --paper: #f5ead0;
      --wood: #d8a85f;
      --wood2: #f2cc83;
      --accent: #0f6b54;
      --accent2: #7f5a2e;
      --danger: #9c2f24;
      --panel: #fff8e8;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100vh;
      color: var(--ink);
      font-family: "Segoe UI", "Noto Sans SC", "Microsoft YaHei", sans-serif;
      background:
        radial-gradient(circle at 20% 10%, rgba(255,255,255,.55), transparent 28rem),
        linear-gradient(135deg, #efe2c0, #cfa15b 45%, #8e6434);
    }
    main {
      display: grid;
      grid-template-columns: minmax(320px, 660px) minmax(320px, 420px);
      gap: 24px;
      width: min(1120px, calc(100vw - 32px));
      margin: 28px auto;
      align-items: stretch;
    }
    .board-wrap, .panel {
      background: rgba(255, 248, 232, .86);
      border: 1px solid rgba(70, 44, 18, .22);
      box-shadow: 0 24px 70px rgba(40, 26, 9, .28);
      border-radius: 22px;
    }
    .board-wrap { padding: 18px; }
    .panel {
      padding: 24px;
      min-height: calc(min(660px, calc(100vw - 32px)) + 36px);
      display: flex;
      flex-direction: column;
    }
    #board {
      display: block;
      width: 100%;
      aspect-ratio: 1;
      border-radius: 16px;
      background: linear-gradient(135deg, var(--wood2), var(--wood));
      cursor: pointer;
    }
    h1 {
      margin: 0 0 14px;
      font-size: 32px;
      letter-spacing: .01em;
      line-height: 1.1;
    }
    .status {
      min-height: 64px;
      padding: 14px 16px;
      background: #fffdf5;
      border-left: 6px solid var(--accent);
      border-radius: 16px;
      font-size: 18px;
      font-weight: 650;
      line-height: 1.45;
    }
    .status.thinking { border-color: #c0791c; }
    .status.error { border-color: var(--danger); }
    .controls {
      display: grid;
      grid-template-columns: 1fr;
      gap: 14px;
      margin: 20px 0;
    }
    /* The hidden attribute must win over the explicit display on these rows. */
    [hidden] { display: none !important; }
    .new-game {
      display: grid;
      gap: 12px;
    }
    #start-vs-engine {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
    }
    .rule-switch, .mode-switch {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 8px;
    }
    .settings {
      display: grid;
      gap: 12px;
      padding: 14px;
      background: rgba(255, 253, 245, .66);
      border: 1px solid rgba(70, 44, 18, .16);
      border-radius: 12px;
    }
    .setting-row {
      display: grid;
      grid-template-columns: 72px minmax(0, 1fr);
      align-items: center;
      gap: 12px;
    }
    .setting-label {
      color: #6b5235;
      font-size: 14px;
      font-weight: 750;
    }
    .settings button {
      padding: 10px 12px;
      font-size: 15px;
      box-shadow: none;
    }
    .settings button:hover {
      box-shadow: 0 5px 12px rgba(37, 23, 9, .12);
    }
    html[lang="en"] .settings button {
      padding-inline: 8px;
    }
    select {
      width: 100%;
      min-height: 46px;
      padding: 0 40px 0 14px;
      color: #2a1b10;
      font: inherit;
      font-size: 15px;
      font-weight: 700;
      background: #fffdf5;
      border: 1px solid rgba(78, 50, 20, .28);
      border-radius: 8px;
      cursor: pointer;
    }
    select:disabled { opacity: .48; cursor: not-allowed; }
    .actions {
      display: grid;
      grid-template-columns: 1fr;
      gap: 12px;
      padding-top: 2px;
    }
    button {
      border: 0;
      border-radius: 14px;
      padding: 14px 16px;
      background: linear-gradient(180deg, #f6ead0, #dfc18b);
      color: #2a1b10;
      font-size: 17px;
      font-weight: 800;
      letter-spacing: .02em;
      cursor: pointer;
      border: 1px solid rgba(78, 50, 20, .28);
      box-shadow: 0 9px 18px rgba(37, 23, 9, .14);
      transition: transform .12s ease, box-shadow .12s ease, filter .12s ease;
    }
    button:hover {
      transform: translateY(-1px);
      filter: brightness(1.04);
      box-shadow: 0 13px 24px rgba(37, 23, 9, .2);
    }
    button.primary { background: linear-gradient(180deg, #ead2a2, #c99b5a); }
    button.secondary { background: linear-gradient(180deg, #e7ddc8, #c9b188); }
    button.active {
      background: linear-gradient(180deg, #5c8c73, #27664f);
      color: #fff8e8;
      border-color: rgba(15, 76, 57, .58);
    }
    button.warn {
      background: linear-gradient(180deg, #efe0ca, #d4a972);
      color: #562018;
    }
    button:disabled { opacity: .42; cursor: not-allowed; }
    .kv {
      display: grid;
      grid-template-columns: 118px 1fr;
      gap: 10px 12px;
      padding: 16px 0 0;
      border-top: 1px solid rgba(70, 44, 18, .18);
      font-family: "Segoe UI", "Noto Sans SC", "Microsoft YaHei", sans-serif;
      font-size: 16px;
      line-height: 1.35;
    }
    .kv div:nth-child(odd) {
      color: #6b5235;
      font-weight: 750;
    }
    .kv div:nth-child(even) {
      font-size: 16px;
      color: #1f1a14;
      word-break: break-word;
    }
    .position-estimate {
      margin-top: 16px;
      padding: 12px;
      border: 1px solid rgba(70, 44, 18, .16);
      border-radius: 12px;
      background: #fffdf5;
    }
    .position-estimate h2 { margin: 0 0 6px; font-size: 16px; }
    .position-caption { margin: 0; color: #6b5235; font-size: 13px; line-height: 1.4; }
    #position-chart { display: block; width: 100%; height: auto; margin: 6px 0; }
    #position-chart:focus-visible { outline: 2px solid var(--accent); border-radius: 4px; }
    .engine-details { margin-top: 14px; font-size: 14px; }
    .engine-details summary { color: #6b5235; cursor: pointer; font-weight: 700; }
    .engine-details .kv { border-top: 0; padding-top: 10px; gap: 6px 10px; }
    .engine-details .kv, .engine-details .kv div:nth-child(even) { font-size: 14px; }
    .check-toggle {
      display: inline-flex;
      align-items: center;
      gap: 10px;
      font-size: 16px;
      font-weight: 700;
      color: #2b2114;
      cursor: pointer;
      user-select: none;
    }
    .check-toggle input[type="checkbox"] {
      width: 20px;
      height: 20px;
      accent-color: #22543d;
      cursor: pointer;
    }
    .check-toggle input[type="checkbox"]:disabled {
      cursor: not-allowed;
      opacity: .55;
    }
    .hint {
      color: #6b5235;
      font-size: 14px;
      line-height: 1.45;
      margin-top: -4px;
    }
    .error-text { color: var(--danger); margin-top: 10px; font-weight: 800; font-size: 16px; }
    .pending-notice {
      padding: 10px 12px;
      border-radius: 10px;
      background: #fff4df;
      border: 1px solid #d9822b;
      color: #8a4300;
      font-size: 14px;
      font-weight: 700;
      line-height: 1.45;
    }
    .pending-notice[hidden] { display: none; }
    .result-backdrop {
      position: fixed;
      inset: 0;
      z-index: 20;
      display: none;
      place-items: center;
      padding: 24px;
      background: rgba(24, 16, 8, .55);
    }
    .result-backdrop.open { display: grid; }
    .result-dialog {
      width: min(420px, 100%);
      padding: 30px;
      text-align: center;
      background: #fffdf5;
      border: 1px solid rgba(70, 44, 18, .22);
      border-radius: 12px;
      box-shadow: 0 24px 70px rgba(40, 26, 9, .32);
    }
    .result-dialog h2 { margin: 0; font-size: 30px; }
    .result-dialog p { margin: 10px 0 24px; color: #6b5235; font-size: 17px; }
    .result-actions {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
    }
    @media (max-width: 860px) {
      main { grid-template-columns: 1fr; margin-top: 14px; align-items: start; }
      .panel { min-height: auto; }
      h1 { font-size: 25px; }
    }
  </style>
</head>
<body>
  <main>
    <section class="board-wrap">
      <canvas id="board" width="720" height="720" aria-label="gomoku board"></canvas>
    </section>
    <aside class="panel">
      <h1>rust_gomoku</h1>
      <div id="status" class="status" data-en="Loading…">加载中...</div>
      <div id="error" class="error-text"></div>
      <div class="controls">
        <div class="settings">
          <div class="setting-row">
            <span class="setting-label" data-en="Play">对战</span>
            <div class="mode-switch">
              <button id="mode-engine" class="secondary" onclick="selectMode('vs_engine')" data-en="Vs Engine">人机</button>
              <button id="mode-two" class="secondary" onclick="selectMode('two_player')" data-en="Two Players">双人</button>
            </div>
          </div>
          <div class="setting-row">
            <span class="setting-label" data-en="Rules">规则</span>
            <div class="rule-switch">
              <button id="rule-freestyle" class="secondary" onclick="selectRule('freestyle')" data-en="Freestyle">无禁手</button>
              <button id="rule-renju" class="secondary" onclick="selectRule('renju')" data-en="Renju">有禁手</button>
            </div>
          </div>
          <div class="setting-row" id="row-clock-mode">
            <span class="setting-label" data-en="Mode">模式</span>
            <div class="rule-switch">
              <button id="clock-untimed" class="secondary" onclick="selectClock('untimed')" data-en="Casual">休闲</button>
              <button id="clock-timed" class="secondary" onclick="selectClock('timed')" data-en="Pro · NNUE">专业 · NNUE</button>
            </div>
          </div>
          <label class="setting-row" id="row-difficulty" for="difficulty-select" hidden>
            <span class="setting-label" data-en="Casual difficulty">休闲难度</span>
            <select id="difficulty-select" onchange="setDifficulty(this.value)">
              <option value="beginner" data-en="Beginner · d2 / w10 · Classical">入门 · d2 / w10 · 经典</option>
              <option value="normal" selected data-en="Normal · d4 / w20 · Classical">普通 · d4 / w20 · 经典</option>
              <option value="senior" data-en="Advanced · d6 / w30 · Classical + VCF">高级 · d6 / w30 · 经典 + VCF</option>
              <option value="master" data-en="Master · d8 / w40 · Classical + VCF/VCT">大师 · d8 / w40 · 经典 + VCF/VCT</option>
            </select>
          </label>
          <label class="setting-row" id="row-time-limit" for="time-limit-select">
            <span class="setting-label" data-en="Pro time limit">专业限时</span>
            <select id="time-limit-select" onchange="setTimeLimit(this.value)">
              <option value="500" data-en="0.5 s (500 ms)">0.5 秒 (500 ms)</option>
              <option value="1000" selected data-en="1 s (1000 ms)">1 秒 (1000 ms)</option>
              <option value="2500" data-en="2.5 s (2500 ms)">2.5 秒 (2500 ms)</option>
              <option value="5000" data-en="5 s (5000 ms)">5 秒 (5000 ms)</option>
            </select>
          </label>
          <label class="setting-row" id="row-multithread" for="multithread-toggle">
            <span class="setting-label" data-en="Threads">多线程</span>
            <span class="check-toggle">
              <input id="multithread-toggle" type="checkbox" checked onchange="setMultithread(this.checked)">
              <span id="multithread-label" data-en="On">开启</span>
            </span>
          </label>
          <div class="setting-row">
            <span class="setting-label" data-en="Move sounds">落子音效</span>
            <button id="sound-toggle" class="secondary" onclick="toggleSound()" data-en="On">开</button>
          </div>
          <label class="setting-row" for="ui-language">
            <span class="setting-label" data-en="Language">语言</span>
            <select id="ui-language">
              <option value="system" data-en="System default">跟随系统</option>
              <option value="zh-CN" lang="zh-CN">简体中文</option>
              <option value="en" lang="en">English</option>
            </select>
          </label>
        </div>
        <div id="pending-notice" class="pending-notice" hidden></div>
        <div class="new-game">
          <div id="start-vs-engine">
            <button class="primary" onclick="newGame('black')" data-en="Play Black">执黑</button>
            <button class="primary" onclick="newGame('white')" data-en="Play White">执白</button>
          </div>
          <button id="start-two-player" class="primary" onclick="startTwoPlayer()" hidden data-en="Start two-player game">开始双人对局</button>
        </div>
        <div class="actions">
          <button id="undo-btn" class="warn" onclick="undo()" data-en="Undo">悔棋</button>
        </div>
        <div class="hint" data-en="U Undo · R Restart">U 悔棋 · R 重新开局</div>
      </div>
      <div class="kv" id="info"></div>
      <section class="position-estimate" id="position-estimate" aria-labelledby="position-title" hidden>
        <h2 id="position-title" data-en="NNUE position estimate">NNUE 局势估计</h2>
        <p class="position-caption" id="position-current"></p>
        <canvas id="position-chart" width="640" height="180" tabindex="0" hidden
          aria-label="黑方预计得分曲线，使用左右方向键查看历史估计" data-aria-en="Estimated score for Black. Use the arrow keys to browse moves."
          aria-describedby="position-point"></canvas>
        <p class="position-caption" id="position-point"></p>
      </section>
      <details class="engine-details" id="engine-details" hidden>
        <summary data-en="Engine details">引擎详情</summary>
        <div class="kv" id="search-info"></div>
      </details>
    </aside>
  </main>
  <div id="result-backdrop" class="result-backdrop" aria-hidden="true">
    <section class="result-dialog" role="dialog" aria-modal="true" aria-labelledby="result-title">
      <h2 id="result-title" data-en="Game over">对局结束</h2>
      <p id="result-message"></p>
      <div class="result-actions">
        <button class="secondary" onclick="closeResult()" data-en="View board">查看棋盘</button>
        <button class="primary" onclick="restartFromResult()" data-en="Play again">再来一局</button>
      </div>
    </section>
  </div>
  <script src="/ui_language.js"></script>
  <script>
    const { tag: uiTag, text: tr } = localizeUi(document.documentElement.lang);
    const canvas = document.getElementById('board');
    const ctx = canvas.getContext('2d');
    const positionChart = document.getElementById('position-chart');
    const positionCtx = positionChart.getContext('2d');
    const chartBounds = { left: 58, right: 622, top: 14, bottom: 146 };
    let selectedEstimatePly = null;
    let state = null;
    let selectedRule = 'freestyle';
    let selectedMode = 'vs_engine';
    let selectedClock = 'timed';
    let syncedInitialSelection = false;
    let announcedResult = null;
    let soundOn = true;
    let prevMoveCount = null;
    let suppressNextSound = false; // human move already played optimistically on click

    // Stone-placement sound, synthesized via Web Audio (no asset file).
    let audioCtx = null;
    function unlockAudio() {
      try {
        if (!audioCtx) {
          const AC = window.AudioContext || window.webkitAudioContext;
          if (!AC) return;
          audioCtx = new AC();
        }
        if (audioCtx.state === 'suspended') audioCtx.resume();
      } catch (e) { /* audio unsupported */ }
    }
    function playStoneSound() {
      if (!soundOn) return;
      unlockAudio();
      if (!audioCtx || audioCtx.state !== 'running') return;
      const c = audioCtx;
      const now = c.currentTime;
      const dur = 0.05;
      const frames = Math.max(1, Math.floor(c.sampleRate * dur));
      const buffer = c.createBuffer(1, frames, c.sampleRate);
      const data = buffer.getChannelData(0);
      for (let i = 0; i < frames; i++) {
        const decay = 1 - i / frames;
        data[i] = (Math.random() * 2 - 1) * decay * decay;
      }
      const noise = c.createBufferSource();
      noise.buffer = buffer;
      const bandpass = c.createBiquadFilter();
      bandpass.type = 'bandpass';
      bandpass.frequency.value = 1900;
      bandpass.Q.value = 0.9;
      const noiseGain = c.createGain();
      noiseGain.gain.value = 0.5;
      noise.connect(bandpass); bandpass.connect(noiseGain); noiseGain.connect(c.destination);
      noise.start(now); noise.stop(now + dur);
      const osc = c.createOscillator();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(230, now);
      osc.frequency.exponentialRampToValueAtTime(120, now + 0.045);
      const oscGain = c.createGain();
      oscGain.gain.setValueAtTime(0.45, now);
      oscGain.gain.exponentialRampToValueAtTime(0.0001, now + 0.05);
      osc.connect(oscGain); oscGain.connect(c.destination);
      osc.start(now); osc.stop(now + 0.05);
    }
    function maybePlayStoneSound() {
      if (!state) return;
      if (prevMoveCount !== null && state.move_count === prevMoveCount + 1) {
        // engine replies play here; the human's own move was already played on
        // click (suppressNextSound) to avoid the HTTP round-trip latency
        if (!suppressNextSound) playStoneSound();
        suppressNextSound = false;
      }
      prevMoveCount = state.move_count;
    }
    function toggleSound() {
      soundOn = !soundOn;
      const btn = document.getElementById('sound-toggle');
      btn.textContent = soundOn ? tr('开', 'On') : tr('关', 'Off');
      btn.classList.toggle('primary', soundOn);
      btn.classList.toggle('secondary', !soundOn);
      if (soundOn) unlockAudio();
    }
    window.addEventListener('pointerdown', unlockAudio);

    async function api(path) {
      const url = new URL(path, location.origin);
      url.searchParams.set('lang', uiTag);
      const res = await fetch(url, { method: path === '/state' ? 'GET' : 'POST' });
      if (!res.ok) throw new Error(await res.text());
      state = await res.json();
      maybePlayStoneSound();
      draw();
      renderInfo();
    }
    const browserTab = Date.now().toString(36) + '-' + Math.random().toString(36).slice(2);
    function registerBrowserTab() {
      fetch('/browser/open?tab=' + browserTab, { method: 'POST', keepalive: true }).catch(() => {});
    }
    window.addEventListener('pagehide', event => {
      if (!event.persisted) navigator.sendBeacon('/browser/close?tab=' + browserTab);
    });
    window.addEventListener('pageshow', event => {
      if (event.persisted) registerBrowserTab();
    });
    function refresh() { api('/state').catch(showError); }
    function newGame(side) { api('/new?side=' + side + '&rule=' + selectedRule + '&mode=' + selectedMode + '&clock=' + selectedClock).catch(showError); }
    function startTwoPlayer() { newGame('black'); }
    function setDifficulty(difficulty) { api('/difficulty?value=' + difficulty).catch(showError); }
    function setTimeLimit(ms) { api('/time_limit?ms=' + ms).catch(showError); }
    function setMultithread(enabled) { api('/threads?enabled=' + (enabled ? '1' : '0')).catch(showError); }
    function syncEmptyBoardSettings() {
      if (state && state.move_count === 0 && !state.engine_thinking) {
        api('/new?side=black&rule=' + selectedRule + '&mode=' + selectedMode + '&clock=' + selectedClock).catch(showError);
      }
    }
    function selectRule(rule) {
      selectedRule = rule;
      renderRuleButtons();
      renderPendingNotice();
      syncEmptyBoardSettings();
    }
    function selectClock(clock) {
      selectedClock = clock;
      renderClockButtons();
      renderModeButtons();
      renderPendingNotice();
      syncEmptyBoardSettings();
    }
    function selectMode(mode) {
      selectedMode = mode;
      renderModeButtons();
      renderPendingNotice();
      syncEmptyBoardSettings();
    }
    function renderPendingNotice() {
      const notice = document.getElementById('pending-notice');
      if (!notice || !state || !state.params || state.move_count === 0) {
        if (notice) notice.hidden = true;
        return;
      }
      const p = state.params;
      const modeDiff = selectedMode !== p.mode;
      const ruleDiff = selectedRule !== p.rule;
      const clockDiff = selectedMode !== 'two_player' && p.mode !== 'two_player' && selectedClock !== p.clock_mode;
      if (!modeDiff && !ruleDiff && !clockDiff) {
        notice.hidden = true;
        return;
      }
      const curParts = [
        p.mode === 'two_player' ? tr('双人', 'Two Players') : tr('人机', 'Vs Engine'),
        p.rule === 'renju' ? tr('有禁手', 'Renju') : tr('无禁手', 'Freestyle'),
      ];
      if (p.mode !== 'two_player') {
        curParts.push(p.clock_mode === 'timed' ? '专业' : tr('休闲', 'Casual'));
      }
      notice.textContent = tr(`当前对局仍为「${curParts.join(' / ')}」，点击下方开局按钮后才会切换为新设置。`, `Current game: ${curParts.join(' / ')}. Start a new game to apply the changes.`);
      notice.hidden = false;
    }
    function renderModeButtons() {
      const twoPlayer = selectedMode === 'two_player';
      const engineButton = document.getElementById('mode-engine');
      const twoButton = document.getElementById('mode-two');
      if (engineButton && twoButton) {
        engineButton.className = 'secondary' + (twoPlayer ? '' : ' active');
        twoButton.className = 'secondary' + (twoPlayer ? ' active' : '');
      }
      document.getElementById('start-vs-engine').hidden = twoPlayer;
      document.getElementById('start-two-player').hidden = !twoPlayer;
      // Engine-only settings are meaningless in two-player mode.
      document.getElementById('row-clock-mode').hidden = twoPlayer;
      document.getElementById('row-difficulty').hidden = twoPlayer || selectedClock === 'timed';
      document.getElementById('row-time-limit').hidden = twoPlayer || selectedClock !== 'timed';
      document.getElementById('row-multithread').hidden = twoPlayer;
    }
    function restartSameSide() {
      if (state && state.params) {
        selectedMode = state.params.mode || selectedMode;
        selectedRule = state.params.rule || selectedRule;
        selectedClock = state.params.clock_mode || selectedClock;
        renderRuleButtons();
        renderClockButtons();
        renderModeButtons();
        if (state.params.mode === 'two_player') {
          startTwoPlayer();
          return;
        }
      }
      const side = state && state.human_side === -1 ? 'white' : 'black';
      newGame(side);
    }
    function undo() { api('/undo').catch(showError); }
    function closeResult() {
      const backdrop = document.getElementById('result-backdrop');
      backdrop.classList.remove('open');
      backdrop.setAttribute('aria-hidden', 'true');
    }
    function restartFromResult() {
      closeResult();
      restartSameSide();
    }
    function renderResult() {
      if (!state || (state.winner === 0 && !state.draw)) {
        announcedResult = null;
        closeResult();
        return;
      }
      const key = state.draw ? `draw:${state.move_count}` : `${state.winner}:${state.move_count}`;
      if (announcedResult === key) return;
      announcedResult = key;
      const twoPlayer = state.params && state.params.mode === 'two_player';
      if (state.draw) {
        document.getElementById('result-title').textContent = tr('和棋', 'Draw');
        document.getElementById('result-message').textContent = tr(`棋盘在第 ${state.move_count} 手填满，双方均未连成五子。`, `The board filled on move ${state.move_count} without a winner.`);
      } else if (twoPlayer) {
        const winnerName = sideName(state.winner);
        document.getElementById('result-title').textContent = tr(`${winnerName}胜`, `${winnerName} wins`);
        document.getElementById('result-message').textContent = tr(`${winnerName}在第 ${state.move_count} 手取胜。`, `${winnerName} won on move ${state.move_count}.`);
      } else {
        const humanWon = state.winner === state.human_side;
        document.getElementById('result-title').textContent = humanWon ? tr('你赢了', 'You win') : tr('引擎获胜', 'Engine wins');
        document.getElementById('result-message').textContent = humanWon
          ? tr(`第 ${state.move_count} 手取胜。`, `You won on move ${state.move_count}.`)
          : tr(`对局在第 ${state.move_count} 手结束。`, `The game ended on move ${state.move_count}.`);
      }
      const backdrop = document.getElementById('result-backdrop');
      backdrop.classList.add('open');
      backdrop.setAttribute('aria-hidden', 'false');
    }
    function showError(err) {
      document.getElementById('error').textContent = String(err);
    }
    canvas.addEventListener('click', (event) => {
      if (!state || !state.can_play) return;
      const rect = canvas.getBoundingClientRect();
      const p = boardMetrics();
      const x = Math.round(((event.clientX - rect.left) / rect.width * canvas.width - p.margin) / p.cell);
      const y = Math.round(((event.clientY - rect.top) / rect.height * canvas.height - p.margin) / p.cell);
      if (x >= 0 && y >= 0 && x < state.board_size && y < state.board_size) {
        // play immediately on click for a legal-looking point so the click feels
        // instant; maybePlayStoneSound() then skips this move's server echo
        const idx = y * state.board_size + x;
        const empty = !state.cells || state.cells[idx] === 0;
        const forbidden = (state.forbidden_points || []).some((pt) => pt.x === x && pt.y === y);
        if (empty && !forbidden) {
          playStoneSound();
          suppressNextSound = true;
        }
        api(`/play?x=${x}&y=${y}`).catch(showError);
      }
    });
    window.addEventListener('keydown', (event) => {
      if (event.target && ['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName)) return;
      const key = event.key.toLowerCase();
      if (key === 'u') {
        event.preventDefault();
        undo();
      } else if (key === 'r') {
        event.preventDefault();
        restartSameSide();
      }
    });
    function boardMetrics() {
      const margin = 42;
      return { margin, cell: (canvas.width - margin * 2) / 14 };
    }
    function draw() {
      if (!state) return;
      const p = boardMetrics();
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      const grad = ctx.createLinearGradient(0, 0, canvas.width, canvas.height);
      grad.addColorStop(0, '#f5d38e');
      grad.addColorStop(1, '#c89145');
      ctx.fillStyle = grad;
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.strokeStyle = 'rgba(45, 28, 10, .82)';
      ctx.lineWidth = 2;
      for (let i = 0; i < state.board_size; i++) {
        const a = p.margin + i * p.cell;
        ctx.beginPath(); ctx.moveTo(p.margin, a); ctx.lineTo(canvas.width - p.margin, a); ctx.stroke();
        ctx.beginPath(); ctx.moveTo(a, p.margin); ctx.lineTo(a, canvas.height - p.margin); ctx.stroke();
      }
      [[3,3],[11,3],[7,7],[3,11],[11,11]].forEach(([x,y]) => {
        ctx.beginPath();
        ctx.arc(p.margin + x*p.cell, p.margin + y*p.cell, 5, 0, Math.PI*2);
        ctx.fillStyle = 'rgba(45, 28, 10, .75)';
        ctx.fill();
      });
      for (const point of state.forbidden_points) {
        forbiddenMark(point.x, point.y);
      }
      const lastNumber = state.moves.length;
      for (const move of state.moves) {
        stone(move.x, move.y, move.side, move.number, move.number === lastNumber);
      }
    }
    function forbiddenMark(x, y) {
      const p = boardMetrics();
      const cx = p.margin + x*p.cell, cy = p.margin + y*p.cell;
      const r = p.cell * .24;
      ctx.save();
      ctx.strokeStyle = 'rgba(170, 25, 20, .9)';
      ctx.fillStyle = 'rgba(255, 242, 218, .72)';
      ctx.lineWidth = Math.max(2.5, p.cell * .055);
      ctx.beginPath();
      ctx.arc(cx, cy, r * 1.35, 0, Math.PI*2);
      ctx.fill();
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(cx-r, cy-r);
      ctx.lineTo(cx+r, cy+r);
      ctx.moveTo(cx+r, cy-r);
      ctx.lineTo(cx-r, cy+r);
      ctx.stroke();
      ctx.restore();
    }
    function stone(x, y, side, number, isLast) {
      const p = boardMetrics();
      const cx = p.margin + x*p.cell, cy = p.margin + y*p.cell;
      const r = p.cell * .45;
      const grad = ctx.createRadialGradient(cx-r/3, cy-r/3, 2, cx, cy, r);
      if (side === 1) {
        grad.addColorStop(0, '#555');
        grad.addColorStop(1, '#050505');
      } else {
        grad.addColorStop(0, '#fff');
        grad.addColorStop(1, '#d7d1c2');
      }
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI*2);
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.strokeStyle = 'rgba(0,0,0,.35)';
      ctx.stroke();
      ctx.fillStyle = isLast ? '#d82418' : (side === 1 ? '#fff7e8' : '#111');
      ctx.font = `500 ${Math.max(18, Math.floor(p.cell * 0.50))}px "Segoe UI", "Noto Sans SC", "Microsoft YaHei", sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(number), cx, cy + 0.5);
    }
    function sideName(v) {
      return v === 1 ? tr('黑', 'Black') : (v === -1 ? tr('白', 'White') : tr('无', 'None'));
    }
    function formatMs(ms) {
      return `${ms.toFixed(1)} ms`;
    }
    function renderRows(id, rows) {
      document.getElementById(id).replaceChildren(...rows.flatMap(row => row.map(value => {
        const cell = document.createElement('div');
        cell.textContent = value;
        return cell;
      })));
    }
    function blackExpectedScore(point) { return point.black_win + point.draw * 0.5; }
    function percent(value) { return `${(value * 100).toFixed(1)}%`; }
    function chartMaxPly(points) { return Math.max(4, points.at(-1).ply); }
    function renderNnueEstimate() {
      const panel = document.getElementById('position-estimate');
      const current = document.getElementById('position-current');
      const caption = document.getElementById('position-point');
      const points = state.nnue_history || [];
      const p = state.params;
      panel.hidden = p.mode !== 'vs_engine' || p.clock_mode !== 'timed' ||
        !p.nnue_available || state.last_result?.used_nnue === false;
      positionChart.hidden = points.length === 0;
      if (panel.hidden || points.length === 0) {
        selectedEstimatePly = null;
        current.textContent = tr('引擎落子后更新', 'Updates after an engine move');
        caption.textContent = '';
        return;
      }

      const latest = points.at(-1);
      const selected = points.find(point => point.ply === selectedEstimatePly) || latest;
      const latestScore = blackExpectedScore(latest);
      const scorePerMille = Math.round(latestScore * 1000);
      const advantage = scorePerMille > 500 ? tr('黑方占优', 'Black ahead') : scorePerMille < 500 ? tr('白方占优', 'White ahead') : tr('均势', 'Even');
      current.textContent = tr(`第 ${latest.ply} 手后 · ${advantage} · 黑方预计得分 ${percent(latestScore)}`, `Move ${latest.ply} · ${advantage} · Black estimated score ${percent(latestScore)}`);
      caption.textContent = tr(`第 ${selected.ply} 手：黑胜 ${percent(selected.black_win)} · 和棋 ${percent(selected.draw)} · 白胜 ${percent(selected.white_win)}`, `Move ${selected.ply} · Black ${percent(selected.black_win)} · Draw ${percent(selected.draw)} · White ${percent(selected.white_win)}`);

      const { left, right, top, bottom } = chartBounds;
      const maxPly = chartMaxPly(points);
      const x = ply => left + (right - left) * ply / maxPly;
      const y = score => bottom - (bottom - top) * score;
      const c = positionCtx;
      c.clearRect(0, 0, positionChart.width, positionChart.height);
      c.font = '19px "Segoe UI", "Microsoft YaHei", sans-serif';
      c.fillStyle = '#6b5235';
      c.textAlign = 'right';
      c.textBaseline = 'middle';
      for (const score of [1, 0.5, 0]) {
        c.beginPath();
        c.strokeStyle = score === 0.5 ? '#a68a60' : '#e5d8bd';
        c.lineWidth = 1.5;
        c.setLineDash(score === 0.5 ? [6, 5] : []);
        c.moveTo(left, y(score));
        c.lineTo(right, y(score));
        c.stroke();
        c.fillText(`${score * 100}%`, left - 7, y(score));
      }
      c.setLineDash([]);
      c.fillText(tr(`${maxPly} 手`, `${maxPly} moves`), right, bottom + 21);
      c.textAlign = 'left';
      c.fillText('0', left, bottom + 21);
      c.beginPath();
      c.strokeStyle = '#0f6b54';
      c.lineWidth = 3;
      points.forEach((point, index) => {
        const px = x(point.ply);
        const py = y(blackExpectedScore(point));
        if (index === 0) c.moveTo(px, py);
        else c.lineTo(px, py);
      });
      c.stroke();
      c.beginPath();
      c.fillStyle = '#0f6b54';
      c.arc(x(selected.ply), y(blackExpectedScore(selected)), 5, 0, Math.PI * 2);
      c.fill();
    }
    positionChart.addEventListener('pointermove', event => {
      const points = state?.nnue_history;
      if (!points?.length) return;
      const rect = positionChart.getBoundingClientRect();
      const x = (event.clientX - rect.left) * positionChart.width / rect.width;
      const ply = (x - chartBounds.left) * chartMaxPly(points) / (chartBounds.right - chartBounds.left);
      selectedEstimatePly = points.reduce((best, point) =>
        Math.abs(point.ply - ply) < Math.abs(best.ply - ply) ? point : best).ply;
      renderNnueEstimate();
    });
    positionChart.addEventListener('pointerleave', () => {
      selectedEstimatePly = null;
      renderNnueEstimate();
    });
    positionChart.addEventListener('keydown', event => {
      const offset = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0;
      const points = state?.nnue_history;
      if (!offset || !points?.length) return;
      event.preventDefault();
      const selected = points.findIndex(point => point.ply === selectedEstimatePly);
      const next = (selected < 0 ? points.length - 1 : selected) + offset;
      selectedEstimatePly = points[Math.max(0, Math.min(next, points.length - 1))].ply;
      renderNnueEstimate();
    });
    function renderSearchInfo(r, t, p) {
      const details = document.getElementById('engine-details');
      const twoPlayer = p.mode === 'two_player';
      details.hidden = twoPlayer;
      const rows = [[tr('手数', 'Moves'), state.move_count]];
      if (!twoPlayer) {
        rows.unshift([tr('你执', 'You play'), sideName(state.human_side)]);
        if (r?.ms != null) rows.push([tr('上次搜索', 'Last search'), formatMs(r.ms)]);
        if (t?.vcf_found || t?.vct_accepted) rows.push([tr('战术', 'Tactics'), tr('找到强制胜着', 'Forced win found')]);
        if (p.clock_mode === 'timed' && r?.used_nnue === false) {
          rows.push([tr('评估器', 'Evaluator'), tr('已回落到经典评估', 'Using classical evaluation')]);
        }
      }
      renderRows('info', rows);
      renderNnueEstimate();
      if (twoPlayer) return;

      const usingFastNnue = r ? !!r.used_nnue : (p.clock_mode === 'timed' && p.use_nnue && p.nnue_available);
      let evaluator = tr('经典评估', 'Classical evaluation');
      if (r?.used_nnue) evaluator = 'NNUE (Mix9svq · FastEngine)';
      else if (p.clock_mode === 'timed') {
        if (!p.nnue_available) evaluator += tr('（未找到 NNUE 权重）', ' (NNUE weights missing)');
        else evaluator = r ? tr('经典评估（NNUE 加载失败）', 'Classical evaluation (NNUE load failed)') : 'NNUE (Mix9svq · FastEngine)';
      }
      const config = p.clock_mode === 'timed' ? `${p.time_limit_ms} ms` : tr(`深度 ${p.depth}`, `Depth ${p.depth}`);
      const searchRows = [
        [tr('评估器', 'Evaluator'), evaluator],
        [tr('搜索配置', 'Search settings'), usingFastNnue
          ? tr(`${config} · ${p.threads} 线程 · 32 MiB TT`, `${config} · ${p.threads} threads · 32 MiB TT`)
          : tr(`${config} · 宽度 ${p.width} · ${p.threads} 线程`, `${config} · Width ${p.width} · ${p.threads} threads`)],
      ];
      if (!usingFastNnue) {
        searchRows.push([tr('战术配置', 'Tactical settings'), [p.compute_vcf ? `VCF ${p.root_vcf_depth}` : tr('VCF 关闭', 'VCF off'),
          p.compute_vct ? `VCT ${p.root_vct_depth}` : tr('VCT 关闭', 'VCT off')].join(' · ')]);
      }
      if (r) {
        searchRows.push([tr('搜索结果', 'Search result'), tr(`深度 ${r.depth} · ${r.nodes.toLocaleString(uiTag)} 节点 · 引擎侧分数 ${r.score}`, `Depth ${r.depth} · ${r.nodes.toLocaleString(uiTag)} nodes · Engine score ${r.score}`)]);
      }
      if (t?.used_vcf) searchRows.push(['VCF', t.vcf_found ? tr('找到胜着', 'Win found') : tr('未找到胜着', 'No win found')]);
      if (t?.vct_triggered) {
        const outcome = t.vct_accepted ? tr('通过验证', 'Verified') : t.vct_found ? tr('未通过验证', 'Verification failed') : tr('未找到胜着', 'No win found');
        const vctDetails = [outcome];
        if (t.vct_ms != null) vctDetails.push(formatMs(t.vct_ms));
        if (t.vct_reject_reason) vctDetails.push(t.vct_reject_reason);
        searchRows.push(['VCT', vctDetails.join(' · ')]);
      }
      if (t?.alphabeta_ms != null) searchRows.push([tr('主搜索', 'Main search'), formatMs(t.alphabeta_ms)]);
      if (t?.overlap_used) {
        const times = [
          [tr('主搜索', 'Main search'), t.overlap_ab_ms], [tr('等待', 'Wait'), t.overlap_wait_ms], [tr('TT 快照', 'TT snapshot'), t.tt_snapshot_ms],
        ].filter(([, ms]) => ms != null).map(([label, ms]) => `${label} ${formatMs(ms)}`);
        searchRows.push([tr('战术并行', 'Tactical overlap'), times.join(' · ')]);
      }
      renderRows('search-info', searchRows);
    }
    function renderRuleButtons() {
      const freestyleButton = document.getElementById('rule-freestyle');
      const renjuButton = document.getElementById('rule-renju');
      if (!freestyleButton || !renjuButton) return;
      freestyleButton.className = 'secondary' + (selectedRule === 'freestyle' ? ' active' : '');
      renjuButton.className = 'secondary' + (selectedRule === 'renju' ? ' active' : '');
      const disabled = !!(state && state.engine_thinking);
      freestyleButton.disabled = disabled;
      renjuButton.disabled = disabled;
    }
    function renderClockButtons() {
      const untimedButton = document.getElementById('clock-untimed');
      const timedButton = document.getElementById('clock-timed');
      if (!untimedButton || !timedButton) return;
      untimedButton.className = 'secondary' + (selectedClock === 'untimed' ? ' active' : '');
      timedButton.className = 'secondary' + (selectedClock === 'timed' ? ' active' : '');
      const disabled = !!(state && state.engine_thinking);
      untimedButton.disabled = disabled;
      timedButton.disabled = disabled;
    }
    function renderInfo() {
      const status = document.getElementById('status');
      status.textContent = state.status;
      status.className = 'status' + (state.engine_thinking ? ' thinking' : '') + (state.error ? ' error' : '');
      document.getElementById('error').textContent = state.error || '';
      const r = state.last_result;
      const t = state.last_trace;
      const p = state.params;
      if (!syncedInitialSelection) {
        selectedRule = p.rule || selectedRule;
        selectedMode = p.mode || selectedMode;
        selectedClock = p.clock_mode || selectedClock;
        syncedInitialSelection = true;
      }
      const difficultySelect = document.getElementById('difficulty-select');
      if (p.difficulty !== 'custom') difficultySelect.value = p.difficulty;
      difficultySelect.disabled = !!state.engine_thinking;
      const timeLimitSelect = document.getElementById('time-limit-select');
      if (timeLimitSelect && p.time_limit_ms) {
        timeLimitSelect.value = String(p.time_limit_ms);
        timeLimitSelect.disabled = !!state.engine_thinking;
      }
      const mtToggle = document.getElementById('multithread-toggle');
      const mtLabel = document.getElementById('multithread-label');
      mtToggle.checked = p.threads > 1;
      mtToggle.disabled = !!state.engine_thinking;
      mtLabel.textContent = p.threads > 1 ? tr(`开启（${p.threads} 线程）`, `On (${p.threads} threads)`) : tr('关闭（单线程）', 'Off (1 thread)');
      // Undo is rejected server-side while the engine thinks; disable it to match.
      document.getElementById('undo-btn').disabled = !!state.engine_thinking;
      renderRuleButtons();
      renderClockButtons();
      renderModeButtons();
      renderPendingNotice();
      renderSearchInfo(r, t, p);
      renderResult();
    }
    renderModeButtons();
    registerBrowserTab();
    setInterval(refresh, 500);
    refresh();
  </script>
</body>
</html>
"#;
