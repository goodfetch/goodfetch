// A 100 ms timer, as a page that animates or polls keeps one. Every second
// the page reports how many times the timer fired and how many
// milliseconds passed since it started (`ticks` as "<count> <elapsed>"),
// and whether the document has focus (`has_focus`). A page a browser
// throttles behind another tab fires about once a second; one it does not
// fires about ten times (ADR 0085's background-tab question).
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const page = window.location.pathname;

  const send = (reported, observed) =>
    fetch(reports, {
      method: "POST",
      body: JSON.stringify({ page, check: reported, observed }),
      keepalive: true,
    }).catch(() => {});

  const started = performance.now();
  let ticks = 0;
  setInterval(() => {
    ticks += 1;
  }, 100);
  setInterval(() => {
    const elapsed = Math.round(performance.now() - started);
    void send("ticks", `${ticks} ${elapsed}`);
    void send("has_focus", String(document.hasFocus()));
  }, 1000);
})();
