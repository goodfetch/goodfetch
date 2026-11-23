// cases: dropped-link-resends-reads
// remote daemon: connect/remote-drop-resends-a-read.json remote-daemon-token
// Collie's Client reaches the remote Daemon `GOODFETCH_URL` names over
// WebSocket, which drops the link while an Effect and a read are in flight:
// the read is sent again and answered once the Client reached the Daemon
// again, and the Effect raises `connection_lost`, never sent again.
import assert from "node:assert/strict";
import { Collie, ConnectionLostError } from "@goodfetch/collie";

const collie = await Collie.connect();
const run = await collie.start();
const [lost, listed] = await Promise.allSettled([run.newPage(), collie.call("run.list", {})]);
await run.finish();
await collie.close();

assert.equal(lost.status, "rejected");
assert.ok(lost.reason instanceof ConnectionLostError, String(lost.reason));
assert.equal(lost.reason.code, "connection_lost");
assert.equal(lost.reason.group, "unavailable");
assert.equal(lost.reason.operation, "run.new_page");
assert.deepEqual(listed, { status: "fulfilled", value: { runs: [] } });
