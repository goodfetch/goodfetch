// The Channel over a transport the test plays the engine's side of: what it
// writes, the replies it reads in any order, and how a call that cannot be
// sent fails alone.
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { encodeMessage } from "./framing.ts";
import {
  CallCancelledError,
  ClosedError,
  ConnectionLostError,
  ConnectionOverloadedError,
  RunNotFoundError,
} from "./generated/index.ts";
import {
  Channel,
  type ChannelOwner,
  type ChannelRelinking,
  type Transport,
  type TransportEnd,
} from "./rpc.ts";

/** The engine's side of an in-memory transport: what the Client wrote, and what it reads next. */
interface PlayedEngine {
  readonly transport: Transport;
  /** Each message the Client wrote, parsed. */
  readonly written: unknown[];
  /** Sends the Client one message. */
  send(message: Readonly<Record<string, unknown>>): void;
  /** Sends the Client one line as it is, such as one that is no message. */
  sendLine(line: string): void;
  /** Ends the engine's side. */
  end(): void;
}

function playedEngine(): PlayedEngine {
  const decoder = new TextDecoder();
  const written: unknown[] = [];
  let engine: ReadableStreamDefaultController<Uint8Array> | undefined;
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      engine = controller;
    },
  });
  const writable = new WritableStream<Uint8Array>({
    write(bytes) {
      written.push(JSON.parse(decoder.decode(bytes)));
    },
  });
  const controller = (): ReadableStreamDefaultController<Uint8Array> => {
    if (engine === undefined) {
      throw new TypeError("invariant: a ReadableStream starts at once");
    }
    return engine;
  };
  return {
    transport: { readable, writable },
    written,
    send: (message) => controller().enqueue(encodeMessage(message)),
    sendLine: (line) => controller().enqueue(new TextEncoder().encode(`${line}\n`)),
    end: () => controller().close(),
  };
}

const owner: ChannelOwner = {
  operationNames: {},
  ended: () => Promise.resolve(new ClosedError("the transport ended")),
  failure: () => new ClosedError("the engine failed the call"),
};

