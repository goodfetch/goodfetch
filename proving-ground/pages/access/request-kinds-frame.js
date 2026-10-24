// The frame of the request kinds page (#632), on another site than its
// page, so the browser runs it out of process: it requests
// `/request-kinds/reached?kind=frame` on the host its query's `to` names,
// on its own port, then tells its page the attempt settled.
(() => {
  "use strict";
  const query = new URLSearchParams(window.location.search);
  void fetch(`http://${query.get("to")}:${window.location.port}/request-kinds/reached?kind=frame`, {
    mode: "no-cors",
  })
    .catch(() => null)
    .then(() => window.parent.postMessage("done", "*"));
})();
