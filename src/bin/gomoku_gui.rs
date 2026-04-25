//! Local browser GUI for playing against the engine.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use serde::Serialize;

use rust_gomoku::{
    load_default_config, move_to_xy, xy_to_move, Board, EngineConfig, RootSearcher, RootTrace,
    SearchLimits, SearchResult, Side, BLACK, BOARD_SIZE, EMPTY, WHITE,
};

const DEFAULT_ADDR: &str = "127.0.0.1:7878";

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("warning: debug build is much slower; use `cargo run --release --bin gomoku_gui` for real play.");
    }
    let args = parse_args();
    let mut config = load_default_config();
    if let Some(depth) = args.depth {
        config.root_search.depth = depth;
    }
    if let Some(width) = args.width {
        config.root_search.wide = width;
        config.root_search.timed_max_wide = width;
    }
    let state = Arc::new(Mutex::new(GameState::new(config)));
    let addr = args.addr;
    let listener = TcpListener::bind(&addr).expect("GUI server binds to local address");
    eprintln!("gomoku gui listening on http://{addr}");
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
}

fn parse_args() -> GuiArgs {
    let mut result = GuiArgs {
        addr: DEFAULT_ADDR.to_string(),
        depth: None,
        width: None,
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
            _ => {}
        }
    }
    result
}

#[derive(Clone)]
struct GameState {
    config: EngineConfig,
    board: Board,
    human_side: Side,
    engine_thinking: bool,
    status: String,
    error: Option<String>,
    last_mark: Option<(usize, usize)>,
    last_result: Option<SearchResult>,
    last_trace: Option<RootTrace>,
    last_search_ms: Option<f64>,
    game_id: u64,
}

impl GameState {
    fn new(config: EngineConfig) -> Self {
        Self {
            config,
            board: Board::new(),
            human_side: BLACK,
            engine_thinking: false,
            status: "请选择执黑或执白，然后开始对局。".to_string(),
            error: None,
            last_mark: None,
            last_result: None,
            last_trace: None,
            last_search_ms: None,
            game_id: 0,
        }
    }

    fn reset(&mut self, human_side: Side) {
        self.game_id = self.game_id.wrapping_add(1);
        self.board.reset();
        self.human_side = human_side;
        self.engine_thinking = false;
        self.status = if human_side == BLACK {
            "新局开始：你执黑，请落子。".to_string()
        } else {
            "新局开始：你执白，引擎执黑思考中。".to_string()
        };
        self.error = None;
        self.last_mark = None;
        self.last_result = None;
        self.last_trace = None;
        self.last_search_ms = None;
    }

    fn board_cells(&self) -> Vec<i8> {
        let mut cells = Vec::with_capacity(BOARD_SIZE * BOARD_SIZE);
        for y in 0..BOARD_SIZE {
            for x in 0..BOARD_SIZE {
                cells.push(self.board.at(x, y).expect("coordinates stay in range"));
            }
        }
        cells
    }

    fn can_human_play(&self) -> bool {
        !self.engine_thinking
            && self.board.winner() == EMPTY
            && self.board.side_to_move() == self.human_side
    }

    fn search_limits(&self) -> SearchLimits {
        SearchLimits::fixed_from_config(&self.config)
    }
}

#[derive(Serialize)]
struct StateResponse {
    board_size: usize,
    cells: Vec<i8>,
    moves: Vec<MoveResponse>,
    human_side: i8,
    side_to_move: i8,
    winner: i8,
    move_count: usize,
    can_play: bool,
    engine_thinking: bool,
    status: String,
    error: Option<String>,
    last_mark: Option<[usize; 2]>,
    last_result: Option<ResultResponse>,
    last_trace: Option<TraceResponse>,
    params: ParamsResponse,
}

#[derive(Serialize)]
struct MoveResponse {
    x: usize,
    y: usize,
    side: i8,
    number: usize,
}

#[derive(Serialize)]
struct ResultResponse {
    move_xy: [usize; 2],
    score: i32,
    depth: i32,
    nodes: usize,
    ms: Option<f64>,
}

#[derive(Serialize)]
struct TraceResponse {
    used_vcf: bool,
    vcf_found: bool,
    used_vct: bool,
    vct_triggered: bool,
    vct_ms: Option<f64>,
    vct_found: bool,
    vct_accepted: bool,
    vct_reject_reason: Option<&'static str>,
}

#[derive(Serialize)]
struct ParamsResponse {
    depth: i32,
    width: usize,
    compute_vcf: bool,
    root_vcf_depth: i32,
    opponent_vcf_depth: i32,
    compute_vct: bool,
    root_vct_depth: i32,
    static_board: bool,
    dynamic_board_margin: i32,
    lazy_smp: bool,
}