/** Lets every queued write and read settle. */
async function settled(): Promise<void> {
  await new Promise((resolve) => setImmediate(resolve));
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

describe("a Channel", () => {
  test("fails a call whose params cannot be encoded, and leaves nothing to cancel", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);

    const error = await failure(channel.call("run.start", { tags: [1n] }));
    channel.cancelAll(new CallCancelledError("the call's Connection was closed"));
    await settled();

    expect({ error: error instanceof TypeError, written: engine.written }).toEqual({
      error: true,
      written: [],
    });
  });

  test("answers each of 50 calls in flight by its id, whatever order the replies come in", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    const calls = Array.from({ length: 50 }, (_, index) => channel.call("sample.echo", { index }));
    await settled();

    for (let id = 49; id >= 0; id -= 1) {
      engine.send({ id, result: { echoed: id } });
    }
    const answers = await Promise.all(calls);

    expect(answers).toEqual(Array.from({ length: 50 }, (_, id) => ({ echoed: id })));
  });

  test("fails a call in flight with the error its cancellation gives, and tells the engine", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    const call = failure(channel.call("run.abort", { run_id: "nightly" }));
    await settled();

    channel.cancelAll(new CallCancelledError("the call's Connection was closed"));
    const error = await call;

    expect(error).toEqual(expect.objectContaining({ code: "call_cancelled" }));
    await expect
      .poll(() => engine.written)
      .toEqual([
        { jsonrpc: "2.0", id: 0, method: "run.abort", params: { run_id: "nightly" } },
        { jsonrpc: "2.0", method: "$/cancel", params: { id: 0 } },
      ]);
  });

  test("fails a call in flight with its owner's error once the engine's side ends", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    const call = failure(channel.call("run.start", {}));
    await settled();

    engine.end();

    expect(await call).toMatchObject({ code: "closed" });
  });

  test(
    "refuses a call past its bound on calls waiting, at once, and answers the earlier ones",
    { tags: ["error:connection_overloaded"] },
    async () => {
      const engine = playedEngine();
      const channel = new Channel(engine.transport, owner, { maxPendingCalls: 2 });
      const earlier = [channel.call("sample.echo", {}), channel.call("sample.echo", {})];

      const error = await failure(channel.call("sample.echo", {}));
      await settled();
      engine.send({ id: 0, result: "first" });
      engine.send({ id: 1, result: "second" });

      expect(error).toBeInstanceOf(ConnectionOverloadedError);
      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "pending_calls", value: 2 },
      });
      expect(await Promise.all(earlier)).toEqual(["first", "second"]);
      expect(engine.written).toHaveLength(2);
    },
  );

  test(
    "refuses a call past its bound on bytes waiting to be written, and takes one again once they are",
    { tags: ["error:connection_overloaded"] },
    async () => {
      const writes: (() => void)[] = [];
      const writable = new WritableStream<Uint8Array>(
        { write: () => new Promise<void>((resolve) => writes.push(resolve)) },
        new CountQueuingStrategy({ highWaterMark: 16 }),
      );
      const transport = { readable: new ReadableStream<Uint8Array>(), writable };
      const channel = new Channel(transport, owner, { maxQueuedWriteBytes: 100 });
      const text = "x".repeat(100);

      // One message larger than the bound is taken while nothing waits, or it could never be sent.
      void channel.call("sample.echo", { text });
      const error = await failure(channel.call("sample.echo", {}));
      await settled();
      writes.shift()?.();
      await settled();
      void channel.call("sample.echo", {});
      await settled();

      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "queued_write_bytes", value: 100 },
      });
      expect(writes).toHaveLength(1);
    },
  );

  test.for([0, -1, 1.5, Number.NaN])(
    "refuses a bound of %s, taking neither stream",
    { tags: ["error:setting_invalid"] },
    (bound) => {
      const { transport } = playedEngine();

      expect(() => new Channel(transport, owner, { maxPendingCalls: bound })).toThrow(
        expect.objectContaining({ code: "setting_invalid" }),
      );
      expect(() => new Channel(transport, owner, { maxQueuedWriteBytes: bound })).toThrow(
        expect.objectContaining({ code: "setting_invalid" }),
      );
      expect([transport.readable.locked, transport.writable.locked]).toEqual([false, false]);
    },
  );

  test("stops reading the engine's bytes once one of its lines is no message", async () => {
    const stopped: unknown[] = [];
    const readable = new ReadableStream<Uint8Array>({
      start: (controller) => {
        controller.enqueue(new TextEncoder().encode("no message\n"));
      },
      cancel: (reason) => {
        stopped.push(reason);
      },
    });
    const channel = new Channel({ readable, writable: new WritableStream<Uint8Array>() }, owner);

    await channel.ended;

    expect(stopped).toHaveLength(1);
  });

  test("ends on a line that is no message as such, though a write was rejected too", async () => {
    const readable = new ReadableStream<Uint8Array>({
      start: (controller) => {
        controller.enqueue(new TextEncoder().encode("no message\n"));
      },
    });
    const writable = new WritableStream<Uint8Array>({
      write: () => Promise.reject(new Error("the pipe is broken")),
    });
    const ends: TransportEnd[] = [];
    const channel = new Channel(
      { readable, writable },
      {
        ...owner,
        ended: (end) => {
          ends.push(end);
          return Promise.resolve(new ClosedError("the transport ended"));
        },
      },
    );

    await failure(channel.call("run.start", {}));
    await channel.ended;

    expect(ends).toEqual([{ kind: "invalid", line: "no message" }]);
  });

  test("fails a call in flight with the cause of a write the engine's side rejected, its readable still open", async () => {
    const refused = new Error("the pipe is broken");
    const readable = new ReadableStream<Uint8Array>();
    const writable = new WritableStream<Uint8Array>({
      write: () => Promise.reject(refused),
    });
    const ends: TransportEnd[] = [];
    const channel = new Channel(
      { readable, writable },
      {
        ...owner,
        ended: (end) => {
          ends.push(end);
          return Promise.resolve(new ClosedError("the transport ended"));
        },
      },
    );

    const error = await failure(channel.call("run.start", {}));
    await channel.ended;

    expect({ error, ends }).toEqual({
      error: expect.objectContaining({ code: "closed" }),
      ends: [{ kind: "write_failed", cause: refused }],
    });
  });
});

