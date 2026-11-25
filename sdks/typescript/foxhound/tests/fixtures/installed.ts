// A program that uses Foxhound as a package manager installed it: it
// connects with no engine-path variable, starts a Run, finishes it, then
// closes, and writes the engine it reached and the Run's lifecycle as a JSON
// object into the file its argument names. The test copies it into the
// project it installs. Run by Node alone, from the package's own source.
// Python's `tests/programs/foxhound_installed.py` is its twin.
import { writeFileSync } from "node:fs";
import { Foxhound } from "@goodfetch/foxhound";

const out = process.argv[2];
if (out === undefined) {
  throw new TypeError("the installed program takes the file it writes as its argument");
}

// oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
const foxhound = await Foxhound.connect();
const { kind, product, enginePath } = foxhound.engine();
const run = await foxhound.start();
const { lifecycle } = await run.finish();
await foxhound.close();
writeFileSync(out, JSON.stringify({ kind, product, enginePath, lifecycle }));
