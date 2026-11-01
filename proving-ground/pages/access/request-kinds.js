// The request kinds page (#632): it makes one request of the kind its
// path's last segment names to each host its query's `to` names (#648: `to`
// may repeat), on its own port, at `/request-kinds/reached?kind=<kind>`,
// then titles itself `done` once every attempt settled, so a test knows
// when a refused one would have arrived. The kinds are ADR 0014's: `fetch`
// (a subresource), `navigation` (the page sends itself to the first host
// once it loaded), `frame` (an iframe per host on the host the query's
// `frame` names, another site, which makes it itself), `dedicated_worker`
// and `shared_worker` (a worker per host that makes it), `service_worker`
// (a service worker that makes them all as it installs, at the scope the
// query's `wave` names; the page must be a secure context, such as
// `access.localhost`), `websocket` (a WebSocket per
// host opened there) and `webrtc` (a peer connection whose TURN server,
// over TCP, is the first host on the port the query's `turn` names; with
// the query's `stun`, a `host:port` such as the UDP fixture's, also a STUN
// server and a TURN server over UDP there).
// Titles the page `done`.
const finish = () => {
  document.title = "done";
};

// Calls `finish` once `count` attempts settled.
const settling = (count) => {
  let left = count;
  if (left === 0) {
    finish();
  }
  return () => {
    left -= 1;
    if (left === 0) {
      finish();
    }
  };
};

(() => {
  "use strict";
  const query = new URLSearchParams(window.location.search);
  const targets = query.getAll("to");
  const [to] = targets;
  const port = window.location.port;
  const kind = window.location.pathname.split("/").pop();
  const reached = (host) => `${host}:${port}/request-kinds/reached?kind=${kind}`;
  const worker = (host) => `/request-kinds/worker.js?kind=${kind}&to=${encodeURIComponent(host)}`;
  const attempts = {
    fetch: () => {
      void Promise.all(
        targets.map((host) =>
          fetch(`http://${reached(host)}`, { mode: "no-cors" }).catch(() => null),
        ),
      ).then(finish);
    },
    navigation: () => {
      // Once the page loaded, so no call waits on the navigation; a refused
      // one leaves an error page, which never titles itself `done`.
      window.addEventListener("load", () => {
        window.setTimeout(() => {
          window.location.href = `http://${reached(to)}`;
        }, 100);
      });
    },
    frame: () => {
      const settled = settling(targets.length);
      window.addEventListener("message", settled);
      for (const host of targets) {
        const frame = document.createElement("iframe");
        frame.src = `http://${query.get("frame")}:${port}/request-kinds/frame-document?to=${encodeURIComponent(host)}`;
        document.body.append(frame);
      }
    },
    dedicated_worker: () => {
      const settled = settling(targets.length);
      for (const host of targets) {
        const dedicated = new Worker(worker(host));
        dedicated.addEventListener("message", settled, { once: true });
      }
    },
    shared_worker: () => {
      const settled = settling(targets.length);
      for (const host of targets) {
        const shared = new SharedWorker(worker(host));
        shared.port.addEventListener("message", settled, { once: true });
        shared.port.start();
      }
    },
    service_worker: () => {
      const hosts = targets.map((host) => `to=${encodeURIComponent(host)}`).join("&");
      void navigator.serviceWorker
        // A scope of its own per `wave`, so each page's worker installs
        // anew rather than updating another's.
        .register(`/request-kinds/sw.js?${hosts}`, {
          scope: `/request-kinds/wave-${query.get("wave") ?? 0}/`,
        })
        .then(
          (registration) =>
            new Promise((resolve) => {
              const installing = registration.installing;
              if (!installing) {
                resolve();
                return;
              }
              installing.addEventListener("statechange", () => {
                if (installing.state !== "installing") {
                  resolve();
                }
              });
            }),
        )
        .catch(() => null)
        .then(finish);
    },
    websocket: () => {
      const settled = settling(targets.length);
      for (const host of targets) {
        const socket = new WebSocket(`ws://${reached(host)}`);
        socket.addEventListener("open", () => socket.close());
        socket.addEventListener("close", settled, { once: true });
      }
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
