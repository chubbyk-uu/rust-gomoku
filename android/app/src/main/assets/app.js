"use strict";

/* ===================================================================== *
 * Native transport: a promise wrapper over the @JavascriptInterface
 * bridge exposed by MainActivity (AndroidBridge.request(json, callbackId)).
 * Responses arrive on window.__onNativeResult(callbackId, responseJson).
 * ===================================================================== */
const pending = new Map();
let nextCallId = 0;

window.__onNativeResult = function (callbackId, responseJson) {
  const entry = pending.get(callbackId);
  if (!entry) return;
  pending.delete(callbackId);
  try {
    entry.resolve(JSON.parse(responseJson));
  } catch (err) {
    entry.reject(err);
  }
};

function nativeRequest(payload) {
  return new Promise((resolve, reject) => {
    if (!window.AndroidBridge || typeof window.AndroidBridge.request !== "function") {
      reject(new Error("native bridge unavailable"));
      return;
    }
    const id = "c" + (nextCallId++);
    pending.set(id, { resolve, reject });
    window.AndroidBridge.request(JSON.stringify(payload), id);
  });
}

/* ===================================================================== *
 * Constants and DOM references.
 * ===================================================================== */
const BLACK = 1, WHITE = -1, EMPTY = 0;
const GRID = 15;
const STARS = [[3, 3], [11, 3], [7, 7], [3, 11], [11, 11]];
const COLORS = {
  board: "#d9a85f", line: "#33291d", black: "#191714", white: "#f7f5ef",
  accent: "#176b55", danger: "#b3331f",
};

const canvas = document.getElementById("board");
const wrap = document.getElementById("board-wrap");
const area = document.getElementById("board-area");
const ctx = canvas.getContext("2d");
const statusMsg = document.getElementById("status-msg");
const turnDot = document.getElementById("turn-dot");
const toastEl = document.getElementById("toast");

let lastState = null;     // latest controller snapshot
let ghost = null;         // pending stone awaiting same-point confirmation: {x,y}
let busy = false;         // a native round-trip (incl. engine search) is running
let showNumbers = false;  // render every move number vs. only the last marker

// Board geometry, recomputed on resize (CSS-pixel logical space).
let size = 0, cell = 0, margin = 0;

/* ===================================================================== *
 * Geometry + rendering (devicePixelRatio aware).
 * ===================================================================== */
function resize() {
  const pad = 12; // #board-wrap padding (6px) * 2
  const avail = Math.min(area.clientWidth, area.clientHeight);
  size = Math.max(0, Math.floor(avail - pad));
  if (size <= 0) return;

  wrap.style.width = size + "px";
  wrap.style.height = size + "px";

  const dpr = window.devicePixelRatio || 1;
  canvas.style.width = size + "px";
  canvas.style.height = size + "px";
  canvas.width = Math.round(size * dpr);
  canvas.height = Math.round(size * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0); // draw in CSS-pixel coordinates

  cell = size / 15.2;
  margin = cell * 0.6;
  draw();
}

function px(i) { return margin + i * cell; }

function stone(cx, cy, r, side) {
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.fillStyle = side === BLACK ? COLORS.black : COLORS.white;
  ctx.fill();
  ctx.lineWidth = 1;
  ctx.strokeStyle = COLORS.line;
  ctx.stroke();
}

