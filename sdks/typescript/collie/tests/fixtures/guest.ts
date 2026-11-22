// A program that imports `@goodfetch/collie`, connects to the Attached
// engine its environment names, starts a Run, follows its progress to the
// first value, finishes it and closes, recording what
// the SDK changed in its process at each stage into the file its argument
// names, with each process warning and the stage it came at: import, connect,
// or the Run after them. It writes nothing to stdout or stderr itself, so
// anything there is the SDK's. Run by Node alone, from the package's own
// source.
import { writeFileSync } from "node:fs";
import { setImmediate } from "node:timers/promises";

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

/** A process warning the guest heard, and the stage it came at. */
interface Heard {
  readonly stage: Stage;
  readonly name: string;
  readonly code: unknown;
  readonly message: string;
}
type Stage = "import" | "connect" | "run";
let stage: Stage = "import";
const warnings: Heard[] = [];
// The host's channel (ADR 0078), heard before the first stage, so each
// stage's listener count is the same. Node still prints each warning.
process.on("warning", (warning) => {
  warnings.push({
    stage,
    name: warning.name,
    code: "code" in warning ? warning.code : null,
    message: warning.message,
  });
});

// Node's module loader reads variables of its own on every import.
await import("./probe.ts");
const loaderReads = new Set(reads);
reads.length = 0;

const beforeImport = state();
const { Collie } = await import("@goodfetch/collie");
const afterImport = state();
const readAtImport = [...new Set(reads)].filter((key) => !loaderReads.has(key));
stage = "connect";
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect({ env });
// A warning comes on the next tick of the call that emits it.
await setImmediate();
stage = "run";
const run = await collie.start();
// Following the progress prints no status line of its own (ADR 0102).
for await (const _ of run.progress()) break;
const report = await run.finish();
await collie.close();
const afterClose = state();
await setImmediate();

writeFileSync(
  out,
  JSON.stringify({
    beforeImport,
    afterImport,
    afterClose,
    readAtImport,
    pid: process.pid,
    warnings,
    lifecycle: report.lifecycle,
  }),
);
