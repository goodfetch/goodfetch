// `openRun(runId)`, `RunView`, Key-value store `keys()` & schema-typed values,
// and `Checkpoint` / `RunTrace` `saveAs(path)` + `stream()` (ADRs 0082, 0102,
// 0103 and 0116; issue 1284), played on the Scripted engine attached and as a
// Daemon.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import {
  Collie,
  ProgramError,
  RunInterruptedError,
  RunNotFoundError,
  UnavailableError,
  type Connection,
  type GoodfetchError,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  goodfetchFailure,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";

const BookSchema = z.object({
  title: z.string(),
  pages: z.number().int(),
});

/** Type-level proof: a function typed with `Connection` calls `openRun` on either Product. */
async function openFromConnection(connection: Connection, runId: string): Promise<string> {
  const view = await connection.openRun(runId);
  const id = view.id();
  await view.close();
  return id;
}
void openFromConnection;

interface Played {
  readonly liveKeys: readonly string[];
  readonly checkpointSavedBytes: Uint8Array;
  readonly checkpointStreamedBytes: Uint8Array;
  readonly traceSavedBytes: Uint8Array;
  readonly traceStreamedBytes: Uint8Array;
  readonly finishedProgressLast: boolean;
  readonly finishedReportId: string;
  readonly finishedTraceLocation: string | null;
  readonly finishedBook: { readonly title: string; readonly pages: number } | null;
  readonly finishedBytes: { readonly content: Uint8Array; readonly contentType: string } | null;
  readonly finishedKeys: readonly string[];
  readonly interruptedLifecycle: string;
  readonly interruptedReportError: GoodfetchError;
  readonly missingReportError: GoodfetchError;
}

