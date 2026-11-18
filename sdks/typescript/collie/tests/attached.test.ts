// Collie's Connection on an Attached engine of the executable of the same
// commit (ADR 0069), which `env` points at with `GOODFETCH_COLLIE_ENGINE_PATH`
// and the test's own Goodfetch home. The tracer program is the one the unit
// tests run on the Scripted engine.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { once } from "node:events";
import { Worker } from "node:worker_threads";
import { Collie, type Logger } from "@goodfetch/collie";
import { scriptedEngineEnv, test, type Exit, type ProcessRecord } from "@goodfetch/test-support";
import { Schema } from "@goodfetch/test-support/protocol-contract";
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
    "stops a Run its program closes unfinished, so a start under its Run ID resumes it",
    { tags: ["story:11-17"] },
    async ({ env }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const run = await collie.start({ runId: "closed-unfinished" });
      await run.close();

      const again = await collie.start({ runId: "closed-unfinished" });
      const report = await again.finish();
      await collie.close();

      expect(again.startedAs()).toBe("resumed");
      expect(report).toMatchObject({ lifecycle: "finished" });
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
  test(
    "reaches the engine through the members generated from the catalog",
    { tags: ["client_case/catalog-mapping", "attached", "story:11-178"] },
    async ({ env, clientCase }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();

      const set = await run.setProgressMessage("three pages left");
      const read = await run.progress();
      const finished = await run.finish();
      const report = await run.report();

      await clientCase.check("a member with a positional parameter reaches its operation", () =>
        expect(set).toMatchObject({ run_id: run.id(), lifecycle: "running" }),
      );
      await clientCase.check("a member with only options answers what the engine holds", () =>
        expect(read.message).toEqual(set.message),
      );
      await clientCase.check("a member that ends the Run answers its report", () =>
        expect(finished).toMatchObject({ run_id: run.id(), lifecycle: "finished" }),
      );
      await clientCase.check("the engine holds what the members did", () =>
        expect(report).toMatchObject({ run_id: run.id(), lifecycle: "finished" }),
      );
    },
  );
});