describe("a Channel under the engine's cap on calls in flight", () => {
  test("keeps 300 calls at most at the cap in flight, sends the rest in the order they were made, and answers them all", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(256);
    const calls = Array.from({ length: 300 }, (_, index) => channel.call("sample.echo", { index }));
    await settled();
    const sentAtOnce = engine.written.length;

    let mostInFlight = 0;
    for (let id = 0; id < 300; id += 1) {
      await expect.poll(() => engine.written.length).toBeGreaterThan(id);
      mostInFlight = Math.max(mostInFlight, engine.written.length - id);
      engine.send({ id, result: { echoed: id } });
    }
    const answers = await Promise.all(calls);

    expect({ sentAtOnce, mostInFlight }).toEqual({ sentAtOnce: 256, mostInFlight: 256 });
    expect(answers).toEqual(Array.from({ length: 300 }, (_, id) => ({ echoed: id })));
    expect(engine.written).toEqual(
      Array.from({ length: 300 }, (_, id) =>
        expect.objectContaining({ id, params: { index: id } }),
      ),
    );
  });

  test(
    "fails a queued call whose timeout passes with deadline_exceeded, never sending it",
    { tags: ["error:deadline_exceeded"] },
    async () => {
      const engine = playedEngine();
      const channel = new Channel(engine.transport, owner);
      channel.capCallsInFlight(1);
      const first = channel.call("sample.echo", {});

      const error = await failure(
        channel.call(
          "page.goto",
          { timeout: 1 },
          { deadline: { key: "timeout.navigation", value: 1 } },
        ),
      );
      engine.send({ id: 0, result: "first" });
      await first;
      await settled();

      expect(error).toMatchObject({
        code: "deadline_exceeded",
        retryable: true,
        limit: { key: "timeout.navigation", value: 1, layer: "run", origin: "argument" },
        details: { waited_for: "calls_in_flight" },
      });
      expect(engine.written).toEqual([expect.objectContaining({ id: 0 })]);
    },
  );

  test("sends a call queued for part of its own timeout with what is left of it as its timeout", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(1);
    const first = channel.call("sample.echo", {});
    const queued = channel.call(
      "page.goto",
      { url: "https://example.test", timeout: 1000 },
      { deadline: { key: "timeout.navigation", value: 1000 } },
    );

    await new Promise((resolve) => setTimeout(resolve, 40));
    engine.send({ id: 0, result: "first" });
    await first;
    await expect.poll(() => engine.written).toHaveLength(2);
    engine.send({ id: 1, result: "navigated" });
    await queued;
    const sent = engine.written[1] as { params: { url: string; timeout: number } };

    expect(sent.params.url).toBe("https://example.test");
    expect(sent.params.timeout).toBeGreaterThanOrEqual(1);
    expect(sent.params.timeout).toBeLessThanOrEqual(960);
  });

  test(
    "fails a queued call that gives no timeout of its own at its setting's value, as hello's result resolved it",
    { tags: ["error:deadline_exceeded"] },
    async () => {
      const engine = playedEngine();
      const channel = new Channel(engine.transport, owner);
      channel.capCallsInFlight(1);
      channel.boundByTimeoutSettings([
        { key: "timeout.navigation", value: 5, layer: "user_configuration" },
        { key: "timeout.wait", value: null, layer: "run", origin: "settings" },
      ]);
      const first = channel.call("sample.echo", {});

      const error = await failure(
        channel.call(
          "page.goto",
          {},
          { deadline: { key: "timeout.navigation", value: undefined } },
        ),
      );
      engine.send({ id: 0, result: "first" });
      await first;
      await settled();

      expect(error).toMatchObject({
        code: "deadline_exceeded",
        limit: { key: "timeout.navigation", value: 5, layer: "user_configuration" },
        details: { waited_for: "calls_in_flight" },
      });
      expect(engine.written).toEqual([expect.objectContaining({ id: 0 })]);
    },
  );

  test("keeps a queued call waiting whose setting resolved no timeout", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(1);
    channel.boundByTimeoutSettings([
      { key: "timeout.wait", value: null, layer: "run", origin: "settings" },
    ]);
    const first = channel.call("sample.echo", {});
    const queued = channel.call(
      "page.wait_for_load_state",
      {},
      { deadline: { key: "timeout.wait", value: undefined } },
    );

    await new Promise((resolve) => setTimeout(resolve, 20));
    engine.send({ id: 0, result: "first" });
    await first;
    await expect.poll(() => engine.written).toHaveLength(2);
    engine.send({ id: 1, result: "loaded" });

    await expect(queued).resolves.toBe("loaded");
    expect(engine.written[1]).toMatchObject({ params: {} });
  });

  test("keeps a queued call whose timeout is past the longest timer waiting, rather than failing it at once", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(1);
    const first = channel.call("sample.echo", {});
    const thirtyDays = 30 * 24 * 60 * 60 * 1000;
    let failed = false;
    const queued = channel.call(
      "run.finish",
      {},
      { deadline: { key: "timeout.run", value: thirtyDays } },
    );
    queued.catch(() => {
      failed = true;
    });

    await new Promise((resolve) => setTimeout(resolve, 20));
    const failedWhileQueued = failed;
    engine.send({ id: 0, result: "first" });
    await first;
    await expect.poll(() => engine.written).toHaveLength(2);
    engine.send({ id: 1, result: "finished" });

    expect(failedWhileQueued).toBe(false);
    await expect(queued).resolves.toBe("finished");
  });

  test("never sends a queued call its signal aborts, and sends the next one in its place", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(1);
    const first = channel.call("sample.echo", {});
    const aborting = new AbortController();
    const aborted = failure(channel.call("sample.echo", {}, { signal: aborting.signal }));
    const next = channel.call("sample.echo", {});

    aborting.abort(new CallCancelledError("the program cancelled the call"));
    const error = await aborted;
    engine.send({ id: 0, result: "first" });
    await first;
    await expect.poll(() => engine.written).toHaveLength(2);
    engine.send({ id: 2, result: "next" });

    expect(error).toMatchObject({ code: "call_cancelled" });
    expect(await next).toBe("next");
    expect(engine.written).toEqual([
      expect.objectContaining({ id: 0 }),
      expect.objectContaining({ id: 2 }),
    ]);
  });

  test("sends a queued stream's credits right after the call itself, never before", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(1);
    const first = channel.call("sample.echo", {});
    const { id } = channel.open("events.subscribe", {});
    channel.notify("$/credit", { id, credits: 8 });
    await settled();
    const beforeAPlaceFrees = engine.written.length;

    engine.send({ id: 0, result: "first" });
    await first;
    await expect.poll(() => engine.written).toHaveLength(3);

    expect(beforeAPlaceFrees).toBe(1);
    expect(engine.written.slice(1)).toEqual([
      expect.objectContaining({ id: 1, method: "events.subscribe" }),
      expect.objectContaining({ method: "$/credit", params: { id: 1, credits: 8 } }),
    ]);
  });

  test("holds a sent call's place until the engine answers its $/cancel", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    channel.capCallsInFlight(1);
    const aborting = new AbortController();
    const cancelled = failure(channel.call("sample.echo", {}, { signal: aborting.signal }));
    await settled();
    aborting.abort(new CallCancelledError("the program cancelled the call"));
    await cancelled;
    const next = channel.call("sample.echo", {});
    await settled();
    const beforeTheEnginesAnswer = engine.written.length;

    engine.send({
      id: 0,
      error: { code: -32006, message: "cancelled", data: { code: "call_cancelled" } },
    });
    await expect.poll(() => engine.written).toHaveLength(3);
    engine.send({ id: 1, result: "next" });

    expect(beforeTheEnginesAnswer).toBe(2);
    expect(await next).toBe("next");
    expect(engine.written[2]).toMatchObject({ id: 1 });
  });

  test(
    "counts queued calls against its bound on calls waiting, refusing the next at once",
    { tags: ["error:connection_overloaded"] },
    async () => {
      const engine = playedEngine();
      const channel = new Channel(engine.transport, owner, { maxPendingCalls: 2 });
      channel.capCallsInFlight(1);
      void channel.call("sample.echo", {});
      void channel.call("sample.echo", {});

      const error = await failure(channel.call("sample.echo", {}));
      await settled();

      expect(error).toMatchObject({
        code: "connection_overloaded",
        details: { bound: "pending_calls", value: 2 },
      });
      expect(engine.written).toHaveLength(1);
    },
  );
});

