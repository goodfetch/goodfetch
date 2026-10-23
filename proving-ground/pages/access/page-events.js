// The page events page (#703): what a page does that its events report.
// It writes three console messages, one of each type a program filters
// on, then requests, one after another, a JSON body, a missing page, a
// path a program may abort, the same body on another site and a host that
// does not resolve; then it opens a WebSocket, sends one message and
// closes the socket once a message arrives, and finally throws an
// exception it does not catch. The document's title is `done` once the
// socket closed.
(() => {
  "use strict";
  console.log("page-events", 1, true);
  console.warn("page-events warning");
  console.error("page-events error");

  const requests = [
    ["/page-events/data.json"],
    ["/page-events/missing"],
    ["/page-events/aborted"],
    [`http://elsewhere.test:${location.port}/page-events/data.json`, { mode: "no-cors" }],
    ["http://nowhere.invalid/page-events"],
  ];

  const run = async () => {
    for (const [url, init] of requests) {
      try {
        const response = await fetch(url, init);
        await response.text();
      } catch {
        // A failed request is one of the events the page reports.
      }
    }
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(`${scheme}//${location.host}/page-events/socket`);
    socket.addEventListener("open", () => socket.send("page-events hello"));
    socket.addEventListener("message", () => socket.close(), { once: true });
    socket.addEventListener("close", () => {
      document.title = "done";
      setTimeout(() => {
        throw new TypeError("page-events uncaught");
      }, 0);
    });
  };
  void run();
})();
