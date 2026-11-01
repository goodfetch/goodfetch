// The Channel over a transport the test plays the engine's side of: what it
// writes, the replies it reads in any order, and how a call that cannot be
// sent fails alone.
import { describe, expect, test } from "vitest";
import { encodeMessage } from "./framing.ts";
import { CallCancelledError, ClosedError } from "./generated/index.ts";
import { Channel, type ChannelOwner, type Transport } from "./rpc.ts";

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
    await settled();

    expect({ error, written: engine.written }).toEqual({
      error: expect.objectContaining({ code: "call_cancelled" }),
      written: [
        { jsonrpc: "2.0", id: 0, method: "run.abort", params: { run_id: "nightly" } },
        { jsonrpc: "2.0", method: "$/cancel", params: { id: 0 } },
      ],
    });
  });

  test("fails a call in flight with its owner's error once the engine's side ends", async () => {
    const engine = playedEngine();
    const channel = new Channel(engine.transport, owner);
    const call = failure(channel.call("run.start", {}));
    await settled();

    engine.end();

    expect(await call).toMatchObject({ code: "closed" });
  });
});
