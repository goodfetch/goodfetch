// A sequence's iterator (ADR 0081) on a stream faked here, which records the
// credits granted and sends notifications as the engine does: it subscribes
// when made, reads in order, and leaving it any way (`break`, `return()`,
// `close()`, `Symbol.dispose`) cancels its stream at once.
import { describe, expect, test } from "vitest";
import type { Stream } from "./client-connection.ts";
import type { ChannelOpenOptions } from "./rpc.ts";
import { Sequence } from "./sequence.ts";

/** A sequence of numbers on a faked stream, and the engine's side of it. */
function sequenced(signal?: AbortSignal) {
  const granted: number[] = [];
  let opened = 0;
  let given: ChannelOpenOptions | undefined;
  let answer: { resolve(value: unknown): void; reject(error: unknown): void } | undefined;
  const result = new Promise<unknown>((resolve, reject) => {
    answer = { resolve, reject };
  });
  const open = (options: ChannelOpenOptions): Stream => {
    opened += 1;
    given = options;
    // As the Channel does, a cancelled call rejects with its signal's reason.
    options.signal?.addEventListener("abort", () => answer?.reject(options.signal?.reason));
    return { result, grant: (credits) => granted.push(credits) };
  };
  const sequence = new Sequence<number>({
    open,
    itemOf: (method, params) =>
      method === "$/item" && typeof params === "number" ? params : undefined,
    credits: 4,
    regrant: 2,
    signal,
  });
  return {
    sequence,
    granted,
    opened: (): number => opened,
    /** Sends one notification per item. */
    send(...items: number[]): void {
      for (const item of items) given?.notified?.("$/item", item);
    },
    cancelled: (): boolean => given?.signal?.aborted === true,
    end: (): void => answer?.resolve({}),
    fail: (error: unknown): void => answer?.reject(error),
  };
}

describe("a sequence's iterator", () => {
  test(
    "subscribes when made and reads its items in order, granting credits as they are read",
    { tags: ["story:11-62"] },
    async () => {
      const stream = sequenced();
      expect(stream.opened()).toBe(1);
      expect(stream.granted).toEqual([4]);

      stream.send(1, 2, 3);
      const read = [
        await stream.sequence.next(),
        await stream.sequence.next(),
        await stream.sequence.next(),
      ].map((result) => result.value);
      expect(stream.granted).toEqual([4, 2]);
      stream.end();

      expect(read).toEqual([1, 2, 3]);
      expect(await stream.sequence.next()).toEqual({ done: true, value: undefined });
      expect(stream.cancelled()).toBe(false);
    },
  );

  test(
    "fails with the error that ended its stream once the items before it are read",
    { tags: ["story:11-62"] },
    async () => {
      const stream = sequenced();
      const lost = new Error("events_lost");

      stream.send(1);
      stream.fail(lost);

      expect(await stream.sequence.next()).toEqual({ done: false, value: 1 });
      await expect(stream.sequence.next()).rejects.toBe(lost);
    },
  );

  test.for([
    [
      "`break`",
      async (sequence: Sequence<number>): Promise<void> => {
        for await (const _ of sequence) break;
      },
    ],
    [
      "`return()`",
      async (sequence: Sequence<number>): Promise<void> => {
        await sequence.next();
        await sequence.return();
      },
    ],
    [
      "`close()`",
      async (sequence: Sequence<number>): Promise<void> => {
        await sequence.next();
        sequence.close();
      },
    ],
    [
      "`Symbol.dispose`",
      async (sequence: Sequence<number>): Promise<void> => {
        using reading = sequence;
        await reading.next();
      },
    ],
  ] as const)(
    "unsubscribes on %s: its stream is cancelled and the items still waiting are never read",
    { tags: ["story:11-62"] },
    async ([, leave]) => {
      const stream = sequenced();
      stream.send(1, 2);

      await leave(stream.sequence);

      expect(stream.cancelled()).toBe(true);
      expect(await stream.sequence.next()).toEqual({ done: true, value: undefined });
    },
  );

  test(
    "closes synchronously, ending a read still waiting for an item",
    { tags: ["story:11-62"] },
    async () => {
      const stream = sequenced();
      const waiting = stream.sequence.next();

      stream.sequence.close();

      expect(stream.cancelled()).toBe(true);
      expect(await waiting).toEqual({ done: true, value: undefined });
    },
  );

  test(
    "fails with its signal's reason once the signal aborts, leaving the items already received unread",
    { tags: ["story:11-62"] },
    async () => {
      const stop = new AbortController();
      const stream = sequenced(stop.signal);
      stream.send(1, 2);

      expect(await stream.sequence.next()).toEqual({ done: false, value: 1 });
      stop.abort(new Error("read enough"));

      await expect(stream.sequence.next()).rejects.toThrow("read enough");
      expect(stream.cancelled()).toBe(true);
    },
  );
});