/** A Channel to `engine` whose owner serves `crawl_request.handle` with `served`, and no other method. */
function servingChannel(
  engine: PlayedEngine,
  served: (params: unknown, signal: AbortSignal) => Promise<unknown>,
): Channel {
  return new Channel(engine.transport, {
    ...owner,
    serve: (method, params, signal) =>
      method === "crawl_request.handle" ? served(params, signal) : undefined,
  });
}

describe("a Channel serving the engine's requests", () => {
  test("answers a request with what its owner serves, and a method it does not serve as JSON-RPC's unknown method", async () => {
    const engine = playedEngine();
    servingChannel(engine, (params) => Promise.resolve({ handled: params }));

    engine.send({ id: 1, method: "crawl_request.handle", params: { attempt: 1 } });
    await expect
      .poll(() => engine.written)
      .toEqual([{ jsonrpc: "2.0", id: 1, result: { handled: { attempt: 1 } } }]);
    engine.send({ id: 2, method: "route.handle", params: {} });

    await expect.poll(() => engine.written).toHaveLength(2);
    expect(engine.written[1]).toEqual({
      jsonrpc: "2.0",
      id: 2,
      error: { code: -32601, message: "this Client serves no `route.handle`" },
    });
  });

  test("answers a request whose serving failed with the error's wire class, code, fix and details", async () => {
    const engine = playedEngine();
    const failures = [
      new RunNotFoundError("no Run has this Run ID", {
        fix: "Start the Run first.",
        details: { run_id: "nightly" },
      }),
      new TypeError("a bug"),
    ];
    servingChannel(engine, () => Promise.reject(failures.shift()));

    engine.send({ id: 1, method: "crawl_request.handle", params: {} });
    await expect.poll(() => engine.written).toHaveLength(1);
    engine.send({ id: 2, method: "crawl_request.handle", params: {} });
    await expect.poll(() => engine.written).toHaveLength(2);

    expect(engine.written).toEqual([
      {
        jsonrpc: "2.0",
        id: 1,
        error: {
          code: -32001,
          message: "no Run has this Run ID",
          data: {
            code: "run_not_found",
            retryable: false,
            fix: "Start the Run first.",
            run_id: "nightly",
          },
        },
      },
      {
        jsonrpc: "2.0",
        id: 2,
        error: { code: -32007, message: "a bug", data: { code: "internal", retryable: false } },
      },
    ]);
  });

  test(
    "stops a request the engine cancels, answers it call_cancelled, and sends nothing when its serving ends after",
    { tags: ["error:call_cancelled"] },
    async () => {
      const engine = playedEngine();
      const signals: AbortSignal[] = [];
      let finish: ((result: unknown) => void) | undefined;
      servingChannel(
        engine,
        (_params, signal) =>
          new Promise((resolve) => {
            signals.push(signal);
            finish = resolve;
          }),
      );

      engine.send({ id: 0, method: "crawl_request.handle", params: {} });
      await expect.poll(() => signals).toHaveLength(1);
      engine.send({ method: "$/cancel", params: { id: 0 } });
      await expect.poll(() => engine.written).toHaveLength(1);
      finish?.({});
      await settled();

      expect({ reason: signals[0]?.reason, written: engine.written }).toEqual({
        reason: expect.objectContaining({ code: "call_cancelled", details: { by: "client" } }),
        written: [
          {
            jsonrpc: "2.0",
            id: 0,
            error: {
              code: -32006,
              message: "the engine cancelled its request",
              data: { code: "call_cancelled", retryable: false, by: "client" },
            },
          },
        ],
      });
    },
  );

  test("stops what serves a request once the engine's side ends", async () => {
    const engine = playedEngine();
    const signals: AbortSignal[] = [];
    const channel = servingChannel(engine, (_params, signal) => {
      signals.push(signal);
      return new Promise(() => undefined);
    });

    engine.send({ id: 0, method: "crawl_request.handle", params: {} });
    await expect.poll(() => signals).toHaveLength(1);
    engine.end();
    await channel.ended;

    expect(signals[0]?.reason).toMatchObject({ code: "closed" });
  });
});

