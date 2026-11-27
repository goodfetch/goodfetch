// Multi-process Crawl workers for TypeScript (ADR 0024; spec #611's #1497):
// `workers: { count, module, export? }` validates that `module` exports a
// `Crawl`, forks `count - 1` Node processes running `worker-entry.ts` and
// connects them to the Attached engine's private endpoint or Daemon before
// `crawl.start`, then instructs each to join the started Run via
// `crawl.serve` so all `count` processes share work from the first request,
// and stops every worker cleanly when the Run ends or closes.
import { fork, type ChildProcess } from "node:child_process";
import { isAbsolute, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { SettingInvalidError } from "@goodfetch/collie-client";
import type { WorkerTarget } from "@goodfetch/collie-client/runtime";
import { Crawl } from "./crawl.ts";
import { CrawlNotImportableError } from "./generated/errors.ts";

/**
 * Offers this Connection's handler to a live crawl another Connection started,
 * as each of `workers=N` handler processes does (ADR 0024): the engine leases
 * the crawl's work items to it as `crawl_request.handle` requests too, as many
 * at once as `crawl.handler.concurrency` allows on each Connection. Answers the
 * crawl's Run report once the Run ends; cancelling the call stops new work
 * items, while those already sent still expect their answers. A Run this engine
 * does not run as a crawl fails with `run_not_found`.
 *
 * How `startCrawl` spreads a crawl's handlers across Node processes (ADR 0024).
 *
 * @experimental
 */
export interface CrawlWorkers {
  /** Total processes serving the crawl (1 is this process alone; must be a whole number >= 1). */
  readonly count: number;
  /** Module path or specifier that exports the `Crawl` for worker processes to import. */
  readonly module: string;
  /** Export name on `module` holding the `Crawl`; defaults to `"default"`. */
  readonly export?: string | undefined;
}

/** Validated multi-worker plan ready to spawn before `crawl.start`. */
export interface WorkerPlan {
  readonly count: number;
  readonly module: string;
  readonly moduleUrl: string;
  readonly exportName: string;
}

/** Control messages sent from the starter process to a worker over Node IPC. */
export type ParentToWorkerMessage =
  | {
      readonly type: "init";
      readonly module: string;
      readonly moduleUrl: string;
      readonly exportName: string;
      readonly target: WorkerTarget;
    }
  | { readonly type: "serve"; readonly runId: string }
  | { readonly type: "stop" };

/** Status messages sent from a worker to the starter process over Node IPC. */
export type WorkerToParentMessage =
  | { readonly type: "connected"; readonly pid: number }
  | { readonly type: "ready"; readonly pid: number }
  | {
      readonly type: "error";
      readonly kind: "not_importable" | "other";
      readonly message: string;
      readonly fix?: string | undefined;
      readonly module?: string | undefined;
      readonly exportName?: string | undefined;
    };

/** Running worker processes owned by one `CrawlRun`. */
export interface WorkerPool {
  /** Instructs every pre-connected worker to join `runId` via `crawl.serve`. */
  join(runId: string): Promise<void>;
  /** Tells every worker to stop and waits until every process exits. */
  stop(): Promise<void>;
}

const WORKER_ENTRY_PATH = fileURLToPath(new URL("./worker-entry.ts", import.meta.url));
const WORKER_READY_TIMEOUT_MS = 30_000;
const WORKER_STOP_GRACE_MS = 3_000;
const WORKER_KILL_GRACE_MS = 1_000;

/**
 * Validates `workers` and confirms the named module exports a `Crawl` before
 * `crawl.start` is sent. Returns `undefined` when only 1 process runs.
 */
export async function prepareWorkers(
  workers: number | CrawlWorkers | undefined,
): Promise<WorkerPlan | undefined> {
  if (workers === undefined) return undefined;
  if (typeof workers === "number") {
    checkWorkerCount(workers);
    if (workers === 1) return undefined;
    throw new CrawlNotImportableError(
      `cannot start ${String(workers)} worker processes without a module exporting the Crawl`,
      {
        fix: "Define the Crawl at module level and pass workers: { count, module, export? } naming the module that exports it.",
        details: { module: null, export: null },
      },
    );
  }
  checkWorkerCount(workers.count);
  if (workers.count === 1) return undefined;
  const exportName = workers.export ?? "default";
  if (workers.module.trim() === "") {
    throw notImportable(
      workers.module,
      exportName,
      "workers.module must name a module exporting the Crawl",
    );
  }
  const moduleUrl = toModuleUrl(workers.module);
  let loaded: unknown;
  try {
    loaded = await import(moduleUrl);
  } catch (cause) {
    throw notImportable(
      workers.module,
      exportName,
      `failed to import Crawl module ${workers.module}`,
      cause,
    );
  }
  const exported = readExport(loaded, exportName);
  if (!(exported instanceof Crawl)) {
    throw notImportable(
      workers.module,
      exportName,
      `export "${exportName}" of ${workers.module} is not a Crawl`,
    );
  }
  return {
    count: workers.count,
    module: workers.module,
    moduleUrl,
    exportName,
  };
}

/**
 * Forks `plan.count - 1` Node worker processes, imports the Crawl module, and
 * connects each worker to `target` so they are ready to join immediately after
 * `crawl.start`.
 */
export async function spawnWorkers(plan: WorkerPlan, target: WorkerTarget): Promise<WorkerPool> {
  const children: ChildProcess[] = [];
  const initMessage: ParentToWorkerMessage = {
    type: "init",
    module: plan.module,
    moduleUrl: plan.moduleUrl,
    exportName: plan.exportName,
    target,
  };
  try {
    const connected: Promise<void>[] = [];
    for (let i = 1; i < plan.count; i += 1) {
      const child = fork(WORKER_ENTRY_PATH, [], {
        execArgv: [],
        stdio: ["ignore", "ignore", "pipe", "ipc"],
      });
      children.push(child);
      connected.push(waitForSignal(child, initMessage, "connected"));
    }
    await Promise.all(connected);
  } catch (error) {
    await stopChildren(children);
    throw error;
  }

  let stopping: Promise<void> | undefined;
  return {
    async join(runId: string) {
      const serveMessage: ParentToWorkerMessage = { type: "serve", runId };
      try {
        await Promise.all(children.map((child) => waitForSignal(child, serveMessage, "ready")));
      } catch (error) {
        await stopChildren(children);
        throw error;
      }
    },
    stop() {
      stopping ??= stopChildren(children);
      return stopping;
    },
  };
}

/** Sends `message` to `child` and resolves once `child` replies with `expected`. */
function waitForSignal(
  child: ChildProcess,
  message: ParentToWorkerMessage,
  expected: "connected" | "ready",
): Promise<void> {
  return new Promise<void>((resolveReady, rejectReady) => {
    let settled = false;
    const stderrChunks: Buffer[] = [];
    const onStderr = (chunk: Buffer) => {
      if (stderrChunks.length < 32) stderrChunks.push(chunk);
    };
    child.stderr?.on("data", onStderr);

    const timer = setTimeout(() => {
      finish(
        new Error(
          `worker process did not reach ${expected} within ${String(WORKER_READY_TIMEOUT_MS)} ms${stderrSuffix(stderrChunks)}`,
        ),
      );
    }, WORKER_READY_TIMEOUT_MS);
    timer.unref();

    const cleanup = () => {
      clearTimeout(timer);
      child.stderr?.off("data", onStderr);
      child.off("message", onMessage);
      child.off("exit", onExit);
      child.off("error", onError);
    };

    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      cleanup();
      if (error === undefined) resolveReady();
      else rejectReady(error);
    };

    const onMessage = (raw: unknown) => {
      if (!isWorkerMessage(raw)) return;
      if (raw.type === expected) {
        finish();
        return;
      }
      if (raw.type === "error") {
        if (raw.kind === "not_importable") {
          finish(
            new CrawlNotImportableError(raw.message, {
              fix:
                raw.fix ??
                "Define the Crawl at module level and pass workers: { count, module, export? } naming the module that exports it.",
              details: { module: raw.module ?? null, export: raw.exportName ?? null },
            }),
          );
          return;
        }
        finish(new Error(raw.message));
      }
    };

    const onExit = (code: number | null, signal: NodeJS.Signals | null) => {
      finish(
        new Error(
          `worker process exited before ${expected} (code=${String(code)}, signal=${String(signal)})${stderrSuffix(stderrChunks)}`,
        ),
      );
    };

    const onError = (error: Error) => {
      finish(error);
    };

    child.on("message", onMessage);
    child.on("exit", onExit);
    child.on("error", onError);
    child.send(message, (error) => {
      if (error !== null) finish(error);
    });
  });
}

