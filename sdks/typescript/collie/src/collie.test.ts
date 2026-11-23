// Collie's Connection on the Scripted engine (ADR 0069): each test plays a
// script of the shared corpus over the in-memory transport, as an Attached
// engine's stdio would carry it, so the hand-written runtime runs with no
// process. The script fails the test on a message it does not expect,
// `hello` included, and on a step left unused.
import { MessageChannel } from "node:worker_threads";
import { playScript, test } from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";
import { startAndFinish } from "../tests/programs/tracer.ts";
import { EngineExitedError, NotInProductError } from "@goodfetch/collie-client";
import { pageOf } from "@goodfetch/collie-client/runtime";
import { Collie } from "./collie.ts";
import { connectOver } from "./testing.ts";

/** The Scripted engine playing `script`, and a Collie Connection opened over it. */
async function collieOn(
  script: string,
): Promise<{ collie: Collie; played: ReturnType<typeof playScript> }> {
  const played = playScript(script);
  const collie = await connectOver(played.transport);
  return { collie, played };
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

describe("a Collie Connection", () => {
  test(
    "starts a Run, finishes it and answers its Run report",
    { tags: ["story:11-2"] },
    async () => {
      const { collie, played } = await collieOn("run/started-and-finished.json");

      const traced = await startAndFinish(collie);

      expect(traced).toEqual({
        startedAs: "new",
        runId: "01ke6p4ym0j8x3v5wq2rtbn7hc",
        report: {
          runId: "01ke6p4ym0j8x3v5wq2rtbn7hc",
          lifecycle: "finished",
          verdict: "verified",
          completeness: "complete",
          startedAt: new Date("2026-01-01T00:00:00Z"),
          endedAt: new Date("2026-01-01T00:00:00Z"),
          usage: {
            model: {
              calls: 0,
              tokens: 0,
              inputTokens: 0,
              cacheReadTokens: 0,
              cacheWriteTokens: 0,
              outputTokens: 0,
              reasoningTokens: 0,
              price: null,
            },
            browserTime: 0,
            proxyBytes: 0,
            requests: 0,
          },
        },
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("reports the engine it reached, of either Product", async () => {
    const { collie, played } = await collieOn("connect/accepted-by-foxhound.json");

    const engine = collie.engine();
    await collie.close();

    expect(engine).toEqual({
      kind: "attached",
      product: "foxhound",
      version: "0.0.0",
      protocolVersion: "0.1",
      projectFile: null,
      home: null,
      enginePath: null,
      url: null,
      fallback: null,
    });
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test(
    "fails connect() when the engine refuses its Product, naming the fix",
    { tags: ["story:11-2", "error:not_in_product"] },
    async () => {
      const played = playScript("connect/refused-not-in-product.json");

      const error = await failure(connectOver(played.transport));

      expect(error).toBeInstanceOf(NotInProductError);
      expect(error).toMatchObject({
        code: "not_in_product",
        group: "unavailable",
        fix: "Install @goodfetch/collie and connect to its engine.",
        details: { needs: "collie", product: "foxhound", product_version: "0.0.0" },
      });
      expect(played.done()).toEqual({ kind: "closed" });
    },
  );

  test(
    "fails connect() when the engine refuses its protocol version, naming the fix",
    { tags: ["story:11-2", "error:protocol_version_unsupported"] },
    async () => {
      const played = playScript("connect/refused-protocol-version-unsupported.json");

      const error = await failure(connectOver(played.transport));

      expect(error).toMatchObject({
        code: "protocol_version_unsupported",
        fix: "Upgrade the engine to a version that speaks protocol 0.1, or use a Client of the engine's own version.",
      });
      expect(played.done()).toEqual({ kind: "closed" });
    },
  );

  test(
    "refuses an engine that answers with another protocol major, naming the fix",
    { tags: ["error:protocol_version_unsupported"] },
    async () => {
      const played = playScript("connect/answered-with-another-major.json");

      const error = await failure(connectOver(played.transport));

      expect(error).toMatchObject({
        code: "protocol_version_unsupported",
        details: { client_protocol_version: "0.1", engine_protocol_version: "1.0" },
        fix: "Use the engine and the SDK of one Product version: this SDK 0.0.0 speaks protocol 0.1, and the engine 1.0.0 speaks 1.0.",
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses an engine of a newer preview minor, which counts as a major",
    { tags: ["error:protocol_version_unsupported"] },
    async () => {
      const played = playScript("connect/answered-with-another-preview-minor.json");

      const error = await failure(connectOver(played.transport));

      expect(error).toMatchObject({
        code: "protocol_version_unsupported",
        details: { client_protocol_version: "0.1", engine_protocol_version: "0.2" },
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses an engine whose hello answer names no protocol version",
    { tags: ["error:protocol_version_unsupported"] },
    async () => {
      const played = playScript("connect/answered-without-a-protocol-version.json");

      const error = await failure(connectOver(played.transport));

      expect(error).toMatchObject({
        code: "protocol_version_unsupported",
        details: { client_protocol_version: "0.1", engine_protocol_version: null },
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("aborts a Run once when two closes run at once", async () => {
    const { collie, played } = await collieOn("run/closed-while-running.json");
    await collie.start({ runId: "nightly", tags: ["shop"] });

    await Promise.all([collie.close(), collie.close()]);

    expect(played.done()).toEqual({ kind: "finished" });
  });

  test(
    "fails connect() with deadline_exceeded once timeout.connect's default passes with hello unanswered",
    { tags: ["story:11-41", "story:11-42", "setting:timeout.connect", "error:deadline_exceeded"] },
    async () => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
      try {
        const played = playScript("connect/hello-never-answered.json");

        const connected = failure(connectOver(played.transport));
        await vi.advanceTimersByTimeAsync(30_000);
        const error = await connected;

        expect(error).toMatchObject({
          code: "deadline_exceeded",
          group: "deadline_exceeded",
          limit: { key: "timeout.connect", value: 30_000, layer: "default" },
          details: {
            elapsed: 30_000,
            waited_for: "hello",
          },
        });
        expect(played.done()).toEqual({ kind: "finished" });
      } finally {
        vi.useRealTimers();
      }
    },
  );

  test(
    "gives up waiting for a Run's abort once timeout.close passes, failing the close with deadline_exceeded",
    { tags: ["story:11-2", "story:11-43", "error:deadline_exceeded"] },
    async () => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
      try {
        const { collie, played } = await collieOn("run/abort-never-answered.json");
        await collie.start({ runId: "nightly" });

        const closed = failure(collie.close());
        await vi.advanceTimersByTimeAsync(30_000);
        const error = await closed;

        expect(error).toMatchObject({
          code: "deadline_exceeded",
          group: "deadline_exceeded",
          limit: { key: "timeout.close", value: 30_000, layer: "default" },
          details: {
            elapsed: 30_000,
            waited_for: "close",
          },
        });
        expect(played.done()).toEqual({ kind: "finished" });
      } finally {
        vi.useRealTimers();
      }
    },
  );

  test(
    "fails the call in flight when the engine ends before the Connection closes",
    { tags: ["error:engine_exited"] },
    async () => {
      const { collie, played } = await collieOn("connect/exit-during-a-call.json");

      const error = await failure(collie.start());
      await collie.close();

      expect(error).toMatchObject({
        code: "engine_exited",
        group: "unavailable",
        retryable: false,
        details: { exit_code: null, signal: null, stderr_tail: "" },
      });
      expect(played.done()).toEqual({ kind: "exit", code: 101 });
    },
  );
});

describe("connect() over a transport whose writes the engine's side rejects", () => {
  test(
    "fails with the write's cause, though the engine's bytes to read stay open",
    { tags: ["error:engine_exited"] },
    async () => {
      const refused = new Error("the pipe is broken");
      const transport = {
        readable: new ReadableStream<Uint8Array>(),
        writable: new WritableStream<Uint8Array>({ write: () => Promise.reject(refused) }),
      };

      const error = await failure(connectOver(transport));

      expect(error).toBeInstanceOf(EngineExitedError);
      expect(error).toMatchObject({ code: "engine_exited", cause: refused });
    },
  );
});

/**
 * An engine played by hand whose `hello` result caps its calls in flight
 * at `cap` and resolves `timeouts`: each later request it reads waits in
 * `calls` for `answer`.
 */
function cappingEngine(
  cap: number,
  timeouts: readonly unknown[] = [],
): {
  transport: { readable: ReadableStream<Uint8Array>; writable: WritableStream<Uint8Array> };
  calls: { readonly id: unknown; readonly method: unknown }[];
  answer(id: unknown, result: unknown): void;
} {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const calls: { readonly id: unknown; readonly method: unknown }[] = [];
  let replies: ReadableStreamDefaultController<Uint8Array> | undefined;
  const reply = (id: unknown, result: unknown): void =>
    replies?.enqueue(encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`));
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      replies = controller;
    },
  });
  const writable = new WritableStream<Uint8Array>({
    write(bytes) {
      const lines = decoder.decode(bytes).split("\n");
      for (const line of lines.filter((text) => text !== "")) {
        const message: unknown = JSON.parse(line);
        if (typeof message !== "object" || message === null || !("id" in message)) {
          continue;
        }
        const method = "method" in message ? message.method : undefined;
        if (method === "hello") {
          reply(message.id, {
            protocol_version: "0.1",
            product: "collie",
            product_version: "0.0.0",
            engine_instance: "0123456789abcdef",
            max_calls_in_flight: cap,
            timeouts,
          });
        } else {
          calls.push({ id: message.id, method });
        }
      }
    },
  });
  return { transport: { readable, writable }, calls, answer: reply };
}

/** Answers with `result` the call `engine` reads in place `index`, once it is sent. */
async function answerCall(
  engine: ReturnType<typeof cappingEngine>,
  index: number,
  result: unknown,
): Promise<void> {
  await expect.poll(() => engine.calls.length).toBeGreaterThan(index);
  engine.answer(engine.calls[index]?.id, result);
}

describe("a Connection whose engine caps its calls in flight", () => {
  test("starts 300 calls at once and gets every answer, at most the cap in flight, the rest sent in order", async () => {
    const engine = cappingEngine(256);
    const collie = await connectOver(engine.transport);
    const calls = Array.from({ length: 300 }, () => collie.call("run.list", {}));
    await new Promise((resolve) => setImmediate(resolve));
    const sentAtOnce = engine.calls.length;

    for (let index = 0; index < 300; index += 1) {
      await answerCall(engine, index, { runs: [] });
    }
    const answers = await Promise.all(calls);

    expect(sentAtOnce).toBe(256);
    expect(engine.calls.map((call) => call.id)).toEqual(
      Array.from({ length: 300 }, (_, index) => index + 1),
    );
    expect(answers).toEqual(Array.from({ length: 300 }, () => ({ runs: [] })));
  });

  test(
    "fails a raw call that waits past its timeout for a place with deadline_exceeded, never sent",
    { tags: ["error:deadline_exceeded"] },
    async () => {
      const engine = cappingEngine(1);
      const collie = await connectOver(engine.transport);
      const holding = collie.call("run.list", {});

      const error = await failure(collie.call("run.list", {}, { timeout: 5 }));
      await answerCall(engine, 0, { runs: [] });
      await holding;

      expect(error).toMatchObject({
        code: "deadline_exceeded",
        limit: { value: 5, layer: "run", origin: "argument" },
        details: { waited_for: "calls_in_flight" },
      });
      expect(engine.calls).toHaveLength(1);
    },
  );

  test(
    "fails a handler's page call that waits past its timeout for a place with deadline_exceeded, never sent",
    { tags: ["error:deadline_exceeded"] },
    async () => {
      const engine = cappingEngine(1);
      const collie = await connectOver(engine.transport);
      const starting = collie.start();
      await answerCall(engine, 0, {
        run_id: "01ke6p4ym0j8x3v5wq2rtbn7hc",
        lifecycle: "running",
        needs: "collie",
        started_as: "new",
      });
      const run = await starting;
      const holding = collie.call("run.list", {});
      const page = pageOf(run, "page-1", new AbortController().signal);

      const error = await failure(page.goto("https://example.com/", { timeout: 5 }));
      await answerCall(engine, 1, { runs: [] });
      await holding;

      expect(error).toMatchObject({
        code: "deadline_exceeded",
        limit: { key: "timeout.navigation", value: 5 },
        details: { waited_for: "calls_in_flight" },
      });
      expect(engine.calls.map((call) => call.method)).toEqual(["run.start", "run.list"]);
    },
  );

  test(
    "fails a handler's page call with no timeout of its own at the setting's value hello resolved, never sent",
    { tags: ["error:deadline_exceeded"] },
    async () => {
      const navigation = { key: "timeout.navigation", value: 5, layer: "user_configuration" };
      const engine = cappingEngine(1, [navigation]);
      const collie = await connectOver(engine.transport);
      const starting = collie.start();
      await answerCall(engine, 0, {
        run_id: "01ke6p4ym0j8x3v5wq2rtbn7hc",
        lifecycle: "running",
        needs: "collie",
        started_as: "new",
      });
      const run = await starting;
      const holding = collie.call("run.list", {});
      const page = pageOf(run, "page-1", new AbortController().signal);

      const error = await failure(page.goto("https://example.com/"));
      await answerCall(engine, 1, { runs: [] });
      await holding;

      expect(error).toMatchObject({
        code: "deadline_exceeded",
        limit: navigation,
        details: { waited_for: "calls_in_flight" },
      });
      expect(engine.calls.map((call) => call.method)).toEqual(["run.start", "run.list"]);
    },
  );
});

/**
 * The transport of `played`, which takes only the first message written,
 * `hello`: each later one goes to `write` instead.
 */
function afterHello(
  played: ReturnType<typeof playScript>,
  write: (bytes: Uint8Array) => void | Promise<void>,
): { readable: ReadableStream<Uint8Array>; writable: WritableStream<Uint8Array> } {
  const engine = played.transport.writable.getWriter();
  let greeted = false;
  const writable = new WritableStream<Uint8Array>({
    write: (bytes) => {
      if (greeted) {
        return write(bytes);
      }
      greeted = true;
      return engine.write(bytes);
    },
  });
  return { readable: played.transport.readable, writable };
}

describe("a call on a Connection whose writes the engine's side then rejects", () => {
  test(
    "fails with the write's cause, though the engine's bytes to read stay open",
    { tags: ["error:engine_exited"] },
    async () => {
      const refused = new Error("the pipe is broken");
      const played = playScript("connect/accepted-by-foxhound.json");
      const collie = await connectOver(afterHello(played, () => Promise.reject(refused)));

      const error = await failure(collie.start());

      expect(error).toBeInstanceOf(EngineExitedError);
      expect(error).toMatchObject({ code: "engine_exited", cause: refused });
    },
  );
});

describe("a Connection whose settings set bounds of its own", () => {
  test(
    "refuses the call past its bound on calls waiting",
    { tags: ["error:connection_overloaded", "setting:connection.max_pending_calls"] },
    async () => {
      const played = playScript("connect/bound-pending-calls-set.json");
      const transport = afterHello(played, () => {});
      const collie = await connectOver(transport, {
        settings: { connection: { maxPendingCalls: 1 } },
      });
      let earlier = "waiting";
      collie.start().then(
        () => (earlier = "answered"),
        () => (earlier = "refused"),
      );

      const error = await failure(collie.start());
      await new Promise((resolve) => setImmediate(resolve));

      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "pending_calls", value: 1 },
      });
      expect(earlier).toBe("waiting");
    },
  );

  test(
    "refuses the call past its bound on bytes waiting to be written",
    { tags: ["error:connection_overloaded", "setting:connection.max_queued_write_bytes"] },
    async () => {
      const played = playScript("connect/bound-queued-write-bytes-set.json");
      const transport = afterHello(played, () => new Promise<void>(() => {}));
      const collie = await connectOver(transport, {
        settings: { connection: { maxQueuedWriteBytes: 1 } },
      });
      void collie.start();

      const error = await failure(collie.start());

      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "queued_write_bytes", value: 1 },
      });
    },
  );

  test.for([
    { key: "connection.max_pending_calls", connection: { maxPendingCalls: 0 } },
    { key: "connection.max_queued_write_bytes", connection: { maxQueuedWriteBytes: -1 } },
    { key: "connection.max_listener_queue", connection: { maxListenerQueue: 1.5 } },
  ])(
    "fails to open on a $key that is no whole number of 1 or more, before it takes the transport",
    { tags: ["error:setting_invalid"] },
    async ({ key, connection }) => {
      const played = playScript("connect/accepted-by-foxhound.json");

      const error = await failure(connectOver(played.transport, { settings: { connection } }));

      expect(error).toMatchObject({ code: "setting_invalid", details: { key, layer: "run" } });
      expect(played.transport.writable.locked).toBe(false);
    },
  );
});

describe("connect() aborted before its hello is answered", () => {
  test("fails with the signal's reason and leaves neither stream open", async () => {
    const streams = { cancelled: false, aborted: false };
    const transport = {
      readable: new ReadableStream<Uint8Array>({
        cancel: () => {
          streams.cancelled = true;
        },
      }),
      writable: new WritableStream<Uint8Array>({
        abort: () => {
          streams.aborted = true;
        },
      }),
    };
    const aborting = new AbortController();
    const reason = new Error("the program gave up");

    const connecting = failure(connectOver(transport, { signal: aborting.signal }));
    aborting.abort(reason);

    expect(await connecting).toBe(reason);
    expect(streams).toEqual({ cancelled: true, aborted: true });
  });

  test("fails with the signal's reason though its hello is never written", async () => {
    const transport = {
      readable: new ReadableStream<Uint8Array>(),
      writable: new WritableStream<Uint8Array>({ write: () => new Promise<void>(() => {}) }),
    };
    const aborting = new AbortController();
    const reason = new Error("the program gave up");

    const connecting = failure(connectOver(transport, { signal: aborting.signal }));
    aborting.abort(reason);

    expect(await connecting).toBe(reason);
  });

  test("fails with the reason of a signal aborted already, writing nothing", async () => {
    const written: Uint8Array[] = [];
    const transport = {
      readable: new ReadableStream<Uint8Array>(),
      writable: new WritableStream<Uint8Array>({
        write: (bytes) => {
          written.push(bytes);
        },
      }),
    };
    const reason = new Error("the program gave up");

    const error = await failure(connectOver(transport, { signal: AbortSignal.abort(reason) }));

    expect({ error, written }).toEqual({ error: reason, written: [] });
  });
});

describe("a Connection's values", () => {
  test("are refused when passed to another realm", { tags: ["story:11-53"] }, async () => {
    const { collie, played } = await collieOn("run/started-and-finished.json");
    const run = await collie.start();
    const { port1, port2 } = new MessageChannel();

    const refused = [collie, run].map((value) => {
      try {
        port1.postMessage(value);
        return "sent";
      } catch (error) {
        return error instanceof Error ? error.name : "thrown";
      }
    });
    port1.close();
    port2.close();
    await run.finish();
    await collie.close();

    expect(refused).toEqual(["DataCloneError", "DataCloneError"]);
    expect(played.done()).toEqual({ kind: "finished" });
  });
});

describe("Collie.connect()", () => {
  test(
    "fails when its engine-path variable names no executable, naming the variable",
    { tags: ["story:11-2", "error:engine_not_found"] },
    async () => {
      const path = "/nonexistent/goodfetch-collie";

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { GOODFETCH_COLLIE_ENGINE_PATH: path } }));

      expect(error).toMatchObject({
        code: "engine_not_found",
        group: "unavailable",
        details: { path },
        fix: `GOODFETCH_COLLIE_ENGINE_PATH names ${path}, which is no executable file: point it at Collie's executable, goodfetch-collie, or leave it unset to use the one @goodfetch/collie installed.`,
      });
    },
  );

  test(
    "fails when no platform package is installed and no variable is set, naming the package",
    { tags: ["story:11-2", "error:engine_not_found"] },
    async () => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: {} }));

      const platformPackage = `@goodfetch/collie-${process.platform}-${process.arch}`;
      expect(error).toMatchObject({
        code: "engine_not_found",
        details: { package: platformPackage },
        fix: `Install @goodfetch/collie with its optional dependencies, which hold its engine in ${platformPackage}, or set GOODFETCH_COLLIE_ENGINE_PATH to Collie's executable.`,
      });
    },
  );

  test(
    "fails when its engine-path variable is set but empty, naming the variable",
    { tags: ["story:11-2", "error:setting_invalid"] },
    async () => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { GOODFETCH_COLLIE_ENGINE_PATH: "" } }));

      expect(error).toMatchObject({
        code: "setting_invalid",
        group: "program",
        details: { key: "GOODFETCH_COLLIE_ENGINE_PATH" },
      });
    },
  );

  test(
    "fails on a GOODFETCH_ variable off the closed list, naming the closest",
    { tags: ["story:11-73", "error:setting_unknown"] },
    async () => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { GOODFETCH_HOEM: "/home/goodfetch" } }));

      expect(error).toMatchObject({
        code: "setting_unknown",
        group: "program",
        message:
          "`GOODFETCH_HOEM` in the environment is no deployment variable; did you mean `GOODFETCH_HOME`?",
        fix: "Rename GOODFETCH_HOEM to GOODFETCH_HOME, or unset it.",
        details: { key: "GOODFETCH_HOEM", closest: "GOODFETCH_HOME" },
      });
    },
  );

  test(
    "fails on a GOODFETCH_ variable close to none on the list, naming none",
    { tags: ["story:11-73", "error:setting_unknown"] },
    async () => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { GOODFETCH_TELEMETRY: "1" } }));

      expect(error).toMatchObject({
        code: "setting_unknown",
        details: { key: "GOODFETCH_TELEMETRY" },
      });
      expect(error).not.toHaveProperty("details.closest");
    },
  );

  test(
    "fails on an empty deployment variable rather than taking it for unset",
    { tags: ["story:11-73", "error:setting_invalid"] },
    async () => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { GOODFETCH_URL: "" } }));

      expect(error).toMatchObject({
        code: "setting_invalid",
        group: "program",
        fix: "Set GOODFETCH_URL to a non-empty Unicode value, or unset it.",
        details: { key: "GOODFETCH_URL", reason: "it is empty; unset it instead" },
      });
    },
  );

  test(
    "fails on a deployment variable whose value is no well-formed Unicode",
    { tags: ["story:11-73", "error:setting_invalid"] },
    async () => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { GOODFETCH_HOME: "/home/\uDCFF" } }));

      expect(error).toMatchObject({
        code: "setting_invalid",
        details: { key: "GOODFETCH_HOME", reason: "it is not Unicode" },
      });
    },
  );

  const channels = "one of npm, pypi, crates-io, homebrew, oci, installer";
  const milliseconds = "a whole number of milliseconds";
  test.for([
    {
      name: "a channel in capitals",
      key: "GOODFETCH_CHANNEL",
      value: "NPM",
      reason: `\`NPM\` is no channel; ${channels}`,
      taken: channels,
    },
    {
      name: "a drain grace in seconds",
      key: "GOODFETCH_DAEMON_DRAIN_GRACE",
      value: "5s",
      reason: "`5s` is no whole number of milliseconds",
      taken: milliseconds,
    },
    {
      name: "a negative drain grace",
      key: "GOODFETCH_DAEMON_DRAIN_GRACE",
      value: "-1",
      reason: "`-1` is no whole number of milliseconds",
      taken: milliseconds,
    },
    {
      name: "a drain grace past u64",
      key: "GOODFETCH_DAEMON_DRAIN_GRACE",
      value: "18446744073709551616",
      reason: "`18446744073709551616` is no whole number of milliseconds",
      taken: milliseconds,
    },
    {
      name: "a Daemon address by host name",
      key: "GOODFETCH_DAEMON_ADDRESS",
      value: "localhost:7900",
      reason: "`localhost:7900` is no `host:port`",
      taken: "an IP address and port such as 127.0.0.1:7900",
    },
    {
      name: "an ops address with a named scope",
      key: "GOODFETCH_OPS_ADDRESS",
      value: "[fe80::1%eth0]:9464",
      reason: "`[fe80::1%eth0]:9464` is no `host:port`",
      taken: "an IP address and port such as 0.0.0.0:9464",
    },
    {
      name: "an ops address with a leading zero",
      key: "GOODFETCH_OPS_ADDRESS",
      value: "10.0.0.01:9464",
      reason: "`10.0.0.01:9464` is no `host:port`",
      taken: "an IP address and port such as 0.0.0.0:9464",
    },
  ])(
    "fails on $name, which the engine cannot parse, as its own check does",
    { tags: ["story:11-73", "error:setting_invalid"] },
    async ({ key, value, reason, taken }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env: { [key]: value } }));

      expect(error).toMatchObject({
        code: "setting_invalid",
        group: "program",
        fix: `Set ${key} to ${taken}, or unset it.`,
        details: { key, reason },
      });
    },
  );

  test.for([
    { name: "a channel", key: "GOODFETCH_CHANNEL", value: "crates-io" },
    { name: "a signed drain grace", key: "GOODFETCH_DAEMON_DRAIN_GRACE", value: "+5" },
    {
      name: "the longest drain grace",
      key: "GOODFETCH_DAEMON_DRAIN_GRACE",
      value: "18446744073709551615",
    },
    { name: "an IPv4 Daemon address", key: "GOODFETCH_DAEMON_ADDRESS", value: "127.0.0.1:7900" },
    {
      name: "an IPv6 ops address with its scope",
      key: "GOODFETCH_OPS_ADDRESS",
      value: "[fe80::1%2]:9464",
    },
  ])("goes on with $name, as Rust reads it", { tags: ["story:11-73"] }, async ({ key, value }) => {
    const error = await failure(
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      Collie.connect({ env: { [key]: value, GOODFETCH_COLLIE_ENGINE_PATH: "/nonexistent" } }),
    );

    expect(error).toMatchObject({ code: "engine_not_found" });
  });

  test(
    "reads only Collie's engine-path variable, never Foxhound's",
    { tags: ["story:11-2", "story:11-299"] },
    async () => {
      const error = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({ env: { GOODFETCH_FOXHOUND_ENGINE_PATH: process.execPath } }),
      );

      expect(error).toMatchObject({ code: "engine_not_found" });
    },
  );
});