function draw() {
  if (size <= 0) return;
  ctx.clearRect(0, 0, size, size);
  ctx.fillStyle = COLORS.board;
  ctx.fillRect(0, 0, size, size);

  // grid
  ctx.strokeStyle = COLORS.line;
  ctx.lineWidth = Math.max(1, size / 480);
  for (let i = 0; i < GRID; i += 1) {
    const p = px(i);
    ctx.beginPath(); ctx.moveTo(px(0), p); ctx.lineTo(px(14), p); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(p, px(0)); ctx.lineTo(p, px(14)); ctx.stroke();
  }
  // star points
  ctx.fillStyle = COLORS.line;
  for (const [x, y] of STARS) {
    ctx.beginPath(); ctx.arc(px(x), px(y), Math.max(2, cell * 0.08), 0, Math.PI * 2); ctx.fill();
  }

  const state = lastState;
  if (!state) return;

  // forbidden crosses (Renju, black to move only; controller-provided)
  ctx.strokeStyle = COLORS.danger;
  ctx.lineWidth = Math.max(2, cell * 0.07);
  for (const fp of state.forbidden_points || []) {
    const cx = px(fp.x), cy = px(fp.y), r = cell * 0.26;
    ctx.beginPath();
    ctx.moveTo(cx - r, cy - r); ctx.lineTo(cx + r, cy + r);
    ctx.moveTo(cx + r, cy - r); ctx.lineTo(cx - r, cy + r);
    ctx.stroke();
  }

  // stones
  const r = cell * 0.44;
  const cells = state.cells || [];
  for (let y = 0; y < GRID; y += 1) {
    for (let x = 0; x < GRID; x += 1) {
      const v = cells[y * GRID + x];
      if (v !== EMPTY) stone(px(x), px(y), r, v);
    }
  }

  // move numbers (optional) or last-move marker
  if (showNumbers) {
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.font = `${Math.round(cell * 0.42)}px system-ui, sans-serif`;
    for (const m of state.moves || []) {
      ctx.fillStyle = m.side === BLACK ? "#f3f1eb" : "#191714";
      ctx.fillText(String(m.number), px(m.x), px(m.y));
    }
  } else if (state.last_mark) {
    const [mx, my] = state.last_mark;
    ctx.beginPath();
    ctx.arc(px(mx), px(my), Math.max(3, cell * 0.12), 0, Math.PI * 2);
    ctx.fillStyle = COLORS.accent;
    ctx.fill();
  }

  // ghost (pending confirmation)
  if (ghost) {
    const side = state.side_to_move;
    ctx.globalAlpha = 0.45;
    stone(px(ghost.x), px(ghost.y), r, side);
    ctx.globalAlpha = 1;
    ctx.strokeStyle = COLORS.accent;
    ctx.lineWidth = Math.max(2, cell * 0.06);
    ctx.beginPath();
    ctx.arc(px(ghost.x), px(ghost.y), r + cell * 0.12, 0, Math.PI * 2);
    ctx.stroke();
  }
}

function locate(evt) {
  const rect = canvas.getBoundingClientRect();
  const lx = (evt.clientX - rect.left) * (size / rect.width);
  const ly = (evt.clientY - rect.top) * (size / rect.height);
  const x = Math.round((lx - margin) / cell);
  const y = Math.round((ly - margin) / cell);
  if (x < 0 || y < 0 || x >= GRID || y >= GRID) return null;
  // reject taps that land too far from any intersection (reduces mis-taps)
  if (Math.hypot(lx - px(x), ly - px(y)) > cell * 0.6) return null;
  return { x, y };
}

/* ===================================================================== *
 * UI helpers.
 * ===================================================================== */
let toastTimer = null;
function toast(text) {
  toastEl.textContent = text;
  toastEl.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove("show"), 1800);
}
function haptic(ms) {
  try { if (navigator.vibrate) navigator.vibrate(ms); } catch (e) { /* unsupported */ }
}

function render(state) {
  lastState = state;
  // turn indicator
  turnDot.className = "";
  if (state.engine_thinking) {
    turnDot.classList.add("thinking");
  } else if (state.winner !== EMPTY) {
    turnDot.classList.add("none");
  } else {
    turnDot.classList.add(state.side_to_move === WHITE ? "white" : "black");
  }
  // status text
  const msg = state.error || state.status || "";
  statusMsg.textContent = msg;
  statusMsg.classList.toggle("error", Boolean(state.error));
  document.getElementById("statusbar").classList.toggle("thinking", Boolean(state.engine_thinking) && !state.error);
  // title-bar meta
  const isRenju = state.params && state.params.rule === "renju";
  const ruleChip = document.getElementById("meta-rule");
  ruleChip.textContent = isRenju ? "有禁手" : "无禁手";
  ruleChip.classList.toggle("renju", isRenju);
  document.getElementById("meta-side").textContent = state.human_side === WHITE ? "你执白" : "你执黑";
  document.getElementById("meta-moves").textContent = "第 " + state.move_count + " 手";
  // controls
  document.getElementById("btn-undo").disabled = busy || state.engine_thinking || state.move_count === 0;
  // advanced panel
  fillAdvanced(state.last_result);
  draw();
}

function fillAdvanced(res) {
  const set = (key, val) => {
    const el = document.querySelector(`[data-adv="${key}"]`);
    if (el) el.textContent = val;
  };
  if (!res) { set("score", "—"); set("depth", "—"); set("nodes", "—"); set("ms", "—"); return; }
  set("score", res.score);
  set("depth", res.depth);
  set("nodes", res.nodes != null ? res.nodes.toLocaleString() : "—");
  set("ms", res.ms != null ? Math.round(res.ms) + " ms" : "—");
}

/* ===================================================================== *
 * Game flow.
 * ===================================================================== */