/** Stops every worker process cleanly and waits until all exit. */
async function stopChildren(children: readonly ChildProcess[]): Promise<void> {
  const stopMessage: ParentToWorkerMessage = { type: "stop" };
  await Promise.all(
    children.map(async (child) => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      const exited = new Promise<void>((resolveExit) => {
        child.once("exit", () => resolveExit());
      });
      if (child.connected) {
        try {
          child.send(stopMessage);
        } catch {
          // Worker already disconnected or exited.
        }
      }
      const graceful = await waitOrTimeout(exited, WORKER_STOP_GRACE_MS);
      if (graceful) return;
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGTERM");
      }
      const terminated = await waitOrTimeout(exited, WORKER_KILL_GRACE_MS);
      if (!terminated && child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await exited;
      }
    }),
  );
}

function waitOrTimeout(promise: Promise<void>, ms: number): Promise<boolean> {
  return new Promise<boolean>((resolveWait) => {
    const timer = setTimeout(() => resolveWait(false), ms);
    timer.unref();
    void promise.then(() => {
      clearTimeout(timer);
      resolveWait(true);
      return undefined;
    });
  });
}

function checkWorkerCount(count: number): void {
  if (!Number.isInteger(count) || count < 1) {
    throw new SettingInvalidError(
      `workers must be a whole number of 1 or more, got ${String(count)}`,
      {
        fix: "Pass 1 or more for workers.",
        details: { key: "workers", reason: "must be a whole number of 1 or more" },
      },
    );
  }
}

function notImportable(
  moduleName: string,
  exportName: string,
  message: string,
  cause?: unknown,
): CrawlNotImportableError {
  return new CrawlNotImportableError(message, {
    fix: "Define the Crawl at module level and pass workers: { count, module, export? } naming the module and export that hold it.",
    details: { module: moduleName, export: exportName },
    cause,
  });
}

function toModuleUrl(specifier: string): string {
  if (specifier.startsWith("file:")) return specifier;
  if (isAbsolute(specifier) || specifier.startsWith(".")) {
    return pathToFileURL(resolve(specifier)).href;
  }
  return specifier;
}

function readExport(loaded: unknown, exportName: string): unknown {
  if (typeof loaded !== "object" || loaded === null) return undefined;
  return Reflect.get(loaded, exportName);
}

function isWorkerMessage(value: unknown): value is WorkerToParentMessage {
  if (typeof value !== "object" || value === null || !("type" in value)) return false;
  return value.type === "connected" || value.type === "ready" || value.type === "error";
}

function stderrSuffix(chunks: readonly Buffer[]): string {
  if (chunks.length === 0) return "";
  const text = Buffer.concat(chunks).toString("utf8").trim();
  return text === "" ? "" : `: ${text}`;
}
