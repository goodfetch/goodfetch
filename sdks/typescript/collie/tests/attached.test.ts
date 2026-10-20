// Collie's Connection on an Attached engine of the executable of the same
// commit (ADR 0069), which `env` points at with `GOODFETCH_COLLIE_ENGINE_PATH`
// and the test's own Goodfetch home. The tracer program is the one the unit
// tests run on the Scripted engine.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { once } from "node:events";
import { Worker } from "node:worker_threads";
import { Collie } from "@goodfetch/collie";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { startAndFinish } from "./programs/tracer.ts";

const RUN_ID = /^[0-9a-z]{26}$/;

/** What the guest program recorded of its process at each stage. */
interface Stages {
  readonly beforeImport: unknown;
  readonly afterImport: unknown;
  readonly afterClose: unknown;
}

/** The stages the guest program's file holds; a `TypeError` when it holds none. */
function stages(value: unknown): Stages {
  if (
    typeof value === "object" &&
    value !== null &&
    "beforeImport" in value &&
    "afterImport" in value &&
    "afterClose" in value
  ) {
    return value;
  }
  throw new TypeError("the guest program recorded no stages");
}

describe("a Collie Connection on an Attached engine", () => {
  test(
    "starts a Run, finishes it and answers its Run report",
    { tags: ["story:11-2"] },
    async ({ env, testEnv }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const engine = collie.engine();

      const traced = await startAndFinish(collie);

      expect(engine).toEqual({
        kind: "attached",
        product: "collie",
        version: "0.0.0",
        protocolVersion: "0.1",
        enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
      });
      expect(traced.startedAs).toBe("new");
      expect(traced.runId).toMatch(RUN_ID);
      expect(traced.report).toMatchObject({
        run_id: traced.runId,
        lifecycle: "finished",
        verdict: "verified",
        completeness: "complete",
      });
      expect(existsSync(join(testEnv.home, "0.0", "runs", traced.runId, "run.sqlite"))).toBe(true);
    },
  );

  test(
    "stops a Run its program closes unfinished, as an abort by its Client",
    { tags: ["story:11-17"] },
    async ({ env }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const run = await collie.start({ runId: "closed-unfinished" });
      await run.close();

      const again = await collie.start({ runId: "closed-unfinished" });
      const report = await again.finish();
      await collie.close();

      expect(again.startedAs()).toBe("existing");
      expect(report).toMatchObject({
        lifecycle: "stopped",
        completeness: "incomplete",
        stop_reason: { kind: "abort", by: "client" },
      });
    },
  );

  test(
    "imports, connects and closes as a guest: nothing written, read or patched",
    { tags: ["client_case/nothing-at-import-or-connect", "attached"] },
    async ({ env, testEnv, processes, clientCase }) => {
      const out = join(testEnv.folder, "guest.json");
      const guest = processes.spawn(
        process.execPath,
        [join(import.meta.dirname, "fixtures", "guest.ts"), out],
        { env },
      );
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      guest.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
      guest.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      guest.child.stdin.end();

      const exit = await guest.exit;

      await clientCase.check("the program ended at once with exit code 0", () =>
        expect({ exit, stderr: Buffer.concat(stderr).toString() }).toEqual({
          exit: { code: 0, signal: null },
          stderr: "",
        }),
      );
      await clientCase.check("the SDK wrote nothing to stdout", () =>
        expect(Buffer.concat(stdout).toString()).toBe(""),
      );
      const seen: unknown = JSON.parse(readFileSync(out, "utf8"));
      clientCase.message(seen);
      await clientCase.check("importing read no variable", () =>
        expect(seen).toMatchObject({ readAtImport: [], lifecycle: "finished" }),
      );
      await clientCase.check("no handler or global was added at import or connect", () => {
        const { beforeImport, afterImport, afterClose } = stages(seen);
        expect([afterImport, afterClose]).toEqual([beforeImport, beforeImport]);
      });
    },
  );
});

describe("the goodfetch-collie bin", () => {
  test(
    "runs Collie's executable with its arguments and exit code",
    { tags: ["story:11-2"] },
    async ({ env, processes }) => {
      const bin = processes.spawn(
        process.execPath,
        [join(import.meta.dirname, "..", "src", "bin.ts"), "--version"],
        { env },
      );
      const stdout: Buffer[] = [];
      bin.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
      bin.child.stdin.end();

      const exit = await bin.exit;

      expect({ exit, stdout: Buffer.concat(stdout).toString() }).toEqual({
        exit: { code: 0, signal: null },
        stdout: "goodfetch-collie 0.0.0\n",
      });
    },
  );
});

describe("a worker_threads worker", () => {
  test(
    "opens its own Connection to an Attached engine",
    { tags: ["story:11-53"] },
    async ({ env }) => {
      const worker = new Worker(join(import.meta.dirname, "fixtures", "worker.ts"), {
        workerData: env,
      });

      const [lifecycle] = await once(worker, "message");
      await once(worker, "exit");

      expect(lifecycle).toBe("finished");
    },
  );
});
