//! Local browser GUI for playing against the engine.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;

use rust_gomoku::{load_default_config, GameController, RuleSet, Side, BLACK, WHITE};

const DEFAULT_ADDR: &str = "127.0.0.1:18080";

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("warning: debug build is much slower; use `cargo run --release --bin gomoku_gui` for real play.");
    }
    let args = parse_args();
    let mut config = load_default_config();
    config.runtime.overlap_vct_alphabeta = true;
    if let Some(depth) = args.depth {
        config.root_search.depth = depth;
    }
    if let Some(width) = args.width {
        config.root_search.wide = width;
        config.root_search.timed_max_wide = width;
    }
    let state = Arc::new(Mutex::new(GameController::new(config)));
    let addr = args.addr;
    let listener = TcpListener::bind(&addr).expect("GUI server binds to local address");
    let url = browser_url(&addr);
    eprintln!("gomoku gui listening on {url}");
    if args.open_browser {
        if let Err(err) = open_browser(&url) {
            eprintln!("warning: failed to open the browser automatically: {err}");
        }
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let state = Arc::clone(&state);
        thread::spawn(move || handle_client(stream, state));
    }
}

struct GuiArgs {
    addr: String,
    depth: Option<i32>,
    width: Option<i32>,
    open_browser: bool,
}

