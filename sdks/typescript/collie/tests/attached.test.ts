// Collie's Connection on an Attached engine of the executable of the same
// commit (ADR 0069), which `env` points at with `GOODFETCH_COLLIE_ENGINE_PATH`
// and the test's own Goodfetch home. The tracer program is the one the unit
// tests run on the Scripted engine.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { once } from "node:events";
import { Worker } from "node:worker_threads";
import { Collie, type RunProgress } from "@goodfetch/collie";
import {
  recordingLogger,
  scriptedEngineEnv,
  test,
  type Exit,
  type ProcessRecord,
} from "@goodfetch/test-support";
import { Schema } from "@goodfetch/test-support/protocol-contract";
import { describe, expect } from "vitest";
import { startAndFinish } from "./programs/tracer.ts";

const RUN_ID = /^[0-9a-z]{26}$/;

/** The line the crashing engine's script writes 40 times to its stderr, 81,000 bytes. */
const CRASH_LINE = `thread 'main' panicked: ${"é".repeat(1000)}`;

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

/** A process warning the guest program heard, and the stage it came at. */
interface Heard {
  readonly stage: string;
  readonly name: string;
  readonly code: unknown;
  readonly message: string;
}

function isHeard(value: unknown): value is Heard {
  return (
    typeof value === "object" &&
    value !== null &&
    "stage" in value &&
    typeof value.stage === "string" &&
    "name" in value &&
    typeof value.name === "string" &&
    "code" in value &&
    "message" in value &&
    typeof value.message === "string"
  );
}

/** The guest program's file, with its process ID and the warnings it heard; a `TypeError` without them. */
function guestRecord(
  value: unknown,
): object & { readonly pid: number; readonly warnings: Heard[] } {
  if (typeof value === "object" && value !== null && "pid" in value && "warnings" in value) {
    const { pid, warnings }: { pid: unknown; warnings: unknown } = value;
    if (typeof pid === "number" && Array.isArray(warnings) && warnings.every(isHeard)) {
      return { ...value, pid, warnings };
    }
  }
  throw new TypeError("the guest program recorded no process ID or warnings");
}

/** Node's line on stderr for a warning (its `lib/internal/process/warning.js`). */
function nodeLine(pid: number, warning: Heard): string {
  const code = typeof warning.code === "string" ? `[${warning.code}] ` : "";
  return `(node:${pid}) ${code}${warning.name}: ${warning.message}\n`;
}

/** `stderr` without the one hint Node prints after its first warning. */
function withoutNodeHint(stderr: string): string {
  return stderr.replace(
    /^\(Use `\S+ --trace-(?:warnings|deprecation) \.\.\.` to show where the warning was created\)\n/m,
    "",
  );
}

/** Collie's catalog, whose markers are the documented ones (ADR 0030). */
const CATALOG = join(
  import.meta.dirname,
  "..",
  "..",
  "..",
  "..",
  "crates",
  "goodfetch-protocol",
  "generated",
  "collie.json",
);

/**
 * The run-time warnings Collie's catalog documents (ADR 0030, ADR 0099): an
 * Experimental entry's whole text, and the start of a deprecated entry's,
 * whose own text follows.
 */
interface Markers {
  readonly experimental: ReadonlySet<string>;
  readonly deprecated: readonly string[];
}

function catalogMarkers(): Markers {
  const catalog: unknown = JSON.parse(readFileSync(CATALOG, "utf8"));
  const experimental = new Set<string>();
  const deprecated: string[] = [];
  for (const section of typeof catalog === "object" && catalog !== null
    ? Object.values(catalog)
    : []) {
    const entries: unknown[] = Array.isArray(section) ? section : [];
    for (const entry of entries) {
      if (
        typeof entry !== "object" ||
        entry === null ||
        !("name" in entry) ||
        typeof entry.name !== "string"
      ) {
        continue;
      }
      if ("stability" in entry && entry.stability === "experimental") {
        experimental.add(
          `\`${entry.name}\` is Experimental: outside semver until it is promoted (ADR 0030).`,
        );
      }
      if ("deprecated" in entry && entry.deprecated !== null) {
        deprecated.push(`\`${entry.name}\` is deprecated. `);
      }
    }
  }
  return { experimental, deprecated };
}

/** Whether `warning` is one the catalog documents, sent through Node's channel by the SDK. */
function documented(markers: Markers, warning: Heard): boolean {
  if (warning.code === "GOODFETCH_EXPERIMENTAL") {
    return warning.name === "ExperimentalWarning" && markers.experimental.has(warning.message);
  }
  if (warning.code === "GOODFETCH_DEPRECATED") {
    return (
      warning.name === "DeprecationWarning" &&
      markers.deprecated.some((start) => warning.message.startsWith(start))
    );
  }
  return false;
}

