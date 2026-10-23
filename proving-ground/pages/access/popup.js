// The popup page (#719): `#open` opens the popup page in a new window
// with `window.open`, as a page opens a popup, and `#tab` is a link that
// opens it in a new tab with no opener. The popup page then loads in the
// opener's browser context.
(() => {
  "use strict";
  const main = document.querySelector("main");
  const target = "/popup/opened";

  const open = document.createElement("button");
  open.id = "open";
  open.type = "button";
  open.textContent = "Open the popup";
  open.addEventListener("click", () => {
    window.open(target, "_blank");
  });

  const tab = document.createElement("a");
  tab.id = "tab";
  tab.href = target;
  tab.target = "_blank";
  tab.rel = "noopener";
  tab.textContent = "Open it in a new tab";

  main.append(open, tab);
})();