async function collectBytes(stream: AsyncIterable<Uint8Array>): Promise<Uint8Array> {
  const parts: Uint8Array[] = [];
  for await (const part of stream) {
    parts.push(part);
  }
  const total = parts.reduce((sum, part) => sum + part.byteLength, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

async function collectStrings(stream: AsyncIterable<string>): Promise<string[]> {
  const items: string[] = [];
  for await (const item of stream) {
    items.push(item);
  }
  return items;
}

async function played(open: () => Promise<Collie>, dir: string): Promise<Played> {
  const collie = await open();
  const run = await collie.start();

  await run.keyValueStore.set("item-a", { title: "Dune", pages: 412 }, BookSchema);
  await run.keyValueStore.setBytes(
    "item-bin",
    new TextEncoder().encode("hello"),
    "application/octet-stream",
  );
  const liveKeys = await collectStrings(run.keyValueStore.keys({ prefix: "item-" }));

  const checkpoint = await run.checkpoint();
  const cpPath = join(dir, "nested", "cp.sqlite");
  await checkpoint.saveAs(cpPath);
  const checkpointSavedBytes = new Uint8Array(await readFile(cpPath));
  const checkpointStreamedBytes = await collectBytes(checkpoint.stream());

  const trace = await run.trace();
  if (trace === null) {
    throw new Error("expected run.trace() to return a RunTrace");
  }
  const tracePath = join(dir, "nested", "trace.zip");
  await trace.saveAs(tracePath);
  const traceSavedBytes = new Uint8Array(await readFile(tracePath));
  const traceStreamedBytes = await collectBytes(trace.stream());

  await run.finish();

  const finishedView = await collie.openRun("finished-run");
  const finishedProgress: { last: boolean }[] = [];
  for await (const p of finishedView.progress()) {
    finishedProgress.push(p);
  }
  const finishedReport = await finishedView.report();
  const finishedTrace = await finishedView.trace();
  const finishedBook = await finishedView.keyValueStore.get("item-a", BookSchema);
  const finishedBytes = await finishedView.keyValueStore.getBytes("item-bin");
  const finishedKeys = await collectStrings(finishedView.keyValueStore.keys({ prefix: "item-" }));
  await finishedView.close();

  const interruptedView = await collie.openRun("interrupted-run");
  const interruptedProgress: { lifecycle: string }[] = [];
  for await (const p of interruptedView.progress()) {
    interruptedProgress.push(p);
  }
  const interruptedReportError = await goodfetchFailure(interruptedView.report());
  await interruptedView.close();

  const missingView = await collie.openRun("missing-run");
  const missingReportError = await goodfetchFailure(missingView.report());
  await missingView.close();

  await collie.close();
  return {
    liveKeys,
    checkpointSavedBytes,
    checkpointStreamedBytes,
    traceSavedBytes,
    traceStreamedBytes,
    finishedProgressLast: finishedProgress.at(-1)?.last === true,
    finishedReportId: finishedReport.runId,
    finishedTraceLocation: finishedTrace?.location ?? null,
    finishedBook,
    finishedBytes,
    finishedKeys,
    interruptedLifecycle: interruptedProgress.at(-1)?.lifecycle ?? "",
    interruptedReportError,
    missingReportError,
  };
}

async function checkPlayed(clientCase: ClientCaseRecord, result: Played): Promise<void> {
  await clientCase.check("KV keys() yields prefixed keys in order on live Run and RunView", () => {
    expect(result.liveKeys).toEqual(["item-a", "item-bin"]);
    expect(result.finishedKeys).toEqual(["item-a", "item-bin"]);
  });
  await clientCase.check(
    "Checkpoint.saveAs and RunTrace.saveAs write whole and stream() delivers the same bytes",
    () => {
      const expectedCp = new TextEncoder().encode("cp-bytes!!");
      const expectedTrace = new TextEncoder().encode("trace-bytes!");
      expect(result.checkpointSavedBytes).toEqual(expectedCp);
      expect(result.checkpointStreamedBytes).toEqual(expectedCp);
      expect(result.traceSavedBytes).toEqual(expectedTrace);
      expect(result.traceStreamedBytes).toEqual(expectedTrace);
    },
  );
  await clientCase.check("RunView on finished Run reads progress, report, trace, and KV", () => {
    expect(result.finishedProgressLast).toBe(true);
    expect(result.finishedReportId).toBe("finished-run");
    expect(result.finishedTraceLocation).toBe("/engine/finished-trace.zip");
    expect(result.finishedBook).toEqual({ title: "Dune", pages: 412 });
    expect(result.finishedBytes).toEqual({
      content: new TextEncoder().encode("hello"),
      contentType: "application/octet-stream",
    });
  });
  await clientCase.check(
    "RunView on interrupted Run raises run_interrupted and unknown ID raises run_not_found",
    () => {
      expect(result.interruptedLifecycle).toBe("stopped");
      expect(result.interruptedReportError).toBeInstanceOf(RunInterruptedError);
      expect(result.interruptedReportError).toBeInstanceOf(UnavailableError);
      expect(result.interruptedReportError.code).toBe("run_interrupted");
      expect(result.missingReportError).toBeInstanceOf(RunNotFoundError);
      expect(result.missingReportError).toBeInstanceOf(ProgramError);
      expect(result.missingReportError.code).toBe("run_not_found");
    },
  );
}

const COVERS = [
  "story:11-130",
  "operation:run.trace",
  "operation:checkpoint.save_as",
  "operation:checkpoint.stream",
  "operation:trace.save_as",
  "operation:trace.stream",
  "operation:key_value_store.keys",
] as const;

function scriptWithSavePaths(folder: string, script: string, outDir: string): string {
  const source = new URL(
    `../../../../test-support/fixtures/engine-scripts/${script}`,
    import.meta.url,
  );
  const cpPath = resolve(join(outDir, "nested", "cp.sqlite"));
  const tracePath = resolve(join(outDir, "nested", "trace.zip"));
  const text = readFileSync(source, "utf8")
    .replaceAll(JSON.stringify("/work/out/cp.sqlite"), JSON.stringify(cpPath))
    .replaceAll(JSON.stringify("/work/out/trace.zip"), JSON.stringify(tracePath));
  const file = join(folder, "save-scripts", script);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, text);
  return pathToFileURL(file).href;
}

describe("openRun, RunView, KV keys and Checkpoint/RunTrace copies", () => {
  test(
    "plays run-view-and-copies on an Attached engine",
    { tags: ["client_case/run-view-and-copies", "attached", ...COVERS] },
    async ({ testEnv, clientCase }) => {
      const dir = testEnv.env.GOODFETCH_HOME ?? process.cwd();
      const script = scriptWithSavePaths(testEnv.folder, "run/run-view-and-copies.json", dir);
      const env = scriptedEngineEnv(testEnv, "collie", script);
      const result = await played(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
        () => Collie.connect({ env }),
        dir,
      );
      await checkPlayed(clientCase, result);
    },
  );

  test(
    "plays run-view-and-copies against a Daemon",
    { tags: ["client_case/run-view-and-copies", "daemon", ...COVERS] },
    async ({ testEnv, processes, clientCase }) => {
      const dir = testEnv.env.GOODFETCH_HOME ?? process.cwd();
      const script = scriptWithSavePaths(
        testEnv.folder,
        "run/run-view-and-copies-from-a-daemon.json",
        dir,
      );
      const daemon = await startScriptedDaemon(testEnv, processes, script, "socket");
      const result = await played(
        () =>
          // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
          Collie.connect({ env: testEnv.env, engine: "daemon" }),
        dir,
      );
      const exit = await daemon.exit;
      await checkPlayed(clientCase, result);
      await clientCase.check("the Daemon played its script through", () =>
        expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
