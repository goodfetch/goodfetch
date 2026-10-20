// A `worker_threads` worker that opens its own Collie Connection with the
// environment map its parent hands it, runs the tracer and posts the Run's
// Lifecycle back (ADR 0081).
import { parentPort, workerData } from "node:worker_threads";
import { Collie } from "@goodfetch/collie";
import { startAndFinish } from "../programs/tracer.ts";

function isEnv(value: unknown): value is Readonly<Record<string, string>> {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.values(value).every((entry) => typeof entry === "string")
  );
}

if (parentPort === null || !isEnv(workerData)) {
  throw new TypeError("the worker runs in a worker thread, given its environment map");
}
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const traced = await startAndFinish(await Collie.connect({ env: workerData }));
// oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker's `parentPort` is a MessagePort, which takes no target origin.
parentPort.postMessage(traced.report.lifecycle);
