// The consistency probes' shared worker (ADR 0074): reads every surface once,
// reports each, and sends each page that connects what it read.
importScripts("/probe.js");
const probed = consistencyProbe("/shared-worker.js");
onconnect = ({ ports: [port] }) => {
  probed.then((values) => port.postMessage(values));
};