fn parse_args() -> GuiArgs {
    let mut result = GuiArgs {
        addr: DEFAULT_ADDR.to_string(),
        depth: None,
        width: None,
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
    spawn_browser("cmd", &["/C", "start", "", url])
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

fn handle_client(mut stream: TcpStream, state: Arc<Mutex<GameController>>) {
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
    let response = match (method, path) {
        ("GET", "/") => html_response(INDEX_HTML),
        ("GET", "/state") => json_response(&state.lock().expect("state lock").snapshot()),
        ("POST", "/new") => {
            let side = query_param(query, "side")
                .and_then(parse_side)
                .unwrap_or(BLACK);
            let rule = query_param(query, "rule")
                .and_then(|value| value.parse::<RuleSet>().ok())
                .unwrap_or(RuleSet::Freestyle);
            {
                let mut game = state.lock().expect("state lock");
                game.new_game(side, rule);
            }
            if side == WHITE {
                maybe_start_engine(Arc::clone(&state));
            }
            json_response(&state.lock().expect("state lock").snapshot())
        }
        ("POST", "/play") => {
            let x = query_param(query, "x").and_then(|value| value.parse::<usize>().ok());
            let y = query_param(query, "y").and_then(|value| value.parse::<usize>().ok());
            let should_engine_move = {
                let mut game = state.lock().expect("state lock");
                if let (Some(x), Some(y)) = (x, y) {
                    game.play_human(x, y)
                } else {
                    game.set_error("缺少坐标。");
                    false
                }
            };
            if should_engine_move {
                maybe_start_engine(Arc::clone(&state));
            }
            json_response(&state.lock().expect("state lock").snapshot())
        }
        ("POST", "/undo") => {
            state.lock().expect("state lock").undo_turn();
            json_response(&state.lock().expect("state lock").snapshot())
        }
        ("POST", "/profile") => {
            let profile = query_param(query, "value").and_then(|value| value.parse().ok());
            {
                let mut game = state.lock().expect("state lock");
                if let Some(profile) = profile {
                    game.set_profile(profile);
                } else {
                    game.set_error("未知模式，请选择 base 或 fast。");
                }
            }
            json_response(&state.lock().expect("state lock").snapshot())
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

    thread::spawn(move || {
        let completion = task.run();
        state
            .lock()
            .expect("state lock")
            .commit_engine_search(completion);
    });
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
    fn browser_url_rewrites_unspecified_bind_addresses() {
        assert_eq!(browser_url("0.0.0.0:18080"), "http://127.0.0.1:18080");
        assert_eq!(browser_url("[::]:18080"), "http://[::1]:18080");
        assert_eq!(browser_url("127.0.0.1:18080"), "http://127.0.0.1:18080");
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
    canvas {
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
      gap: 12px;
      margin: 20px 0;
    }
    .new-game {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
    }
    .profile-switch, .rule-switch {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
    }
    .actions {
      display: grid;
      grid-template-columns: 1fr;
      gap: 12px;
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
    .hint {
      color: #6b5235;
      font-size: 14px;
      line-height: 1.45;
      margin-top: -4px;
    }
    .error-text { color: var(--danger); margin-top: 10px; font-weight: 800; font-size: 16px; }
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
      <div id="status" class="status">加载中...</div>
      <div id="error" class="error-text"></div>
      <div class="controls">
        <div class="new-game">
          <button class="primary" onclick="newGame('black')">执黑</button>
          <button class="primary" onclick="newGame('white')">执白</button>
        </div>
        <div class="rule-switch">
          <button id="rule-freestyle" class="secondary" onclick="selectRule('freestyle')">无禁手</button>
          <button id="rule-renju" class="secondary" onclick="selectRule('renju')">有禁手</button>
        </div>
        <div class="profile-switch">
          <button id="profile-base" class="secondary" onclick="setProfile('base')">Base</button>
          <button id="profile-fast" class="secondary" onclick="setProfile('fast')">Fast</button>
        </div>
        <div class="actions">
          <button class="warn" onclick="undo()">悔棋</button>
        </div>
        <div class="hint">棋盘会自动刷新；规则只在新局生效，点击“执黑/执白”会按当前规则和颜色重新开局。Base/Fast 可在引擎未思考时切换，只影响下一次引擎思考。快捷键：U 悔棋，R 按当前执棋方重新开局。</div>
      </div>
      <div class="kv" id="info"></div>
    </aside>
  </main>
  <script>
    const canvas = document.getElementById('board');
    const ctx = canvas.getContext('2d');
    let state = null;
    let selectedRule = 'freestyle';

    async function api(path) {
      const res = await fetch(path, { method: path === '/state' ? 'GET' : 'POST' });
      if (!res.ok) throw new Error(await res.text());
      state = await res.json();
      draw();
      renderInfo();
    }
    function refresh() { api('/state').catch(showError); }
    function newGame(side) { api('/new?side=' + side + '&rule=' + selectedRule).catch(showError); }
    function setProfile(profile) { api('/profile?value=' + profile).catch(showError); }
    function selectRule(rule) {
      selectedRule = rule;
      renderRuleButtons();
    }
    function restartSameSide() {
      const side = state && state.human_side === -1 ? 'white' : 'black';
      newGame(side);
    }
    function undo() { api('/undo').catch(showError); }
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
      return v === 1 ? '黑' : (v === -1 ? '白' : '无');
    }
    function yesNo(v) {
      return v ? '是' : '否';
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
    function renderInfo() {
      const status = document.getElementById('status');
      status.textContent = state.status;
      status.className = 'status' + (state.engine_thinking ? ' thinking' : '') + (state.error ? ' error' : '');
      document.getElementById('error').textContent = state.error || '';
      const r = state.last_result;
      const t = state.last_trace;
      const p = state.params;
      const baseButton = document.getElementById('profile-base');
      const fastButton = document.getElementById('profile-fast');
      if (baseButton && fastButton) {
        baseButton.className = 'secondary' + (p.profile === 'base' ? ' active' : '');
        fastButton.className = 'secondary' + (p.profile === 'fast' ? ' active' : '');
        baseButton.disabled = !!state.engine_thinking;
        fastButton.disabled = !!state.engine_thinking;
      }
      renderRuleButtons();
      const rows = [
        ['你执', sideName(state.human_side)],
        ['轮到', sideName(state.side_to_move)],
        ['胜者', sideName(state.winner)],
        ['手数', state.move_count],
        ['规则', p.rule === 'renju' ? '有禁手' : '无禁手'],
        ['引擎模式', `${p.profile === 'fast' ? 'Fast' : 'Base'}${p.fast_history_ordering ? ' / history+killer' : ''}`],
        ['搜索参数', `d${p.depth} / w${p.width}`],
        ['VCF/VCT', `${p.compute_vcf ? 'VCF' + p.root_vcf_depth : 'VCF off'} / ${p.compute_vct ? 'VCT' + p.root_vct_depth : 'VCT off'}`],
        ['Overlap VCT/AB', yesNo(p.overlap_vct_alphabeta)],
        ['窗口', p.static_board ? 'static board' : `dynamic margin ${p.dynamic_board_margin}`],
        ['上次搜索', r ? `${r.ms?.toFixed(3)} ms, depth ${r.depth}, nodes ${r.nodes}, score ${r.score}` : '-'],
        ['VCF 使用/命中', t ? `${yesNo(t.used_vcf)} / ${yesNo(t.vcf_found)}` : '-'],
        ['VCT 使用/触发', t ? `${yesNo(t.used_vct)} / ${yesNo(t.vct_triggered)}` : '-'],
        ['VCT 结果/耗时', t ? `${yesNo(t.vct_found)} / ${t.vct_ms == null ? '-' : t.vct_ms.toFixed(3) + ' ms'}` : '-'],
        ['AB 耗时', t && t.alphabeta_ms != null ? `${t.alphabeta_ms.toFixed(3)} ms` : '-'],
        ['Fast ordering', t ? `${yesNo(t.fast_history_ordering)} / killer ${t.killer_hits}, history ${t.history_hits}` : '-'],
        ['Overlap', t ? `${yesNo(t.overlap_used)} / AB ${t.overlap_ab_ms == null ? '-' : t.overlap_ab_ms.toFixed(3) + ' ms'}` : '-'],
        ['Overlap 等待/TT', t ? `${t.overlap_wait_ms == null ? '-' : t.overlap_wait_ms.toFixed(3) + ' ms'} / ${t.tt_snapshot_ms == null ? '-' : t.tt_snapshot_ms.toFixed(3) + ' ms'}` : '-'],
      ];
      document.getElementById('info').innerHTML = rows.map(([k,v]) => `<div>${k}</div><div>${v}</div>`).join('');
    }
    setInterval(refresh, 500);
    refresh();
  </script>
</body>
</html>
"#;
