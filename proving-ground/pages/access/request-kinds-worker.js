// The worker of the request kinds page (#632), dedicated or shared: it
// requests `/request-kinds/reached?kind=<kind>` on the host its query's
// `to` names, on its own port, then tells its page the attempt settled.
"use strict";

const query = new URLSearchParams(self.location.search);
const settled = fetch(
  `http://${query.get("to")}:${self.location.port}/request-kinds/reached?kind=${query.get("kind")}`,
  { mode: "no-cors" },
).catch(() => null);

if ("onconnect" in self) {
  self.addEventListener("connect", (event) => {
    const [port] = event.ports;
    void settled.then(() => port.postMessage("done"));
  });
} else {
  void settled.then(() => self.postMessage("done"));
}
