// A host program that Ctrl+C interrupts while its Run runs (ADRs 0078 and
// 0093). It connects to the Attached engine its environment names, starts
// the Run `nightly`, creates the file its second argument names once the Run
// runs, and waits. On SIGINT or SIGTERM it closes the Run, which stops it
// with a Checkpoint, then the Connection, as the SDK's docs advise; it then
// writes into the file its first argument names the signal it got and what
// its logger heard at warning, and exits 0. It writes nothing to stdout or
// stderr itself, so anything there is the SDK's. Run by Node alone, from the
// package's own source; Python's `programs/collie_interrupted.py` is its
// twin.
import { writeFileSync } from "node:fs";
import { Collie, type Logger } from "@goodfetch/collie";

const [out, running] = process.argv.slice(2);
if (out === undefined || running === undefined) {
  throw new TypeError("the program takes the file it writes and the file it creates");
}

const warned: string[] = [];
const logger: Logger = {
  // It asks for none of the engine's log events: `hello` says `off`, as the script expects.
  level: "off",
  debug: () => {},
  info: () => {},
  warn: (message) => {
    warned.push(message);
  },
  error: (message) => {
    warned.push(message);
  },
};
const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect({ env, logger });
const run = await collie.start({ runId: "nightly", tags: ["shop"] });

const signal = await new Promise<NodeJS.Signals>((resolve) => {
  process.once("SIGINT", resolve);
  process.once("SIGTERM", resolve);
  writeFileSync(running, "");
});
await run.close();
await collie.close();
writeFileSync(out, JSON.stringify({ signal, warned }));
