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
          run_id: "01ke6p4ym0j8x3v5wq2rtbn7hc",
          lifecycle: "finished",
          verdict: "verified",
          completeness: "complete",
          started_at: new Date("2026-01-01T00:00:00Z"),
          ended_at: new Date("2026-01-01T00:00:00Z"),
          usage: {
            model: {
              calls: 0,
              tokens: 0,
              input_tokens: 0,
              cache_read_tokens: 0,
              cache_write_tokens: 0,
              output_tokens: 0,
              reasoning_tokens: 0,
              price: null,
            },
            browser_time: 0,
            proxy_bytes: 0,
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
    "closes a Run still running by aborting it, and refuses every later call with closed",
    { tags: ["error:closed"] },
    async () => {
      const { collie, played } = await collieOn("run/closed-while-running.json");
      const run = await collie.start({ runId: "nightly", tags: ["shop"] });

      await run.close();
      await run.close();
      await collie.close();
      await collie.close();

      expect(await failure(run.finish())).toMatchObject({ code: "closed", group: "program" });
      expect(await failure(collie.start())).toMatchObject({ code: "closed", group: "program" });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

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
    "fails a call in flight when it closes, as cancelled by its Client",
    { tags: ["story:11-2", "error:call_cancelled"] },
    async () => {
      const { collie, played } = await collieOn("connect/closed-during-a-call.json");
      const started = failure(collie.start());

      await collie.close();
      const error = await started;

      expect(error).toMatchObject({
        code: "call_cancelled",
        group: "unavailable",
        details: { by: "client" },
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("runs beside another Connection in the same process", async () => {
    const [first, second] = await Promise.all([
      collieOn("run/started-and-finished.json"),
      collieOn("run/started-and-finished.json"),
    ]);

    const traced = await Promise.all([startAndFinish(first.collie), startAndFinish(second.collie)]);

    expect(traced.map(({ report }) => report.lifecycle)).toEqual(["finished", "finished"]);
    expect([first.played.done(), second.played.done()]).toEqual([
      { kind: "finished" },
      { kind: "finished" },
    ]);
  });

  test("connects again after a close", async () => {
    const first = await collieOn("connect/accepted-by-foxhound.json");
    await first.collie.close();

    const second = await collieOn("run/started-and-finished.json");
    const traced = await startAndFinish(second.collie);

    expect(traced.report.lifecycle).toBe("finished");
    expect([first.played.done(), second.played.done()]).toEqual([
      { kind: "finished" },
      { kind: "finished" },
    ]);
  });

  test(
    "ends when the engine writes a line that is not JSON-RPC, failing the call",
    { tags: ["error:engine_output_invalid"] },
    async () => {
      const { collie, played } = await collieOn("connect/garbage-during-a-call.json");

      const error = await failure(collie.start());
      await collie.close();

      expect(error).toMatchObject({
        code: "engine_output_invalid",
        group: "internal",
        details: { line: "INFO engine started" },
      });
      expect(played.done()).toEqual({ kind: "finished" });
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

describe("a Connection opened with bounds of its own", () => {
  test(
    "refuses the call past its bound on calls waiting",
    { tags: ["error:connection_overloaded"] },
    async () => {
      const played = playScript("connect/accepted-by-foxhound.json");
      const transport = afterHello(played, () => {});
      const collie = await connectOver(transport, { maxPendingCalls: 1 });
      void collie.start();

      const error = await failure(collie.start());

      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "pending_calls", value: 1 },
      });
    },
  );

  test(
    "refuses the call past its bound on bytes waiting to be written",
    { tags: ["error:connection_overloaded"] },
    async () => {
      const played = playScript("connect/accepted-by-foxhound.json");
      const transport = afterHello(played, () => new Promise<void>(() => {}));
      const collie = await connectOver(transport, { maxQueuedWriteBytes: 1 });
      void collie.start();

      const error = await failure(collie.start());

      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "queued_write_bytes", value: 1 },
      });
    },
  );

  test(
    "fails to open on a bound that is no count, before it takes the transport",
    { tags: ["error:setting_invalid"] },
    async () => {
      const played = playScript("connect/accepted-by-foxhound.json");

      const error = await failure(connectOver(played.transport, { maxPendingCalls: 0 }));

      expect(error).toMatchObject({ code: "setting_invalid" });
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
    "reads only Collie's engine-path variable, never Foxhound's",
    { tags: ["story:11-2"] },
    async () => {
      const error = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({ env: { GOODFETCH_FOXHOUND_ENGINE_PATH: process.execPath } }),
      );

      expect(error).toMatchObject({ code: "engine_not_found" });
    },
  );
});
