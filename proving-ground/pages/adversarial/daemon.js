// The public page that calls the local Daemon from its own origin
// (ADR 0067): it tries to read an answer from the Daemon's HTTP listener
// and to open its WebSocket, and reports to the Proving ground each call
// that got through. The Daemon must refuse a foreign Origin, so a run of
// this page reports nothing.
(function () {
  "use strict";
  var script = document.currentScript;
  var reports = script.getAttribute("data-reports");
  var daemon = script.getAttribute("data-daemon");
  if (!daemon) {
    return;
  }
  function report(call) {
    var body = JSON.stringify({ page: "/daemon", check: "daemon", observed: call });
    try {
      navigator.sendBeacon(reports, body);
    } catch (error) {
      fetch(reports, { method: "POST", body: body, mode: "no-cors", keepalive: true });
    }
  }
  fetch("http://" + daemon + "/", { credentials: "include" })
    .then(function (response) {
      return response.text().then(function () {
        report("http_read");
      });
    })
    .catch(function () {
      // Refused, as it must be.
    });
  fetch("http://" + daemon + "/", {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "runs.list", params: {} }),
  })
    .then(function (response) {
      return response.text().then(function () {
        report("rpc_read");
      });
    })
    .catch(function () {
      // Refused, as it must be.
    });
  try {
    var socket = new WebSocket("ws://" + daemon + "/");
    socket.onopen = function () {
      report("websocket_open");
      socket.close();
    };
  } catch (error) {
    // Refused, as it must be.
  }
})();