fn snapshot_state(state: &GameState) -> StateResponse {
    let last_result = state.last_result.map(|result| {
        let (x, y) = move_to_xy(result.move_).expect("engine move stays valid");
        ResultResponse {
            move_xy: [x, y],
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
            ms: state.last_search_ms,
        }
    });
    let last_trace = state.last_trace.as_ref().map(|trace| TraceResponse {
        used_vcf: trace.used_vcf,
        vcf_found: trace.vcf_found,
        used_vct: trace.used_vct,
        vct_triggered: trace.vct_triggered,
        vct_ms: trace.vct_ms,
        vct_found: trace.vct_found,
        vct_accepted: trace.vct_accepted,
        vct_reject_reason: trace.vct_reject_reason,
    });
    let limits = state.search_limits();
    StateResponse {
        board_size: BOARD_SIZE,
        cells: state.board_cells(),
        moves: state
            .board
            .move_history()
            .iter()
            .enumerate()
            .filter_map(|(index, played)| {
                let (x, y) = move_to_xy(played.move_).ok()?;
                Some(MoveResponse {
                    x,
                    y,
                    side: played.side,
                    number: index + 1,
                })
            })
            .collect(),
        human_side: state.human_side,
        side_to_move: state.board.side_to_move(),
        winner: state.board.winner(),
        move_count: state.board.move_count(),
        can_play: state.can_human_play(),
        engine_thinking: state.engine_thinking,
        status: state.status.clone(),
        error: state.error.clone(),
        last_mark: state.last_mark.map(|(x, y)| [x, y]),
        last_result,
        last_trace,
        params: ParamsResponse {
            depth: limits.max_depth,
            width: limits.root_width,
            compute_vcf: state.config.runtime.compute_vcf,
            root_vcf_depth: state.config.runtime.root_vcf_depth,
            opponent_vcf_depth: state.config.runtime.opponent_vcf_depth,
            compute_vct: state.config.runtime.compute_vct,
            root_vct_depth: state.config.runtime.root_vct_depth,
            static_board: state.config.runtime.static_board,
            dynamic_board_margin: state.config.runtime.dynamic_board_margin,
            lazy_smp: state.config.runtime.lazy_smp,
        },
    }
}

