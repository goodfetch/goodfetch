// The request kinds page (#632): it makes one request of the kind its
// path's last segment names to the host its query's `to` names, on its own
// port, at `/request-kinds/reached?kind=<kind>`, then titles itself `done`
// once the attempt settled, so a test knows when a refused one would have
// arrived. The kinds are ADR 0014's: `fetch` (a subresource), `navigation`
// (the page sends itself there once it loaded), `frame` (an iframe on the
// host the query's `frame` names, another site, which makes it itself),
// `dedicated_worker` and `shared_worker` (a worker that makes it),
// `websocket` (a WebSocket opened there) and `webrtc` (a peer connection
// whose TURN server, over TCP, is that host on the port the query's `turn`
// names; with the query's `stun`, a `host:port` such as the UDP fixture's,
// also a STUN server and a TURN server over UDP there).
// Titles the page `done`.
const finish = () => {
  document.title = "done";
};

(() => {
  "use strict";
  const query = new URLSearchParams(window.location.search);
  const to = query.get("to");
  const port = window.location.port;
  const kind = window.location.pathname.split("/").pop();
  const reached = `${to}:${port}/request-kinds/reached?kind=${kind}`;
  const worker = `/request-kinds/worker.js?kind=${kind}&to=${to}`;
  const attempts = {
    fetch: () => {
      void fetch(`http://${reached}`, { mode: "no-cors" })
        .catch(() => null)
        .then(finish);
    },
    navigation: () => {
      // Once the page loaded, so no call waits on the navigation; a refused
      // one leaves an error page, which never titles itself `done`.
      window.addEventListener("load", () => {
        window.setTimeout(() => {
          window.location.href = `http://${reached}`;
        }, 100);
      });
    },
    frame: () => {
      window.addEventListener("message", finish, { once: true });
      const frame = document.createElement("iframe");
      frame.src = `http://${query.get("frame")}:${port}/request-kinds/frame-document?to=${to}`;
      document.body.append(frame);
    },
    dedicated_worker: () => {
      const dedicated = new Worker(worker);
      dedicated.addEventListener("message", finish, { once: true });
    },
    shared_worker: () => {
      const shared = new SharedWorker(worker);
      shared.port.addEventListener("message", finish, { once: true });
      shared.port.start();
    },
    websocket: () => {
      const socket = new WebSocket(`ws://${reached}`);
      socket.addEventListener("open", () => socket.close());
      socket.addEventListener("close", finish, { once: true });
    },
    webrtc: () => {
      const iceServers = [
        {
          urls: `turn:${to}:${query.get("turn")}?transport=tcp`,
          username: "request-kinds",
          credential: "request-kinds",
        },
      ];
      const stun = query.get("stun");
      if (stun) {
        iceServers.push(
          { urls: `stun:${stun}` },
          {
            urls: `turn:${stun}?transport=udp`,
            username: "request-kinds",
            credential: "request-kinds",
          },
        );
      }
      const connection = new RTCPeerConnection({ iceServers });
      connection.addEventListener("icegatheringstatechange", () => {
        if (connection.iceGatheringState === "complete") {
          finish();
        }
      });
      // A TURN server that never answers keeps gathering going.
      window.setTimeout(finish, 5000);
      connection.createDataChannel("request-kinds");
      void connection
        .createOffer()
        .then((offer) => connection.setLocalDescription(offer))
        .catch(finish);
    },
  };
  const attempt = attempts[kind];
  if (attempt) {
    attempt();
  } else {
    finish();
  }
})();
