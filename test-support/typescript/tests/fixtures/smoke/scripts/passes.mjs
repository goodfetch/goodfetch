// cases: hello-refused
// A smoke script that passes in both modes: it runs with the mode the runner
// names, and finds the Daemon's runtime file in its home only in the
// `daemon` mode.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { join } from "node:path";

const mode = process.env.GOODFETCH_SMOKE_MODE;
assert.ok(mode === "attached" || mode === "daemon", `GOODFETCH_SMOKE_MODE is ${mode}`);
assert.equal(existsSync(join(process.env.GOODFETCH_HOME, "0.0", "daemon.json")), mode === "daemon");