/** Each warning that repeats an earlier one's marker and entry. */
function repeated(warnings: readonly Heard[]): Heard[] {
  return warnings.filter(
    (warning, index) =>
      warnings.findIndex(
        (other) => other.code === warning.code && other.message === warning.message,
      ) !== index,
  );
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
        projectFile: null,
        home: testEnv.home,
        enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
        url: null,
        fallback: null,
      });
      expect(traced.startedAs).toBe("new");
      expect(traced.runId).toMatch(RUN_ID);
      expect(traced.report).toMatchObject({
        runId: traced.runId,
        lifecycle: "finished",
        verdict: "verified",
        completeness: "complete",
      });
      expect(existsSync(join(testEnv.home, "0.0", "runs", traced.runId, "run.sqlite"))).toBe(true);
    },
  );

  test(
    "stops a Run its program closes unfinished, so a start under its Run ID resumes it",
    { tags: ["story:11-17", "story:11-18"] },
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
    "imports, connects and closes as a guest: nothing written, read or patched at import or connect, then only marker warnings",
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
        expect(exit).toEqual({ code: 0, signal: null }),
      );
      await clientCase.check("the SDK wrote nothing to stdout", () =>
        expect(Buffer.concat(stdout).toString()).toBe(""),
      );
      const seen = guestRecord(JSON.parse(readFileSync(out, "utf8")));
      clientCase.message(seen);
      await clientCase.check("the SDK wrote nothing to stderr but the warnings it emitted", () =>
        expect(withoutNodeHint(Buffer.concat(stderr).toString())).toBe(
          seen.warnings.map((warning) => nodeLine(seen.pid, warning)).join(""),
        ),
      );
      await clientCase.check("no warning came at import or connect", () =>
        expect(seen.warnings.filter((warning) => warning.stage !== "run")).toEqual([]),
      );
      const markers = catalogMarkers();
      await clientCase.check("after connect, only documented marker warnings, each once", () => {
        expect(seen.warnings.filter((warning) => !documented(markers, warning))).toEqual([]);
        expect(repeated(seen.warnings)).toEqual([]);
      });
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
      const live: RunProgress[] = [];
      for await (const value of run.progress()) {
        live.push(value);
        break;
      }
      const finished = await run.finish();
      const ended: RunProgress[] = [];
      for await (const value of run.progress()) ended.push(value);
      const report = await run.report();

      await clientCase.check("a member with a positional parameter reaches its operation", () =>
        expect(set).toMatchObject({ runId: run.id(), lifecycle: "running" }),
      );
      await clientCase.check("the Run's progress answers what the engine holds at once", () =>
        expect(live.map((value) => value.message)).toEqual([set.message]),
      );
      await clientCase.check("a member that ends the Run answers its report", () =>
        expect(finished).toMatchObject({ runId: run.id(), lifecycle: "finished" }),
      );
      await clientCase.check("the progress of a Run that ended is its last value alone", () =>
        expect(ended.map(({ lifecycle, last }) => ({ lifecycle, last }))).toEqual([
          { lifecycle: "finished", last: true },
        ]),
      );
      await clientCase.check("the engine holds what the members did", () =>
        expect(report).toMatchObject({ runId: run.id(), lifecycle: "finished" }),
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
          projectFile: null,
          home: testEnv.home,
          enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
          url: null,
          fallback: null,
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
    "sends a call's timeout in its three states",
    {
      tags: ["client_case/call-timeout-three-states", "attached", "story:11-40", "story:11-42"],
    },
    async ({ testEnv, clientCase }) => {
      // The script expects each Run's `run.finish` with no `timeout`, with
      // null and with 2 ms (ADR 0080).
      const env = scriptedEngineEnv(testEnv, "collie", "run/call-timeout-three-states.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const omitted = await (await collie.start()).finish();
      const none = await (await collie.start()).finish({ timeout: null });
      const given = await (await collie.start()).finish({ timeout: 2 });
      await collie.close();

      await clientCase.check("each state of the timeout reached the engine as sent", () =>
        expect([omitted, none, given].map(({ lifecycle }) => lifecycle)).toEqual([
          "finished",
          "finished",
          "finished",
        ]),
      );
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test.for([
    { name: "0", timeout: 0 },
    { name: "a negative", timeout: -1 },
    { name: "an infinite", timeout: Number.POSITIVE_INFINITY },
  ])(
    "refuses a call's timeout of $name before sending it",
    {
      tags: [
        "client_case/call-timeout-three-states",
        "attached",
        "story:11-40",
        "story:11-42",
        "error:setting_invalid",
      ],
    },
    async ({ timeout }, { testEnv, clientCase }) => {
      // The script's one `run.finish` has no `timeout`: the refused one sends nothing.
      const env = scriptedEngineEnv(testEnv, "collie", "run/started-and-finished.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const run = await collie.start();
      const refused = await failure(run.finish({ timeout }));
      await run.finish();
      await collie.close();

      await clientCase.check(
        "a timeout of 0 or less, or not finite, is refused before it is sent, naming its setting and the null form",
        () =>
          expect(refused).toMatchObject({
            code: "setting_invalid",
            fix: expect.stringContaining("`timeout: null`"),
            limit: { key: "timeout.run", value: timeout, layer: "run", origin: "argument" },
          }),
      );
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "fails a connect whose hello is unanswered past its timeout, and waits with none",
    {
      tags: [
        "client_case/connect-timeout-three-states",
        "attached",
        "story:11-41",
        "story:11-42",
        "setting:timeout.connect",
        "error:deadline_exceeded",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const { logger, warned } = recordingLogger();
      // The first engine never answers `hello`; the second, whose script
      // replaces the first's in the home once the first has ended, answers it.
      const silent = scriptedEngineEnv(testEnv, "collie", "connect/hello-never-answered.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const late = await failure(Collie.connect({ env: silent, logger, timeout: 200 }));
      const answering = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const none = await Collie.connect({ env: answering, logger, timeout: null });
      await none.close();

      await clientCase.check(
        "a hello unanswered past the connect's own timeout fails it, naming timeout.connect given as an argument",
        () =>
          expect(late).toMatchObject({
            code: "deadline_exceeded",
            limit: { key: "timeout.connect", value: 200, layer: "run", origin: "argument" },
            details: { waited_for: "hello" },
          }),
      );
      await clientCase.check("both scripts played to their end, so no engine wrote a failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test.for([
    { name: "0", timeout: 0 },
    { name: "a negative", timeout: -1 },
    { name: "an infinite", timeout: Number.POSITIVE_INFINITY },
  ])(
    "refuses a connect timeout of $name before the engine starts",
    {
      tags: [
        "client_case/connect-timeout-three-states",
        "attached",
        "story:11-42",
        "setting:timeout.connect",
        "error:setting_invalid",
      ],
    },
    async ({ timeout }, { testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/hello-never-answered.json");

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const refused = await failure(Collie.connect({ env, timeout }));

      await clientCase.check(
        "a connect timeout of 0 or less, or not finite, is refused, naming timeout.connect and the null form",
        () =>
          expect(refused).toMatchObject({
            code: "setting_invalid",
            fix: expect.stringContaining("`timeout: null`"),
            limit: { key: "timeout.connect", value: timeout, layer: "run", origin: "argument" },
          }),
      );
    },
  );

  test(
    "runs beside another Connection in the same process",
    { tags: ["client_case/two-connections-at-once", "attached"] },
    async ({ testEnv, clientCase }) => {
      const { logger, warned } = recordingLogger();
      // The second Connection alone sets a Limit: its own engine expects the
      // `usage.set`, and the first's would fail on it.
      const [first, second] = await Promise.all([
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({
          env: scriptedEngineEnv(
            testEnv,
            "collie",
            "run/started-and-finished.json",
            join(testEnv.folder, "first"),
          ),
          logger,
        }),
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({
          env: scriptedEngineEnv(
            testEnv,
            "collie",
            "run/started-and-finished-under-a-limit.json",
            join(testEnv.folder, "second"),
          ),
          logger,
          limits: { modelCalls: 5 },
        }),
      ]);

      const traced = await Promise.all([startAndFinish(first), startAndFinish(second)]);

      await clientCase.check("each Connection's Run finished", () =>
        expect(traced.map(({ report }) => report.lifecycle)).toEqual(["finished", "finished"]),
      );
      await clientCase.check(
        "each engine got its own Connection's settings, the Limit the second's alone, and played its script to its end",
        () => expect(warned).toEqual([]),
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
    "fails the call in flight with the engine's exit code and crash output when the engine exits mid-call",
    { tags: ["client_case/engine-exited-carries-its-evidence", "attached", "error:engine_exited"] },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/exit-during-a-call.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const error = await failure(collie.start());
      await collie.close();
      // The same crash in a program of its own, whose stdout and stderr
      // hold only what the SDK writes there.
      const program = processes.spawn(
        process.execPath,
        ["--no-warnings", join(import.meta.dirname, "fixtures", "crashed.ts")],
        { env },
      );
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      program.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
      program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      program.child.stdin.end();
      const exit = await program.exit;

      await clientCase.check(
        "the call in flight fails with engine_exited, unavailable and not retryable",
        () =>
          expect(error).toMatchObject({
            code: "engine_exited",
            group: "unavailable",
            retryable: false,
          }),
      );
      await clientCase.check("its details carry the exit code and no signal", () =>
        expect(error).toMatchObject({ details: { exit_code: 101, signal: null } }),
      );
      await clientCase.check(
        "its stderr tail is the output's last 64 KiB, cut where a character starts",
        () =>
          expect(error).toMatchObject({
            details: { stderr_tail: `${"é".repeat(367)}\n${`${CRASH_LINE}\n`.repeat(32)}` },
          }),
      );
      await clientCase.check("each line of its stderr reached the host's logger at warning", () =>
        expect(warned).toEqual(Array.from({ length: 40 }, () => CRASH_LINE)),
      );
      await clientCase.check("the SDK wrote nothing to its own stdout or stderr", () =>
        expect({
          exit,
          stdout: Buffer.concat(stdout).toString(),
          stderr: Buffer.concat(stderr).toString(),
        }).toEqual({ exit: { code: 0, signal: null }, stdout: "", stderr: "" }),
      );
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

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}
