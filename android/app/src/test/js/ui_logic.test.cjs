"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const {
  boardDimensions,
  engineThinkingState,
  gameResult,
  setSheetOpen,
  syncBusyControls,
} = require("../../main/assets/ui_logic.js");

test("board sizing uses width in portrait and the limiting side in landscape", () => {
  assert.deepEqual(boardDimensions(390, 900, 390, 900, 12), {
    outerSize: 390,
    canvasSize: 378,
  });
  assert.deepEqual(boardDimensions(760, 430, 900, 460, 12), {
    outerSize: 430,
    canvasSize: 418,
  });
  assert.deepEqual(boardDimensions(8, 8, 390, 900, 12), {
    outerSize: 8,
    canvasSize: 0,
  });
});

function classList() {
  const classes = new Set();
  return {
    contains: (value) => classes.has(value),
    toggle(value, enabled) {
      if (enabled) classes.add(value);
      else classes.delete(value);
    },
  };
}

function element() {
  const attributes = new Map();
  return {
    classList: classList(),
    disabled: false,
    getAttribute: (name) => attributes.get(name),
    setAttribute: (name, value) => attributes.set(name, value),
  };
}

function fakeDocument() {
  const elements = {
    "btn-undo": element(),
    "btn-newgame": element(),
    "btn-start": element(),
    "sheet-newgame": element(),
    "opt-difficulty": element(),
  };
  const options = [element(), element(), element()];
  const backdrop = element();
  return {
    elements,
    options,
    backdrop,
    getElementById: (id) => elements[id],
    querySelector: (selector) => {
      assert.equal(selector, '.sheet-backdrop[data-backdrop="newgame"]');
      return backdrop;
    },
    querySelectorAll: (selector) => {
      assert.equal(
        selector,
        "#opt-side button, #opt-rule button, #opt-mode button",
      );
      return options;
    },
  };
}

test("engine thinking state disables play without mutating the settled state", () => {
  const settled = {
    can_play: false,
    engine_thinking: false,
    status: "你已落子，引擎思考中。",
    error: "old error",
    move_count: 9,
  };

  const thinking = engineThinkingState(settled);

  assert.notEqual(thinking, settled);
  assert.equal(thinking.can_play, false);
  assert.equal(thinking.engine_thinking, true);
  assert.equal(thinking.status, "引擎思考中...");
  assert.equal(thinking.error, null);
  assert.equal(settled.engine_thinking, false);
  assert.equal(settled.error, "old error");
});

test("busy and engine thinking disable every new-game control", () => {
  const document_ = fakeDocument();

  syncBusyControls(document_, true, {
    engine_thinking: false,
    move_count: 4,
  });
  assert.equal(document_.elements["btn-undo"].disabled, true);
  assert.equal(document_.elements["btn-newgame"].disabled, true);
  assert.equal(document_.elements["btn-start"].disabled, true);
  assert.equal(document_.elements["opt-difficulty"].disabled, true);
  assert.ok(document_.options.every((button) => button.disabled));

  syncBusyControls(document_, false, {
    engine_thinking: false,
    move_count: 4,
  });
  assert.equal(document_.elements["btn-undo"].disabled, false);
  assert.equal(document_.elements["btn-newgame"].disabled, false);
  assert.equal(document_.elements["btn-start"].disabled, false);
  assert.equal(document_.elements["opt-difficulty"].disabled, false);
  assert.ok(document_.options.every((button) => !button.disabled));
});

test("sheet visibility and aria-hidden stay synchronized", () => {
  const document_ = fakeDocument();
  const sheet = document_.elements["sheet-newgame"];

  setSheetOpen(document_, "newgame", true);
  assert.equal(sheet.classList.contains("open"), true);
  assert.equal(sheet.getAttribute("aria-hidden"), "false");
  assert.equal(document_.backdrop.classList.contains("open"), true);
  assert.equal(document_.backdrop.getAttribute("aria-hidden"), "false");

  setSheetOpen(document_, "newgame", false);
  assert.equal(sheet.classList.contains("open"), false);
  assert.equal(sheet.getAttribute("aria-hidden"), "true");
  assert.equal(document_.backdrop.classList.contains("open"), false);
  assert.equal(document_.backdrop.getAttribute("aria-hidden"), "true");
});

test("game result distinguishes the human outcome", () => {
  assert.equal(gameResult({ winner: 0 }), null);
  assert.deepEqual(
    gameResult({ winner: 1, human_side: 1, move_count: 23 }),
    {
      key: "1:23",
      title: "你赢了",
      message: "第 23 手取胜。",
      tone: "win",
    },
  );
  assert.deepEqual(
    gameResult({ winner: -1, human_side: 1, move_count: 24 }),
    {
      key: "-1:24",
      title: "引擎获胜",
      message: "对局在第 24 手结束。",
      tone: "loss",
    },
  );
});

test("the packaged page wires all tested UI helpers before app startup", () => {
  const repositoryRoot = path.resolve(__dirname, "../../../../..");
  const appScript = fs.readFileSync(
    path.join(repositoryRoot, "android/app/src/main/assets/app.js"),
    "utf8",
  );
  const page = fs.readFileSync(
    path.join(repositoryRoot, "android/app/src/main/assets/index.html"),
    "utf8",
  );

  assert.match(appScript, /UiLogic\.syncBusyControls\(document, busy, state\)/);
  assert.match(appScript, /UiLogic\.boardDimensions\(/);
  assert.match(appScript, /render\(UiLogic\.engineThinkingState\(state\)\)/);
  assert.match(appScript, /UiLogic\.setSheetOpen\(document, name, true\)/);
  assert.match(appScript, /UiLogic\.setSheetOpen\(document, name, false\)/);
  assert.match(appScript, /UiLogic\.gameResult\(state\)/);
  assert.match(appScript, /op: "set_difficulty", difficulty/);
  assert.match(appScript, /setDifficultyValue\(state\.params\.difficulty\)/);
  assert.match(page, /id="opt-difficulty"/);
  assert.match(page, /id="difficulty-dialog"/);
  assert.match(page, /class="strength-icon level-5"/);
  assert.match(page, /id="meta-difficulty"/);
  assert.match(appScript, /getElementById\("meta-difficulty"\)/);
  for (const difficulty of ["beginner", "junior", "intermediate", "senior", "master"]) {
    assert.match(page, new RegExp(`data-value="${difficulty}"`));
  }
  assert.ok(page.indexOf('src="ui_logic.js"') < page.indexOf('src="app.js"'));
});
