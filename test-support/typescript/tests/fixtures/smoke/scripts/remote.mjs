// cases: credits-released
// remote daemon: connect/remote-drop-resends-a-read.json remote-daemon-token
// A remote-Daemon smoke script that exits 0 though its Daemon fails: it runs
// in the `daemon` mode only, with `GOODFETCH_URL` and `GOODFETCH_TOKEN`
// naming the Scripted Daemon, and closes a bare TCP connection to it before
// any handshake, so the Daemon never plays its script.
import assert from "node:assert/strict";
import { connect } from "node:net";

assert.equal(process.env.SMOKE_MODE, "daemon");
assert.equal(process.env.GOODFETCH_TOKEN, "remote-daemon-token");
const url = new URL(process.env.GOODFETCH_URL);
assert.equal(url.protocol, "ws:");
await new Promise((resolve, reject) => {
  const socket = connect(Number(url.port), url.hostname, () => socket.end(resolve));
  socket.on("error", reject);
});
