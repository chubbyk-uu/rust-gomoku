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
const UiLogic = window.GomokuUiLogic;
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
const difficultyNames = {
  beginner: "入门",
  junior: "初级",
  intermediate: "中级",
  senior: "高级",
  master: "大师",
  custom: "自定义",
};

let lastState = null;     // latest controller snapshot
let ghost = null;         // pending stone awaiting same-point confirmation: {x,y}
let busy = false;         // a native round-trip (incl. engine search) is running
let showNumbers = false;  // render every move number vs. only the last marker
let soundOn = true;       // play a click when a stone is placed
let prevMoveCount = null; // tracks placements to trigger the stone sound
let announcedResult = null;

// Board geometry, recomputed on resize (CSS-pixel logical space).
let size = 0, cell = 0, margin = 0;

/* ===================================================================== *
 * Geometry + rendering (devicePixelRatio aware).
 * ===================================================================== */
function resize() {
  const pad = 12; // #board-wrap padding (6px) * 2
  const dimensions = UiLogic.boardDimensions(
    area.clientWidth,
    area.clientHeight,
    window.innerWidth,
    window.innerHeight,
    pad,
  );
  const outerSize = dimensions.outerSize;
  size = dimensions.canvasSize;
  if (size <= 0) return;

  wrap.style.width = outerSize + "px";
  wrap.style.height = outerSize + "px";

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

// Stone-placement sound, synthesized via Web Audio (no bundled asset). The
// AudioContext starts suspended until a user gesture, so unlockAudio() is also
// called from the first pointer interaction.
let audioCtx = null;
function unlockAudio() {
  try {
    if (!audioCtx) {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return;
      audioCtx = new AC();
    }
    if (audioCtx.state === "suspended") audioCtx.resume();
  } catch (e) { /* audio unsupported */ }
}
function playStoneSound() {
  if (!soundOn) return;
  unlockAudio();
  if (!audioCtx || audioCtx.state !== "running") return;
  const ctx = audioCtx;
  const now = ctx.currentTime;
  // Sharp filtered noise burst — the wooden "clack".
  const dur = 0.05;
  const frames = Math.max(1, Math.floor(ctx.sampleRate * dur));
  const buffer = ctx.createBuffer(1, frames, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < frames; i += 1) {
    const decay = 1 - i / frames;
    data[i] = (Math.random() * 2 - 1) * decay * decay;
  }
  const noise = ctx.createBufferSource();
  noise.buffer = buffer;
  const bandpass = ctx.createBiquadFilter();
  bandpass.type = "bandpass";
  bandpass.frequency.value = 1900;
  bandpass.Q.value = 0.9;
  const noiseGain = ctx.createGain();
  noiseGain.gain.value = 0.5;
  noise.connect(bandpass);
  bandpass.connect(noiseGain);
  noiseGain.connect(ctx.destination);
  noise.start(now);
  noise.stop(now + dur);
  // Low pitched "thock" body for weight.
  const osc = ctx.createOscillator();
  osc.type = "sine";
  osc.frequency.setValueAtTime(230, now);
  osc.frequency.exponentialRampToValueAtTime(120, now + 0.045);
  const oscGain = ctx.createGain();
  oscGain.gain.setValueAtTime(0.45, now);
  oscGain.gain.exponentialRampToValueAtTime(0.0001, now + 0.05);
  osc.connect(oscGain);
  oscGain.connect(ctx.destination);
  osc.start(now);
  osc.stop(now + 0.05);
}

function render(state) {
  lastState = state;
  // play a click when exactly one stone was added since the last render
  // (covers both the human's confirmed move and each engine reply, but not
  // undo, new game, ghost previews, or the optimistic thinking re-render)
  if (prevMoveCount !== null && state.move_count === prevMoveCount + 1) {
    playStoneSound();
  }
  prevMoveCount = state.move_count;
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
  document.getElementById("meta-difficulty").textContent =
    difficultyNames[state.params.difficulty] || state.params.difficulty;
  document.getElementById("meta-moves").textContent = "第 " + state.move_count + " 手";
  setSegmentValue("opt-side", state.human_side === WHITE ? "white" : "black");
  setSegmentValue("opt-rule", state.params.rule);
  setSegmentValue("opt-mode", state.params.profile);
  if (state.params.difficulty !== "custom") {
    setDifficultyValue(state.params.difficulty);
  }
  // controls
  UiLogic.syncBusyControls(document, busy, state);
  // advanced panel
  fillAdvanced(state.last_result, state.params);
  draw();
  showResultIfNeeded(state);
}

function fillAdvanced(res, params) {
  const set = (key, val) => {
    const el = document.querySelector(`[data-adv="${key}"]`);
    if (el) el.textContent = val;
  };
  set("difficulty", difficultyNames[params.difficulty] || params.difficulty || "—");
  if (!res) { set("score", "—"); set("depth", "—"); set("nodes", "—"); set("ms", "—"); return; }
  set("score", res.score);
  set("depth", res.depth);
  set("nodes", res.nodes != null ? res.nodes.toLocaleString() : "—");
  set("ms", res.ms != null ? Math.round(res.ms) + " ms" : "—");
}

function setResultOpen(open) {
  const dialog = document.getElementById("result-dialog");
  const backdrop = document.getElementById("result-backdrop");
  dialog.classList.toggle("open", open);
  dialog.setAttribute("aria-hidden", String(!open));
  backdrop.classList.toggle("open", open);
  backdrop.setAttribute("aria-hidden", String(!open));
}

function showResultIfNeeded(state) {
  const result = UiLogic.gameResult(state);
  if (!result) {
    announcedResult = null;
    setResultOpen(false);
    return;
  }
  if (announcedResult === result.key) return;
  announcedResult = result.key;
  document.getElementById("result-title").textContent = result.title;
  document.getElementById("result-message").textContent = result.message;
  document.getElementById("result-mark").className = result.tone;
  setResultOpen(true);
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
    const settledState = state;
    render(UiLogic.engineThinkingState(state));
    const moved = await nativeRequest({ op: "engine_move" });
    if (!moved.ok) {
      const message = (moved.error && moved.error.message) || "引擎落子失败";
      render({ ...settledState, engine_thinking: false, status: message, error: message });
      break;
    }
    state = moved.state;
    render(state);
  }
}

