// The service worker of the request kinds page's `service_worker` kind
// (#648): as it installs, it requests `/request-kinds/reached?kind=service_worker`
// on each host its registration's query names in `to`, on its own port,
// then finishes installing once every attempt settled.
"use strict";

const query = new URLSearchParams(self.location.search);

self.addEventListener("install", (event) => {
  event.waitUntil(
    Promise.all(
      query.getAll("to").map((to) =>
        fetch(`http://${to}:${self.location.port}/request-kinds/reached?kind=service_worker`, {
          mode: "no-cors",
        }).catch(() => null),
      ),
    ).then(() => self.skipWaiting()),
  );
});
