// The host's logger (ADR 0078): the level of the engine's logs a Connection
// receives follows what the program's logger records, sent in `hello` and
// again when a Run starts and it changed; each `$/log` event reaches the
// logger at its level; and with no logger the SDK writes nothing anywhere.
// The cases play on the Scripted engine started as the Attached engine, which
// fails on any message its script does not expect, `hello` included.
import { join } from "node:path";
import { Collie, type Logger } from "@goodfetch/collie";
import { scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** One call the logger got: its method, message and fields. */
interface Logged {
  readonly method: "debug" | "info" | "warn" | "error";
  readonly message: string;
  readonly fields: Readonly<Record<string, unknown>> | undefined;
}

/** A logger at `level` that records each call, and the calls it recorded. */
function recordingLogger(level: string): {
  readonly logger: Logger & { level: string };
  readonly logged: Logged[];
} {
  const logged: Logged[] = [];
  const record =
    (method: Logged["method"]) =>
    (message: string, fields?: Readonly<Record<string, unknown>>): void => {
      logged.push({ method, message, fields });
    };
  const logger = {
    level,
    debug: record("debug"),
    info: record("info"),
    warn: record("warn"),
    error: record("error"),
  };
  return { logger, logged };
}

/** What the logger got from the engine's `$/log` events, without the Scripted engine's own stderr lines. */
function fromEngineLog(logged: readonly Logged[]): Logged[] {
  return logged.filter(({ fields }) => fields?.["source"] === "engine");
}

describe("the host's logger on the Scripted engine as the Attached engine", () => {
  test(
    "sends the logger's level in hello, and again before a start only once it changed",
    { tags: ["client_case/log-level-follows-the-logger", "attached", "story:11-26"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/log-level-follows-the-logger.json");
      const { logger, logged } = recordingLogger("debug");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      await (await collie.start({ runId: "first" })).finish();
      await (await collie.start({ runId: "second" })).finish();
      logger.level = "warn";
      await (await collie.start({ runId: "third" })).finish();
      await collie.close();

      await clientCase.check(
        "the engine read hello at debug, no change for the second start and one change to warn before the third, so it wrote no failure",
        () => expect(logged.filter(({ method }) => method === "warn")).toEqual([]),
      );
    },
  );

  test(
    "goes on with a start whose new level the engine refused, warns, and tells it again at the next start",
    { tags: ["attached", "story:11-26"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/log-level-refused.json");
      const { logger, logged } = recordingLogger("debug");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      logger.level = "warn";
      await (await collie.start({ runId: "first" })).finish();
      await (await collie.start({ runId: "second" })).finish();
      await collie.close();

      expect(logged.map(({ method, fields }) => [method, fields?.["logLevel"]])).toEqual([
        ["warn", "warn"],
      ]);
    },
  );

  test(
    "hands each of the engine's log events to the logger at its level",
    { tags: ["client_case/engine-logs-reach-the-logger", "attached", "story:11-25"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/engine-logs-reach-the-logger.json");
      const { logger, logged } = recordingLogger("trace");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      await collie.close();

      const events = fromEngineLog(logged);
      clientCase.message(events);
      await clientCase.check(
        "each event reached the logger's method of its level, trace at debug",
        () =>
          expect(events.map(({ method }) => method)).toEqual([
            "error",
            "warn",
            "info",
            "debug",
            "debug",
          ]),
      );
      await clientCase.check("each event kept its target, its fields and its dropped count", () =>
        expect(events.map(({ fields }) => fields)).toEqual([
          {
            source: "engine",
            target: "goodfetch::rpc::dispatch",
            fields: { operation: "run.start" },
          },
          { source: "engine", target: "goodfetch::page", dropped: 3 },
          { source: "engine", target: "goodfetch::run" },
          {
            source: "engine",
            target: "goodfetch::rpc::dispatch",
            fields: { method: "run.start", id: 7 },
          },
          { source: "engine", target: "goodfetch::page::cdp" },
        ]),
      );
      await clientCase.check("the engine read hello at trace, so it wrote no failure", () =>
        expect(logged.filter(({ fields }) => fields?.["source"] === "engine stderr")).toEqual([]),
      );
    },
  );

  test(
    "writes nothing to stdout or stderr with no logger, even for the engine's log events",
    { tags: ["story:11-25"] },
    async ({ testEnv, processes }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/engine-logs-without-a-logger.json");
      const program = processes.spawn(
        process.execPath,
        [join(import.meta.dirname, "fixtures", "unlogged.ts")],
        { env },
      );
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      program.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
      program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      program.child.stdin.end();

      const exit = await program.exit;

      expect({
        exit,
        stdout: Buffer.concat(stdout).toString(),
        stderr: Buffer.concat(stderr).toString(),
      }).toEqual({ exit: { code: 0, signal: null }, stdout: "", stderr: "" });
    },
  );
});
