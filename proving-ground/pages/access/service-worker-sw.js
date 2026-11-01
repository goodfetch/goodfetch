// The service worker of the service worker page (#707). As it installs it
// keeps `cached.json` in its Cache Storage, made here and never fetched,
// and requests, each with its own registration's query, `own.json`, a body
// on `elsewhere.test` and a path EasyPrivacy blocks; then it takes control
// of its pages at once. It answers `cached.json` from its cache and passes
// `network.json` on with its own fetch. A navigation of a page it controls
// it sends on itself when its registration's query asks
// (`navigations=pass`: the navigation's own request, in `navigate` mode;
// `navigations=refetch`: a `cors` request to its URL, #720), or answers
// from a copy of the page it made in its Cache Storage as it installed
// (`navigations=cache`, #760), sending nothing. Every other request goes on
// as if no worker were there. Asked by its page, when its registration's
// query holds `held`, it requests `/service-worker-held.json`, which the
// Proving ground holds in flight far past `networkidle`'s 500 ms of quiet,
// so a page reaches `networkidle` meanwhile only if a worker's own
// requests are left out (#707).
"use strict";

const query = self.location.search;
const navigations = new URLSearchParams(query).get("navigations");
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
      if (navigations === "cache") {
        await cache.put(
          `./${query}`,
          new Response("<!doctype html><title>cached</title><p>From the worker's cache.</p>", {
            headers: { "content-type": "text/html; charset=utf-8" },
          }),
        );
      }
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

self.addEventListener("message", (event) => {
  if (event.data === "hold" && new URLSearchParams(query).has("held")) {
    event.waitUntil(
      fetch(`/service-worker-held.json${query}`)
        .then((response) => response.text())
        .catch(() => null),
    );
  }
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
  } else if (event.request.mode === "navigate" && navigations === "pass") {
    event.respondWith(fetch(event.request));
  } else if (event.request.mode === "navigate" && navigations === "refetch") {
    event.respondWith(fetch(event.request.url, { mode: "cors", credentials: "include" }));
  } else if (event.request.mode === "navigate" && navigations === "cache") {
    event.respondWith(caches.match(event.request.url));
  }
});
