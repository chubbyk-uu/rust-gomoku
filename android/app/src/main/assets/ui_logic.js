"use strict";

(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  root.GomokuUiLogic = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  function engineThinkingState(state) {
    return {
      ...state,
      can_play: false,
      engine_thinking: true,
      status: "引擎思考中...",
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
      "#opt-side button, #opt-rule button, #opt-mode button",
    )) {
      button.disabled = busy || engineThinking;
    }
    document_.getElementById("opt-difficulty").disabled =
      busy || engineThinking;
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

  function gameResult(state) {
    if (!state || state.winner === 0) return null;
    const humanWon = state.winner === state.human_side;
    return {
      key: `${state.winner}:${state.move_count}`,
      title: humanWon ? "你赢了" : "引擎获胜",
      message: humanWon
        ? `第 ${state.move_count} 手取胜。`
        : `对局在第 ${state.move_count} 手结束。`,
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
