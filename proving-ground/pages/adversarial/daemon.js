// The public page that calls the local Daemon from its own origin
// (ADR 0067): it tries to read an answer from the Daemon's HTTP listener,
// to make a JSON-RPC call and to open its WebSocket and the Console's, and
// reports to the Proving ground each call that got through. The Daemon and
// its Console must refuse a foreign Origin, so a run of this page reports
// nothing.
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const daemon = script.getAttribute("data-daemon");
  if (!daemon) {
    return;
  }
  const report = (call) => {
    const body = JSON.stringify({ page: "/daemon", check: "daemon", observed: call });
    if (!navigator.sendBeacon(reports, body)) {
      fetch(reports, { method: "POST", body, mode: "no-cors", keepalive: true }).catch(() => {});
    }
  };
  const read = async (call, init) => {
    try {
      const response = await fetch(`http://${daemon}/`, { credentials: "include", ...init });
      await response.text();
      report(call);
    } catch {
      // Refused, as it must be.
    }
  };
  void read("http_read", {});
  void read("rpc_read", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "runs.list", params: {} }),
  });
  // The Daemon's WebSocket, and the Console's, which opens at `/protocol`.
  for (const [call, path] of [
    ["websocket_open", "/"],
    ["console_websocket_open", "/protocol"],
  ]) {
    try {
      const socket = new WebSocket(`ws://${daemon}${path}`);
      socket.addEventListener("open", () => {
        report(call);
        socket.close();
      });
    } catch {
      // Refused, as it must be.
    }
  }
})();