fn handle_client(mut stream: TcpStream, state: Arc<Mutex<GameState>>) {
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
        ("GET", "/state") => json_response(&snapshot_state(&state.lock().expect("state lock"))),
        ("POST", "/new") => {
            let side = query_param(query, "side")
                .and_then(parse_side)
                .unwrap_or(BLACK);
            {
                let mut game = state.lock().expect("state lock");
                game.reset(side);
            }
            if side == WHITE {
                maybe_start_engine(Arc::clone(&state));
            }
            json_response(&snapshot_state(&state.lock().expect("state lock")))
        }
        ("POST", "/play") => {
            let x = query_param(query, "x").and_then(|value| value.parse::<usize>().ok());
            let y = query_param(query, "y").and_then(|value| value.parse::<usize>().ok());
            let mut should_engine_move = false;
            {
                let mut game = state.lock().expect("state lock");
                game.error = None;
                if !game.can_human_play() {
                    game.error = Some("现在不能落子。".to_string());
                } else if let (Some(x), Some(y)) = (x, y) {
                    match xy_to_move(x, y).and_then(|move_| game.board.play(move_, None)) {
                        Ok(_) => {
                            game.last_mark = Some((x, y));
                            if game.board.winner() == game.human_side {
                                game.status = "你赢了。".to_string();
                            } else {
                                game.status = "你已落子，引擎思考中。".to_string();
                                should_engine_move = true;
                            }
                        }
                        Err(err) => {
                            game.error = Some(format!("非法落子：{err:?}"));
                        }
                    }
                } else {
                    game.error = Some("缺少坐标。".to_string());
                }
            }
            if should_engine_move {
                maybe_start_engine(Arc::clone(&state));
            }
            json_response(&snapshot_state(&state.lock().expect("state lock")))
        }
        ("POST", "/undo") => {
            {
                let mut game = state.lock().expect("state lock");
                if game.engine_thinking {
                    game.error = Some("引擎思考中，暂不能悔棋。".to_string());
                } else {
                    game.error = None;
                    undo_turn(&mut game);
                }
            }
            json_response(&snapshot_state(&state.lock().expect("state lock")))
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

fn undo_turn(game: &mut GameState) {
    if game.board.move_count() == 0 {
        game.error = Some("当前没有可悔棋步。".to_string());
        return;
    }
    let mut undone = 0;
    while game.board.move_count() > 0 && undone < 2 {
        if game.board.undo().is_ok() {
            undone += 1;
        }
        if game.board.side_to_move() == game.human_side {
            break;
        }
    }
    game.last_mark = last_move_xy(&game.board);
    game.last_result = None;
    game.last_trace = None;
    game.last_search_ms = None;
    game.status = if game.board.side_to_move() == game.human_side {
        "已悔棋，请继续落子。".to_string()
    } else if game.board.move_count() == 0 && game.human_side == WHITE {
        "已撤回引擎首手；点击“我执白”可重新让引擎开局。".to_string()
    } else {
        "已悔棋，当前不是你的回合。".to_string()
    };
}

fn last_move_xy(board: &Board) -> Option<(usize, usize)> {
    board
        .move_history()
        .last()
        .and_then(|played| move_to_xy(played.move_).ok())
}

fn maybe_start_engine(state: Arc<Mutex<GameState>>) {
    let (board, config, limits, game_id) = {
        let mut game = state.lock().expect("state lock");
        if game.engine_thinking
            || game.board.winner() != EMPTY
            || game.board.side_to_move() == game.human_side
        {
            return;
        }
        game.engine_thinking = true;
        game.error = None;
        game.status = "引擎思考中...".to_string();
        (
            game.board.clone(),
            game.config.clone(),
            game.search_limits(),
            game.game_id,
        )
    };

    thread::spawn(move || {
        let mut board_for_search = board;
        let mut searcher = RootSearcher::new(config);
        let start = Instant::now();
        let result = searcher.search(&mut board_for_search, Some(limits));
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let trace = searcher.last_trace.clone();
        let mut game = state.lock().expect("state lock");
        if game.game_id != game_id {
            return;
        }
        if game.board.winner() == EMPTY && game.board.side_to_move() != game.human_side {
            match game.board.play(result.move_, None) {
                Ok(_) => {
                    game.last_mark = move_to_xy(result.move_).ok();
                    game.last_result = Some(result);
                    game.last_trace = trace;
                    game.last_search_ms = Some((elapsed_ms * 1000.0).round() / 1000.0);
                    game.status = if game.board.winner() == EMPTY {
                        "引擎已落子，轮到你。".to_string()
                    } else {
                        "引擎获胜。".to_string()
                    };
                }
                Err(err) => {
                    game.error = Some(format!("引擎返回非法落子：{err:?}"));
                    game.status = "引擎落子失败。".to_string();
                }
            }
        }
        game.engine_thinking = false;
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
        <div class="actions">
          <button class="warn" onclick="undo()">悔棋</button>
        </div>
        <div class="hint">棋盘会自动刷新；点击“执黑/执白”会按对应颜色重新开局。快捷键：U 悔棋，R 按当前执棋方重新开局。</div>
      </div>
      <div class="kv" id="info"></div>
    </aside>
  </main>
  <script>
    const canvas = document.getElementById('board');
    const ctx = canvas.getContext('2d');
    let state = null;

    async function api(path) {
      const res = await fetch(path, { method: path === '/state' ? 'GET' : 'POST' });
      if (!res.ok) throw new Error(await res.text());
      state = await res.json();
      draw();
      renderInfo();
    }
    function refresh() { api('/state').catch(showError); }
    function newGame(side) { api('/new?side=' + side).catch(showError); }
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
      const lastNumber = state.moves.length;
      for (const move of state.moves) {
        stone(move.x, move.y, move.side, move.number, move.number === lastNumber);
      }
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
    function renderInfo() {
      const status = document.getElementById('status');
      status.textContent = state.status;
      status.className = 'status' + (state.engine_thinking ? ' thinking' : '') + (state.error ? ' error' : '');
      document.getElementById('error').textContent = state.error || '';
      const r = state.last_result;
      const t = state.last_trace;
      const p = state.params;
      const rows = [
        ['你执', sideName(state.human_side)],
        ['轮到', sideName(state.side_to_move)],
        ['胜者', sideName(state.winner)],
        ['手数', state.move_count],
        ['搜索参数', `d${p.depth} / w${p.width}`],
        ['VCF/VCT', `${p.compute_vcf ? 'VCF' + p.root_vcf_depth : 'VCF off'} / ${p.compute_vct ? 'VCT' + p.root_vct_depth : 'VCT off'}`],
        ['窗口', p.static_board ? 'static board' : `dynamic margin ${p.dynamic_board_margin}`],
        ['Lazy SMP', p.lazy_smp ? 'on' : 'off'],
        ['上次搜索', r ? `${r.ms?.toFixed(3)} ms, depth ${r.depth}, nodes ${r.nodes}, score ${r.score}` : '-'],
        ['VCF 使用/命中', t ? `${yesNo(t.used_vcf)} / ${yesNo(t.vcf_found)}` : '-'],
        ['VCT 使用/触发', t ? `${yesNo(t.used_vct)} / ${yesNo(t.vct_triggered)}` : '-'],
        ['VCT 结果/耗时', t ? `${yesNo(t.vct_found)} / ${t.vct_ms == null ? '-' : t.vct_ms.toFixed(3) + ' ms'}` : '-'],
      ];
      document.getElementById('info').innerHTML = rows.map(([k,v]) => `<div>${k}</div><div>${v}</div>`).join('');
    }
    setInterval(refresh, 500);
    refresh();
  </script>
</body>
</html>
"#;