/**
 * A Channel over `first` that reaches the engine again over each of `links`
 * in turn, then over none, greeting each with `hello`.
 */
function relinkingChannel(
  first: PlayedEngine,
  links: PlayedEngine[],
  within: number | null = 30_000,
): { channel: Channel; reopened: () => number; closing: AbortController } {
  let reopened = 0;
  const closing = new AbortController();
  const relinking: ChannelRelinking = {
    // `run.new_page` stands for an Effect, every other operation for a read.
    resends: (method) => method !== "run.new_page",
    within,
    signal: closing.signal,
    reopen: () => {
      reopened += 1;
      const link = links.shift();
      return link === undefined
        ? Promise.reject(new Error("connection refused"))
        : Promise.resolve(link.transport);
    },
    greet: async (call) => {
      await call("hello", { protocol_version: "1" });
    },
    // Not the owner's `closed`, so a test tells which of the two ended the Channel by its code.
    gaveUp: (cause) =>
      new ConnectionLostError("reaching the engine again failed within timeout.connect", {
        cause,
      }),
  };
  const channel = new Channel(first.transport, owner);
  channel.relinkWith(relinking);
  return { channel, reopened: () => reopened, closing };
}

/** Answers the `hello` the Client sent `engine`. */
function answerHello(engine: PlayedEngine): void {
  const hello = engine.written.find(
    (message): message is { id: number } =>
      typeof message === "object" &&
      message !== null &&
      "method" in message &&
      message.method === "hello",
  );
  if (hello === undefined) {
    throw new Error("the Client sent no hello");
  }
  engine.send({ id: hello.id, result: {} });
}

