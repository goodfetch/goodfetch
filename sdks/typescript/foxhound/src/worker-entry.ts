// Entry point for forked TypeScript Crawl worker processes (ADR 0024).
// Phase 1 (`init`): imports the user's Crawl module and connects to the
// Attached engine's private endpoint or Daemon before `crawl.start`.
// Phase 2 (`serve`): joins `runId` via `crawl.serve` and serves
// `crawl_request.handle` requests until the Run ends or `stop` arrives.
import {
  fromWire,
  openWorkerConnection,
  type ClientConnection,
} from "@goodfetch/collie-client/runtime";
import { Crawl } from "./crawl.ts";
import { CrawlRun, handleRequest, type CrawlHost } from "./crawl-run.ts";
import { type CrawlRequestHandleParams, type FoxhoundOperationTypes } from "./generated/index.ts";
import { foxhoundWire, type FoxhoundWireType } from "./generated/wire.ts";
import { foxhound } from "./product.ts";
import type { ParentToWorkerMessage, WorkerToParentMessage } from "./workers.ts";

function sendToParent(message: WorkerToParentMessage): void {
  if (typeof process.send === "function" && process.connected) {
    process.send(message);
  }
}

function isParentMessage(value: unknown): value is ParentToWorkerMessage {
  if (typeof value !== "object" || value === null || !("type" in value)) return false;
  return value.type === "init" || value.type === "serve" || value.type === "stop";
}

let loadedCrawl: Crawl | undefined;
let activeConnection: ClientConnection | undefined;
let activeRun: CrawlRun | undefined;
let stopController: AbortController | undefined;
let shuttingDown = false;

async function shutdown(): Promise<void> {
  if (shuttingDown) return;
  shuttingDown = true;
  stopController?.abort();
  try {
    await activeRun?.close();
  } catch {
    // Closing an already-ended worker run is best-effort.
  }
  try {
    await activeConnection?.close();
  } catch {
    // Closing an already-ended worker connection is best-effort.
  }
  process.exit(0);
}

async function initWorker(
  message: Extract<ParentToWorkerMessage, { readonly type: "init" }>,
): Promise<void> {
  let loaded: unknown;
  try {
    loaded = await import(message.moduleUrl);
  } catch (cause) {
    sendToParent({
      type: "error",
      kind: "not_importable",
      message: `worker failed to import Crawl module ${message.module}: ${String(cause)}`,
      module: message.module,
      exportName: message.exportName,
    });
    process.exit(1);
  }
  const exported =
    typeof loaded === "object" && loaded !== null
      ? Reflect.get(loaded, message.exportName)
      : undefined;
  if (!(exported instanceof Crawl)) {
    sendToParent({
      type: "error",
      kind: "not_importable",
      message: `export "${message.exportName}" of ${message.module} is not a Crawl`,
      module: message.module,
      exportName: message.exportName,
    });
    process.exit(1);
  }
  loadedCrawl = exported;

  try {
    activeConnection = await openWorkerConnection(foxhound, message.target);
  } catch (cause) {
    sendToParent({
      type: "error",
      kind: "other",
      message: `worker failed to connect to engine: ${String(cause)}`,
    });
    process.exit(1);
  }
  sendToParent({ type: "connected", pid: process.pid });
}

async function serveRun(runId: string): Promise<void> {
  const connection = activeConnection;
  const crawl = loadedCrawl;
  if (connection === undefined || crawl === undefined) {
    sendToParent({
      type: "error",
      kind: "other",
      message: "worker received serve before init completed",
    });
    process.exit(1);
  }
  const calls = connection.caller<FoxhoundOperationTypes>(foxhoundWire);
  const crawlHost: CrawlHost = {
    connection,
    calls,
    forget: () => {},
  };
  const run = await connection.startRun<CrawlRun>(
    "crawl.serve",
    () =>
      Promise.resolve({
        runId,
        lifecycle: "running",
        startedAs: "resumed",
        needs: "foxhound",
      }),
    (host, started) => new CrawlRun(host, started, crawl, crawlHost, true),
  );
  activeRun = run;

  connection.serve("crawl_request.handle", async (params, signal) =>
    handleRequest(
      run,
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine requests are not validated beyond `hello`'s protocol version (STYLE.md).
      fromWire(
        params,
        { type: "CrawlRequestHandleParams" satisfies FoxhoundWireType },
        foxhoundWire.types,
      ) as CrawlRequestHandleParams,
      signal,
    ),
  );

  stopController = new AbortController();
  const serving = calls.call("crawl.serve", { runId }, { signal: stopController.signal });
  sendToParent({ type: "ready", pid: process.pid });

  try {
    await serving;
  } catch {
    // Run ended, cancelled, or engine stopped.
  }
  await shutdown();
}

process.on("message", (raw: unknown) => {
  if (!isParentMessage(raw)) return;
  if (raw.type === "stop") {
    void shutdown();
    return;
  }
  if (raw.type === "init") {
    void initWorker(raw);
    return;
  }
  void serveRun(raw.runId);
});

process.on("disconnect", () => {
  void shutdown();
});
