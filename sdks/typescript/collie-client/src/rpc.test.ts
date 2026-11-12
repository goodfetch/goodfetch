// The Channel over a transport the test plays the engine's side of: what it
// writes, the replies it reads in any order, and how a call that cannot be
// sent fails alone.
import { describe, expect, test } from "vitest";
import { encodeMessage } from "./framing.ts";
import { CallCancelledError, ClosedError, ConnectionOverloadedError } from "./generated/index.ts";
import { Channel, type ChannelOwner, type Transport, type TransportEnd } from "./rpc.ts";

/** The engine's side of an in-memory transport: what the Client wrote, and what it reads next. */
interface PlayedEngine {
  readonly transport: Transport;
  /** Each message the Client wrote, parsed. */
  readonly written: unknown[];
  /** Sends the Client one message. */
  send(message: Readonly<Record<string, unknown>>): void;
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
    end: () => controller().close(),
  };
}

const owner: ChannelOwner = {
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