async function guarded(fn) {
  if (busy) {
    toast("引擎思考中，请稍候");
    return false;
  }
  busy = true;
  if (lastState) render(lastState);
  try { await fn(); }
  catch (err) { statusMsg.textContent = String(err); statusMsg.classList.add("error"); }
  finally { busy = false; if (lastState) render(lastState); }
  return true;
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
  UiLogic.setSheetOpen(document, name, true);
}
function closeSheet(name) {
  UiLogic.setSheetOpen(document, name, false);
}

function segValue(groupId) {
  return document.querySelector(`#${groupId} button[aria-pressed="true"]`).dataset.value;
}
function setSegmentValue(groupId, value) {
  for (const button of document.querySelectorAll(`#${groupId} button`)) {
    button.setAttribute("aria-pressed", String(button.dataset.value === value));
  }
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

function setDifficultyOpen(open) {
  const dialog = document.getElementById("difficulty-dialog");
  const backdrop = document.getElementById("difficulty-backdrop");
  dialog.classList.toggle("open", open);
  dialog.setAttribute("aria-hidden", String(!open));
  backdrop.classList.toggle("open", open);
  backdrop.setAttribute("aria-hidden", String(!open));
}

function setDifficultyValue(value) {
  const trigger = document.getElementById("opt-difficulty");
  trigger.dataset.value = value;
  document.getElementById("difficulty-summary").textContent =
    difficultyNames[value] || value;
  for (const button of document.querySelectorAll(".difficulty-option")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === value));
  }
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

  document.getElementById("btn-newgame").addEventListener("click", () => {
    if (busy || (lastState && lastState.engine_thinking)) {
      toast("引擎思考中，请稍候");
      return;
    }
    openSheet("newgame");
  });
  document.getElementById("btn-more").addEventListener("click", () => openSheet("more"));
  document.getElementById("btn-result-close").addEventListener("click", () => {
    setResultOpen(false);
  });
  document.getElementById("btn-result-newgame").addEventListener("click", () => {
    setResultOpen(false);
    openSheet("newgame");
  });

  for (const el of document.querySelectorAll("[data-close]")) {
    el.addEventListener("click", () => closeSheet(el.dataset.close));
  }
  for (const el of document.querySelectorAll(".sheet-backdrop")) {
    el.addEventListener("click", () => closeSheet(el.dataset.backdrop));
  }

  bindSegmented("opt-side");
  bindSegmented("opt-rule");
  bindSegmented("opt-mode");
  document.getElementById("opt-difficulty").addEventListener("click", () => {
    if (!busy && !(lastState && lastState.engine_thinking)) {
      setDifficultyOpen(true);
    }
  });
  document.getElementById("btn-difficulty-close").addEventListener("click", () => {
    setDifficultyOpen(false);
  });
  document.getElementById("difficulty-backdrop").addEventListener("click", () => {
    setDifficultyOpen(false);
  });
  document.getElementById("difficulty-options").addEventListener("click", (event) => {
    const button = event.target.closest(".difficulty-option");
    if (!button) return;
    setDifficultyValue(button.dataset.value);
    setDifficultyOpen(false);
  });

  document.getElementById("btn-start").addEventListener("click", () => {
    if (busy || (lastState && lastState.engine_thinking)) {
      toast("引擎思考中，请稍候");
      return;
    }
    const human_side = segValue("opt-side");
    const rule = segValue("opt-rule");
    const profile = segValue("opt-mode");
    const difficulty = document.getElementById("opt-difficulty").dataset.value;
    closeSheet("newgame");
    guarded(async () => {
      ghost = null;
      await nativeRequest({ op: "set_profile", profile });
      await nativeRequest({ op: "set_difficulty", difficulty });
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

  const soundBtn = document.getElementById("opt-sound");
  soundBtn.addEventListener("click", () => {
    soundOn = !soundOn;
    soundBtn.setAttribute("aria-pressed", String(soundOn));
    soundBtn.textContent = soundOn ? "开" : "关";
    if (soundOn) unlockAudio();
  });

  // Resume the AudioContext on the first user gesture so engine-first moves can
  // also play (Web Audio starts suspended until a gesture).
  window.addEventListener("pointerdown", unlockAudio);

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
