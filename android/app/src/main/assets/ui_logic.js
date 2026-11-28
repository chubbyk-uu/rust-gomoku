"use strict";

(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  root.GomokuUiLogic = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  function engineThinkingState(state, tr = zh => zh) {
    return {
      ...state,
      can_play: false,
      engine_thinking: true,
      status: tr("引擎思考中...", "The engine is thinking…"),
      error: null,
    };
  }

  function boardDimensions(
    areaWidth,
    areaHeight,
    viewportWidth,
    viewportHeight,
    padding,
  ) {
    const landscape = viewportWidth > viewportHeight;
    const availableHeight = landscape ? areaHeight : areaWidth;
    const outerSize = Math.max(
      0,
      Math.floor(Math.min(areaWidth, availableHeight)),
    );
    return {
      outerSize,
      canvasSize: Math.max(0, outerSize - padding),
    };
  }

  function syncBusyControls(document_, busy, state) {
    const engineThinking = Boolean(state && state.engine_thinking);
    const moveCount = state ? state.move_count : 0;
    document_.getElementById("btn-undo").disabled =
      busy || engineThinking || moveCount === 0;
    document_.getElementById("btn-newgame").disabled = busy || engineThinking;
    document_.getElementById("btn-start").disabled = busy || engineThinking;
    for (const button of document_.querySelectorAll(
      "#opt-gamemode button, #opt-side button, #opt-rule button, #opt-clockmode button, #opt-difficulty button, #opt-timelimit button",
    )) {
      button.disabled = busy || engineThinking;
    }
    for (const id of ["more-difficulty", "more-timelimit", "opt-multithread"]) {
      const el = document_.getElementById(id);
      if (el) el.disabled = busy || engineThinking;
    }
  }

  function setSheetOpen(document_, name, open) {
    const sheet = document_.getElementById("sheet-" + name);
    const backdrop = document_.querySelector(
      `.sheet-backdrop[data-backdrop="${name}"]`,
    );
    sheet.classList.toggle("open", open);
    sheet.setAttribute("aria-hidden", String(!open));
    backdrop.classList.toggle("open", open);
    backdrop.setAttribute("aria-hidden", String(!open));
  }

  function gameResult(state, tr = zh => zh) {
    if (!state || (state.winner === 0 && !state.draw)) return null;
    if (state.draw) {
      return {
        key: `draw:${state.move_count}`,
        title: tr("和棋", "Draw"),
        message: tr(`棋盘在第 ${state.move_count} 手填满，双方均未连成五子。`, `The board filled on move ${state.move_count} without a winner.`),
        tone: "draw",
      };
    }
    if (state.params && state.params.mode === "two_player") {
      // BLACK is 1, WHITE is -1; announce the winning side, no human/engine.
      const winnerName = state.winner === 1 ? tr("黑方", "Black") : tr("白方", "White");
      return {
        key: `${state.winner}:${state.move_count}`,
        title: tr(`${winnerName}胜`, `${winnerName} wins`),
        message: tr(`${winnerName}在第 ${state.move_count} 手取胜。`, `${winnerName} won on move ${state.move_count}.`),
        tone: "win",
      };
    }
    const humanWon = state.winner === state.human_side;
    return {
      key: `${state.winner}:${state.move_count}`,
      title: humanWon ? tr("你赢了", "You win") : tr("引擎获胜", "Engine wins"),
      message: humanWon
        ? tr(`第 ${state.move_count} 手取胜。`, `You won on move ${state.move_count}.`)
        : tr(`对局在第 ${state.move_count} 手结束。`, `The game ended on move ${state.move_count}.`),
      tone: humanWon ? "win" : "loss",
    };
  }

  return {
    boardDimensions,
    engineThinkingState,
    gameResult,
    setSheetOpen,
    syncBusyControls,
  };
});
