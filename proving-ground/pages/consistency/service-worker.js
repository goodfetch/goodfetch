// The consistency probes' service worker (ADR 0074): once a page asks,
// reads every surface, reports each, and answers on the port the page sent
// what it read.
importScripts("/probe.js");
let probed;
addEventListener("message", (event) => {
  probed ??= consistencyProbe("/service-worker.js");
  event.waitUntil(probed.then((values) => event.ports[0].postMessage(values)));
});