describe("a Collie Connection on the Scripted engine as its Attached engine", () => {
  test(
    "reports the engine it reached, of either Product",
    { tags: ["client_case/collie-accepted-by-foxhound", "attached"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/accepted-by-foxhound.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const engine = collie.engine();
      await collie.close();

      await clientCase.check("the engine names its Product and Product version", () =>
        expect(engine).toEqual({
          kind: "attached",
          product: "foxhound",
          version: "0.0.0",
          protocolVersion: "0.1",
          enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
        }),
      );
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "closes a Run still running by aborting it, and refuses every later call with closed",
    { tags: ["client_case/closed-after-close", "attached", "error:closed"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/closed-while-running.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start({ runId: "nightly", tags: ["shop"] });

      await run.close();
      await run.close();
      await collie.close();
      await collie.close();

      await clientCase.check("the Run's finish fails with closed", async () =>
        expect(await failure(run.finish())).toMatchObject({ code: "closed", group: "program" }),
      );
      await clientCase.check("a start fails with closed", async () =>
        expect(await failure(collie.start())).toMatchObject({ code: "closed", group: "program" }),
      );
      await clientCase.check("the close aborted the Run once, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "runs beside another Connection in the same process",
    { tags: ["client_case/two-connections-at-once", "attached"] },
    async ({ testEnv, clientCase }) => {
      const envs = ["first", "second"].map((name) =>
        scriptedEngineEnv(
          testEnv,
          "collie",
          "run/started-and-finished.json",
          join(testEnv.folder, name),
        ),
      );
      const { logger, warned } = recordingLogger();
      const [first, second] = await Promise.all(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        envs.map((env) => Collie.connect({ env, logger })),
      );

      const traced = await Promise.all([
        startAndFinish(first ?? unreachable()),
        startAndFinish(second ?? unreachable()),
      ]);

      await clientCase.check("each Connection's Run finished", () =>
        expect(traced.map(({ report }) => report.lifecycle)).toEqual(["finished", "finished"]),
      );
      await clientCase.check("each script played to its end, so no engine wrote a failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "connects again after a close",
    { tags: ["client_case/connect-after-close", "attached"] },
    async ({ testEnv, clientCase }) => {
      const { logger, warned } = recordingLogger();
      const firstEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "connect/accepted-by-foxhound.json",
        join(testEnv.folder, "first"),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await (await Collie.connect({ env: firstEnv, logger })).close();

      const secondEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "run/started-and-finished.json",
        join(testEnv.folder, "second"),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const traced = await startAndFinish(await Collie.connect({ env: secondEnv, logger }));

      await clientCase.check("the second Connection's Run finished", () =>
        expect(traced.report.lifecycle).toBe("finished"),
      );
      await clientCase.check("both scripts played to their end, so no engine wrote a failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "fails the call in flight with the engine's exit code when the engine exits mid-call",
    { tags: ["error:engine_exited"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/exit-during-a-call.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });

      const error = await failure(collie.start());
      await collie.close();

      expect(error).toMatchObject({
        code: "engine_exited",
        group: "unavailable",
        details: { exit_code: 101, signal: null, stderr_tail: "" },
      });
    },
  );

  test(
    "stops the engine once it writes a line that is not JSON-RPC, failing the call in flight",
    { tags: ["error:engine_output_invalid"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(
        testEnv,
        "collie",
        "attached-engine/garbage-during-a-start.json",
      );
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const error = await failure(collie.start());
      await collie.close();

      expect(error).toMatchObject({
        code: "engine_output_invalid",
        group: "internal",
        details: { line: "INFO engine started" },
      });
      // An engine left running would fail its play on stderr when the close ends its input.
      expect(warned).toEqual([]);
    },
  );
});

describe("an Attached engine that exits before its hello is answered", () => {
  test(
    "fails connect() with engine_exited, keeping the last 64 KiB of its stderr and logging it",
    { tags: ["error:engine_exited"] },
    async () => {
      const { logger, warned } = recordingLogger();
      const env = {
        GOODFETCH_COLLIE_ENGINE_PATH: join(import.meta.dirname, "fixtures", "noisy-engine.ts"),
        PATH: dirname(process.execPath),
      };

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, logger }));

      expect(error).toMatchObject({
        code: "engine_exited",
        details: { exit_code: 3, signal: null, stderr_tail: "é".repeat(32_768) },
      });
      expect(warned).toEqual(["é".repeat(40_000)]);
    },
  );
});

describe("connect() aborted while its Attached engine has not answered hello", () => {
  test("fails with the signal's reason and leaves no engine process", async ({ testEnv }) => {
    const pidFile = join(testEnv.folder, "silent-engine.pid");
    const env = {
      GOODFETCH_COLLIE_ENGINE_PATH: join(import.meta.dirname, "fixtures", "silent-engine.ts"),
      PATH: dirname(process.execPath),
      SILENT_ENGINE_PID_FILE: pidFile,
    };
    const aborting = new AbortController();
    const reason = new Error("the program gave up");

    // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
    const connecting = failure(Collie.connect({ env, signal: aborting.signal }));
    await expect.poll(() => existsSync(pidFile)).toBe(true);
    aborting.abort(reason);
    const error = await connecting;

    const pid = Number(readFileSync(pidFile, "utf8"));
    expect(error).toBe(reason);
    expect(isRunning(pid)).toBe(false);
  });
});

describe("the goodfetch-collie bin", () => {
  test(
    "runs Collie's executable with its arguments and exit code",
    { tags: ["story:11-2"] },
    async ({ env, processes }) => {
      const { exit, stdout } = await runBin(processes, ["--version"], env);

      expect({ exit, stdout }).toEqual({
        exit: { code: 0, signal: null },
        stdout: "goodfetch-collie 0.0.0\n",
      });
    },
  );

  test(
    "exits 8, unavailable's row, with the failure and its fix on stderr when no executable is installed",
    { tags: ["story:11-205"] },
    async ({ env, processes }) => {
      const { GOODFETCH_COLLIE_ENGINE_PATH: _named, ...unset } = env;
      const platformPackage = `@goodfetch/collie-${process.platform}-${process.arch}`;

      const ran = await runBin(processes, ["--version"], unset);

      expect(ran).toEqual({
        exit: { code: 8, signal: null },
        stdout: "",
        stderr: [
          `Collie's engine is not installed: no ${platformPackage}`,
          "code: engine_not_found",
          "retryable: false",
          `fix: Install @goodfetch/collie with its optional dependencies, which hold its engine in ${platformPackage}, or set GOODFETCH_COLLIE_ENGINE_PATH to Collie's executable.`,
          "",
        ].join("\n"),
      });
    },
  );

  test(
    "prints the failure as one JSON value on stdout under --json when the named executable is missing",
    { tags: ["story:11-204", "story:11-205"] },
    async ({ env, testEnv, processes }) => {
      const missing = join(testEnv.folder, "no-such-goodfetch-collie");

      const ran = await runBin(processes, ["run", "finish", "missing", "--json"], {
        ...env,
        GOODFETCH_COLLIE_ENGINE_PATH: missing,
      });

      expect({ ...ran, stdout: JSON.parse(ran.stdout) as unknown }).toEqual({
        exit: { code: 8, signal: null },
        stdout: {
          error: {
            code: "engine_not_found",
            group: "unavailable",
            retryable: false,
            message: `spawn ${missing} ENOENT`,
            fix: `GOODFETCH_COLLIE_ENGINE_PATH names ${missing}, which is no executable file: point it at Collie's executable, goodfetch-collie, or leave it unset to use the one @goodfetch/collie installed.`,
          },
        },
        stderr: "",
      });
      expect(ran.stdout.endsWith("}\n")).toBe(true);
      expect(new Schema({ $ref: "#/$defs/CommandFailed" }).problems(JSON.parse(ran.stdout))).toBe(
        undefined,
      );
    },
  );
});

/** Whether a process of this ID exists. */
function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Runs the goodfetch-collie bin with `args` and `env`, keeping what it wrote. */
async function runBin(
  processes: ProcessRecord,
  args: readonly string[],
  env: Readonly<Record<string, string>>,
): Promise<{ readonly exit: Exit; readonly stdout: string; readonly stderr: string }> {
  const bin = processes.spawn(
    process.execPath,
    [join(import.meta.dirname, "..", "src", "bin.ts"), ...args],
    { env },
  );
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  bin.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
  bin.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
  bin.child.stdin.end();
  const exit = await bin.exit;
  return {
    exit,
    stdout: Buffer.concat(stdout).toString(),
    stderr: Buffer.concat(stderr).toString(),
  };
}

describe("a worker_threads worker", () => {
  test(
    "opens its own Connection to an Attached engine",
    { tags: ["story:11-53"] },
    async ({ env }) => {
      const worker = new Worker(join(import.meta.dirname, "fixtures", "worker.ts"), {
        workerData: env,
      });
      // The messages are collected from the start and the exit alone is
      // awaited: when the worker has posted and exited by the time this
      // thread looks, Node delivers the message and emits `exit` in one
      // turn, so an `exit` listener added once the message arrived would
      // wait forever. A worker that throws fails the wait with its error.
      const messages: unknown[] = [];
      worker.on("message", (message: unknown) => messages.push(message));

      const [exitCode] = await once(worker, "exit");

      expect({ messages, exitCode }).toEqual({ messages: ["finished"], exitCode: 0 });
    },
  );
});

/** A host logger that keeps each warning, where the engine's stderr lines go. */
function recordingLogger(): { readonly logger: Logger; readonly warned: string[] } {
  const warned: string[] = [];
  const logger: Logger = {
    debug: () => {},
    info: () => {},
    warn: (message) => {
      warned.push(message);
    },
    error: () => {},
  };
  return { logger, warned };
}

function unreachable(): never {
  throw new TypeError("invariant: each environment map opened one Connection");
}

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}
