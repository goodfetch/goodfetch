// The consistency probes' dedicated worker (ADR 0074): reads every surface,
// reports each, and sends the main frame what it read.
importScripts("/probe.js");
consistencyProbe("/dedicated-worker.js").then((values) => postMessage(values));