async function applyAndContinue(response) {
  if (!response.ok) {
    statusMsg.textContent = (response.error && response.error.message) || "请求失败";
    statusMsg.classList.add("error");
    return;
  }
  let state = response.state;
  render(state);
  while (state.winner === EMPTY && state.side_to_move !== state.human_side) {
    const moved = await nativeRequest({ op: "engine_move" });
    if (!moved.ok) {
      statusMsg.textContent = (moved.error && moved.error.message) || "引擎落子失败";
      statusMsg.classList.add("error");
      break;
    }
    state = moved.state;
    render(state);
  }
}

async function guarded(fn) {
  if (busy) return;
  busy = true;
  try { await fn(); }
  catch (err) { statusMsg.textContent = String(err); statusMsg.classList.add("error"); }
  finally { busy = false; if (lastState) render(lastState); }
}

function isForbidden(x, y) {
  return (lastState.forbidden_points || []).some((p) => p.x === x && p.y === y);
}

// Tap-preview + tap-same-point-to-confirm placement.
function onBoardTap(evt) {
  if (busy || !lastState || !lastState.can_play) return;
  const cell_ = locate(evt);
  if (!cell_) return;
  const { x, y } = cell_;

  if (lastState.cells[y * GRID + x] !== EMPTY) return; // occupied

  if (isForbidden(x, y)) {
    haptic(20);
    toast("黑棋禁手，不能落在这里");
    return;
  }

  if (ghost && ghost.x === x && ghost.y === y) {
    ghost = null; // confirm
    guarded(async () => {
      const played = await nativeRequest({ op: "play", x, y });
      await applyAndContinue(played);
    });
  } else {
    ghost = { x, y }; // preview / move preview
    haptic(8);
    statusMsg.textContent = "再次点击该点落子";
    statusMsg.classList.remove("error");
    draw();
  }
}

/* ===================================================================== *
 * Bottom sheets.
 * ===================================================================== */
function openSheet(name) {
  document.getElementById("sheet-" + name).classList.add("open");
  document.querySelector(`.sheet-backdrop[data-backdrop="${name}"]`).classList.add("open");
}
function closeSheet(name) {
  document.getElementById("sheet-" + name).classList.remove("open");
  document.querySelector(`.sheet-backdrop[data-backdrop="${name}"]`).classList.remove("open");
}

function segValue(groupId) {
  return document.querySelector(`#${groupId} button[aria-pressed="true"]`).dataset.value;
}
function bindSegmented(groupId) {
  const group = document.getElementById(groupId);
  group.addEventListener("click", (e) => {
    const btn = e.target.closest("button");
    if (!btn) return;
    for (const b of group.querySelectorAll("button")) {
      b.setAttribute("aria-pressed", String(b === btn));
    }
  });
}

/* ===================================================================== *
 * Wiring.
 * ===================================================================== */
function bindEvents() {
  canvas.addEventListener("click", onBoardTap);

  document.getElementById("btn-undo").addEventListener("click", () => guarded(async () => {
    ghost = null;
    await applyAndContinue(await nativeRequest({ op: "undo" }));
  }));

  document.getElementById("btn-newgame").addEventListener("click", () => openSheet("newgame"));
  document.getElementById("btn-more").addEventListener("click", () => openSheet("more"));

  for (const el of document.querySelectorAll("[data-close]")) {
    el.addEventListener("click", () => closeSheet(el.dataset.close));
  }
  for (const el of document.querySelectorAll(".sheet-backdrop")) {
    el.addEventListener("click", () => closeSheet(el.dataset.backdrop));
  }

  bindSegmented("opt-side");
  bindSegmented("opt-rule");
  bindSegmented("opt-mode");

  document.getElementById("btn-start").addEventListener("click", () => {
    const human_side = segValue("opt-side");
    const rule = segValue("opt-rule");
    const profile = segValue("opt-mode");
    closeSheet("newgame");
    guarded(async () => {
      ghost = null;
      await nativeRequest({ op: "set_profile", profile });
      await applyAndContinue(await nativeRequest({ op: "new_game", human_side, rule }));
    });
  });

  const numBtn = document.getElementById("opt-numbers");
  numBtn.addEventListener("click", () => {
    showNumbers = !showNumbers;
    numBtn.setAttribute("aria-pressed", String(showNumbers));
    numBtn.textContent = showNumbers ? "开" : "关";
    draw();
  });

  window.addEventListener("resize", resize);
  window.addEventListener("orientationchange", resize);
}

/* ===================================================================== *
 * Boot. Re-runs after rotation recreates the page; the native handle and
 * any in-flight search live in the ViewModel, so state() re-syncs the board.
 * ===================================================================== */
bindEvents();
resize();
guarded(async () => {
  await applyAndContinue(await nativeRequest({ op: "state" }));
});
