// The probe of the debugging protocol's side effects: what a page sees when
// a tool drives its browser over CDP, such as the Runtime domain reading an
// error's stack to preview a console message, and the globals automation
// libraries leave in the page, among them the marker of Goodfetch's guard
// world, which only a guard not kept in its isolated world would show. It
// reports that it ran, then each side effect it saw; a browser no tool
// drives reports none.
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const check = script.getAttribute("data-check");
  const page = window.location.pathname;
  const signals = [];

  const send = (reported, observed) =>
    fetch(reports, {
      method: "POST",
      body: JSON.stringify({ page, check: reported, observed }),
      keepalive: true,
    }).catch(() => {});
  const test = (signal, found) => {
    try {
      if (found()) {
        signals.push(signal);
      }
    } catch {
      signals.push(`${signal}_threw`);
    }
  };

  // With the Runtime domain enabled, the browser builds a preview of every
  // console message, reading the stack of an error it logs.
  let stackRead = false;
  const probe = new Error("probe");
  Object.defineProperty(probe, "stack", {
    configurable: true,
    get: () => {
      stackRead = true;
      return "";
    },
  });
  console.debug(probe);

  test("automation_bindings", () => {
    const names = [
      "__playwright__binding__",
      "__pwInitScripts",
      "__playwright_evaluation_script__",
      "__puppeteer_evaluation_script__",
      "__webdriverFunc",
      "__goodfetch_guard__",
    ];
    return (
      names.some((name) => name in window) ||
      Object.keys(window).some((key) => /^cdc_[a-zA-Z0-9]{22}_/.test(key))
    );
  });
  test("patched_console", () =>
    !/\[native code\]/.test(Function.prototype.toString.call(console["debug"])));
  test("webdriver", () => navigator.webdriver);

  const run = async () => {
    // A preview is built after the message is logged.
    await new Promise((resolve) => {
      window.setTimeout(resolve, 0);
    });
    if (stackRead) {
      signals.push("runtime_enable");
    }
    await send("ran", "content");
    await Promise.all(signals.map((signal) => send(check, signal)));
  };
  void run();
})();