describe("a Channel that reconnects after its link dropped", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  test(
    "sends a read in flight again by its id once the new link is greeted, then the calls made meanwhile",
    { tags: ["story:11-6", "story:11-32"] },
    async () => {
      const [first, second] = [playedEngine(), playedEngine()];
      const { channel, reopened } = relinkingChannel(first, [second]);
      const read = channel.call("run.report", { run_id: "nightly" });
      await settled();

      first.end();
      await settled();
      const meanwhile = channel.call("run.list", {});
      await vi.advanceTimersByTimeAsync(249);
      const beforeTheDelay = reopened();
      await vi.advanceTimersByTimeAsync(1);
      await settled();
      const greeting = [...second.written];
      answerHello(second);
      await settled();
      second.send({ id: 0, result: { report: "done" } });
      second.send({ id: 1, result: { runs: [] } });

      expect({
        beforeTheDelay,
        greeting,
        written: second.written,
        answers: await Promise.all([read, meanwhile]),
      }).toEqual({
        beforeTheDelay: 0,
        greeting: [{ jsonrpc: "2.0", id: 2, method: "hello", params: { protocol_version: "1" } }],
        written: [
          { jsonrpc: "2.0", id: 2, method: "hello", params: { protocol_version: "1" } },
          { jsonrpc: "2.0", id: 0, method: "run.report", params: { run_id: "nightly" } },
          { jsonrpc: "2.0", id: 1, method: "run.list", params: {} },
        ],
        answers: [{ report: "done" }, { runs: [] }],
      });
    },
  );

  test(
    "fails an Effect in flight with connection_lost, never sending it again",
    { tags: ["story:11-32"] },
    async () => {
      const [first, second] = [playedEngine(), playedEngine()];
      const { channel } = relinkingChannel(first, [second]);
      const effect = failure(channel.call("run.new_page", { run_id: "nightly" }));
      await settled();

      first.end();
      const error = await effect;
      await vi.advanceTimersByTimeAsync(250);
      answerHello(second);
      await settled();

      expect({
        lost: error instanceof ConnectionLostError,
        error,
        written: second.written,
      }).toEqual({
        lost: true,
        error: expect.objectContaining({
          code: "connection_lost",
          group: "unavailable",
          operation: "run.new_page",
        }),
        written: [{ jsonrpc: "2.0", id: 1, method: "hello", params: { protocol_version: "1" } }],
      });
    },
  );

  test(
    "fails a stream it cannot resume with connection_lost, never sending it again",
    { tags: ["story:11-32"] },
    async () => {
      const [first, second] = [playedEngine(), playedEngine()];
      const { channel } = relinkingChannel(first, [second]);
      const stream = failure(
        channel.open("export.stream", {}, { notified: () => undefined }).result,
      );
      await settled();

      first.end();
      const error = await stream;
      await vi.advanceTimersByTimeAsync(250);
      answerHello(second);
      await settled();

      expect({ error, written: second.written }).toEqual({
        error: expect.objectContaining({ code: "connection_lost", operation: "export.stream" }),
        written: [{ jsonrpc: "2.0", id: 1, method: "hello", params: { protocol_version: "1" } }],
      });
    },
  );

  test(
    "resubscribes a stream from where it was, with the credit it still had",
    { tags: ["story:11-6"] },
    async () => {
      const [first, second] = [playedEngine(), playedEngine()];
      const { channel } = relinkingChannel(first, [second]);
      const events: unknown[] = [];
      const stream = channel.open(
        "events.subscribe",
        {},
        {
          notified: (_method, params) => events.push(params),
          resume: () => ({ params: { after: { engine_instance: "a", position: 2 } }, credits: 14 }),
        },
      );
      channel.notify("$/credit", { id: stream.id, credits: 16 });
      await settled();

      first.end();
      await vi.advanceTimersByTimeAsync(250);
      answerHello(second);
      await settled();
      second.send({ method: "$/events", params: { id: 0, events: [{ position: 3 }] } });
      await settled();

      expect({ written: second.written.slice(1), events }).toEqual({
        written: [
          {
            jsonrpc: "2.0",
            id: 0,
            method: "events.subscribe",
            params: { after: { engine_instance: "a", position: 2 } },
          },
          { jsonrpc: "2.0", method: "$/credit", params: { id: 0, credits: 14 } },
        ],
        events: [{ id: 0, events: [{ position: 3 }] }],
      });
    },
  );

  test(
    "gives up with its owner's error once the next attempt would start past timeout.connect",
    { tags: ["story:11-6"] },
    async () => {
      const first = playedEngine();
      const { channel, reopened } = relinkingChannel(first, [], 1_000);
      const read = failure(channel.call("run.report", {}));
      await settled();

      first.end();
      await vi.advanceTimersByTimeAsync(750);
      const error = await read;

      // Attempts start at 250 ms and 750 ms; the next would start at 1,750 ms.
      expect({ reopened: reopened(), error }).toEqual({
        reopened: 2,
        error: expect.objectContaining({
          code: "connection_lost",
          operation: "run.report",
          cause: new Error("connection refused"),
        }),
      });
    },
  );

  test(
    "ends as its owner says once it is closed while it waits to reach the engine again",
    { tags: ["story:11-6"] },
    async () => {
      const first = playedEngine();
      const { channel, reopened, closing } = relinkingChannel(first, [playedEngine()]);
      const read = failure(channel.call("run.report", {}));
      await settled();

      first.end();
      await settled();
      closing.abort();
      await channel.ended;

      expect({ reopened: reopened(), error: await read }).toEqual({
        reopened: 0,
        error: expect.objectContaining({ code: "closed", operation: "run.report" }),
      });
    },
  );

  test(
    "ends as its owner says once the new link sends a line that is no message, reaching for no other link",
    { tags: ["story:11-6"] },
    async () => {
      const [first, second] = [playedEngine(), playedEngine()];
      const { channel, reopened } = relinkingChannel(first, [second, playedEngine()]);
      const read = failure(channel.call("run.report", {}));
      await settled();

      first.end();
      await vi.advanceTimersByTimeAsync(250);
      second.sendLine("no message");
      await settled();
      await vi.advanceTimersByTimeAsync(10_000);
      await channel.ended;

      expect({ reopened: reopened(), error: await read }).toEqual({
        reopened: 1,
        error: expect.objectContaining({ code: "closed", operation: "run.report" }),
      });
    },
  );

  test(
    "fails a call it holds while it reaches the engine again once the call's own timeout passes, never sending it",
    { tags: ["story:11-6", "error:deadline_exceeded"] },
    async () => {
      const first = playedEngine();
      const { channel } = relinkingChannel(first, []);
      const deadline = { key: "timeout", value: 1_000 };
      const inFlight = failure(channel.call("run.report", {}, { deadline }));
      await settled();

      first.end();
      await settled();
      const meanwhile = failure(channel.call("run.list", {}, { deadline }));
      await vi.advanceTimersByTimeAsync(1_000);

      const held = {
        code: "deadline_exceeded",
        limit: { key: "timeout", value: 1_000, layer: "run", origin: "argument" },
        details: expect.objectContaining({ waited_for: "hello" }),
      };
      expect(await Promise.all([inFlight, meanwhile])).toEqual([
        expect.objectContaining({ ...held, operation: "run.report" }),
        expect.objectContaining({ ...held, operation: "run.list" }),
      ]);
    },
  );
});
