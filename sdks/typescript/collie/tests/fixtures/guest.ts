// A program that imports `@goodfetch/collie`, connects to the Attached
// engine its environment names, runs the tracer and closes, recording what
// the SDK changed in its process at each stage into the file its argument
// names. It writes nothing to stdout or stderr itself, so anything there is
// the SDK's. Run by Node alone, from the package's own source.
import { writeFileSync } from "node:fs";
import { startAndFinish } from "../programs/tracer.ts";

const out = process.argv[2];
if (out === undefined) {
  throw new TypeError("the guest program takes the file it writes as its argument");
}

const EVENTS = [
  "SIGINT",
  "SIGTERM",
  "SIGHUP",
  "SIGBREAK",
  "exit",
  "beforeExit",
  "uncaughtException",
  "unhandledRejection",
  "warning",
] as const;

/** What a guest may never change: the process's handlers and the globals. */
function state(): { listeners: Record<string, number>; globals: string[] } {
  return {
    listeners: Object.fromEntries(EVENTS.map((event) => [event, process.listenerCount(event)])),
    globals: Object.getOwnPropertyNames(globalThis).toSorted(),
  };
}

const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);
const reads: string[] = [];
process.env = new Proxy(process.env, {
  get(target, key, receiver): unknown {
    if (typeof key === "string") {
      reads.push(key);
    }
    return Reflect.get(target, key, receiver);
  },
});

// Node's module loader reads variables of its own on every import.
await import("./probe.ts");
const loaderReads = new Set(reads);
reads.length = 0;

const beforeImport = state();
const { Collie } = await import("@goodfetch/collie");
const afterImport = state();
const readAtImport = [...new Set(reads)].filter((key) => !loaderReads.has(key));
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect({ env });
const traced = await startAndFinish(collie);
const afterClose = state();

writeFileSync(
  out,
  JSON.stringify({
    beforeImport,
    afterImport,
    afterClose,
    readAtImport,
    lifecycle: traced.report.lifecycle,
  }),
);
