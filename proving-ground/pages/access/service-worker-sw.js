// The service worker of the service worker page (#707). As it installs it
// keeps `cached.json` in its Cache Storage, made here and never fetched,
// and requests, each with its own registration's query, `own.json`, a body
// on `elsewhere.test` and a path EasyPrivacy blocks; then it takes control
// of its pages at once. It answers `cached.json` from its cache and passes
// `network.json` on with its own fetch; every other request goes on as if
// no worker were there.
"use strict";

const query = self.location.search;
const CACHE = "service-worker-page";

self.addEventListener("install", (event) => {
  const own = [
    `own.json${query}`,
    `http://elsewhere.test:${self.location.port}/service-worker/forbidden.json${query}`,
    `js/adobe-analytics.js${query}`,
  ];
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      await cache.put(
        `cached.json${query}`,
        new Response(JSON.stringify({ cached: true }), {
          headers: { "content-type": "application/json" },
        }),
      );
      await Promise.all(
        own.map((url) =>
          fetch(url, { mode: "no-cors" })
            .then((response) => response.text())
            .catch(() => null),
        ),
      );
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event) => {
  const path = new URL(event.request.url).pathname;
  if (path === "/service-worker/cached.json") {
    event.respondWith(caches.match(event.request));
  } else if (path === "/service-worker/network.json") {
    event.respondWith(fetch(event.request));
  }
});
