// A program that connects from its working directory, which the test that
// starts it chooses, so no test moves its own. With `found` as its second
// argument it passes no `project`, so `connect()` reads the project file
// found upward from there, as the CLI finds it; with `none` it passes
// `project: null` and reads none (ADR 0083). It explains
// `timeout.navigation`, starts a Run and finishes it, against a script that
// expects exactly the project it carries, then writes the file its engine
// reports and the Run's lifecycle as a JSON object into the file its first
// argument names. Run by Node alone, from the package's own source.
// Python's `tests/programs/collie_project.py` is its twin.
import { writeFileSync } from "node:fs";
import { Collie } from "@goodfetch/collie";

const [out, chosen] = process.argv.slice(2);
if (out === undefined || (chosen !== "found" && chosen !== "none")) {
  throw new TypeError("the project program takes the file it writes, then `found` or `none`");
}

// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect(chosen === "found" ? {} : { project: null });
await collie.explainSettings(["timeout.navigation"]);
const report = await (await collie.start()).finish();
await collie.close();
writeFileSync(
  out,
  JSON.stringify({ projectFile: collie.engine().projectFile ?? null, lifecycle: report.lifecycle }),
);
