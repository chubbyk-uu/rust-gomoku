"use strict";

function localizeUi(systemTag) {
  const storageKey = "gomoku.uiLanguage";
  const selector = document.getElementById("ui-language");
  const preference = localStorage.getItem(storageKey);
  selector.value = preference === "en" || preference === "zh-CN" ? preference : "system";
  const tag = selector.value === "system" ? systemTag : selector.value;
  const isEnglish = tag !== "zh-CN";
  document.documentElement.lang = tag;
  if (isEnglish) {
    for (const node of document.querySelectorAll("[data-en]")) {
      node.textContent = node.dataset.en;
    }
    for (const node of document.querySelectorAll("[data-aria-en]")) {
      node.setAttribute("aria-label", node.dataset.ariaEn);
    }
  }

  selector.addEventListener("change", () => {
    localStorage.setItem(storageKey, selector.value);
    if ((selector.value === "system" ? systemTag : selector.value) !== tag) location.reload();
  });
  return { tag, text: (zh, en) => isEnglish ? en : zh };
}
