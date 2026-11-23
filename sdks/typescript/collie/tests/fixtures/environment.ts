// A program that connects with no `env` and closes, so `connect()` passes
// the process environment (ADR 0083), which the test that starts it gives
// it: no test patches its own. It writes its own environment as a JSON
// object into the file its argument names, for the test to compare with
// the one its engine was started with. Run by Node alone, from the
// package's own source. Python's `tests/programs/collie_environment.py` is
// its twin.
import { writeFileSync } from "node:fs";
import { Collie } from "@goodfetch/collie";

const out = process.argv[2];
if (out === undefined) {
  throw new TypeError("the environment program takes the file it writes as its argument");
}

// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect();
await collie.close();
writeFileSync(out, JSON.stringify(process.env));
