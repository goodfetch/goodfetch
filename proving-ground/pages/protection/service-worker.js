// The service worker of the service-worker Challenge (#1539). It takes
// control of its page at once and answers the page's check with its own
// request, which carries the proof only this script holds. Every other
// request goes on as if no worker were there.
"use strict";

const PROOF = "@PROOF@";

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event) => {
  const { url } = event.request;
  if (new URL(url).pathname === "/service-worker/check") {
    event.respondWith(fetch(url, { method: "POST", headers: { "x-service-worker": PROOF } }));
  }
});
