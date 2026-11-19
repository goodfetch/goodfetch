// A sequence's iterator (ADR 0081): a stream read to its end, such as a
// Dataset's Records. It subscribes when made, its call sent at once, and
// reads the items in order, granting the engine credits as they are read,
// so an unread lossless sequence slows its producer. It is a plain class,
// not a generator, so it closes synchronously: `break`, `return()`,
// `close()` and `Symbol.dispose` cancel its stream at once, and so does
// releasing it unread. It has one reader: it is its own iterator.
import type { Stream } from "./client-connection.ts";
import { ClosedError } from "./generated/errors.ts";
import type { ChannelOpenOptions } from "./rpc.ts";

/** What a sequence reads, and how. */
export interface SequenceOptions<Item> {
  /** Opens its stream, which sends its call. */
  readonly open: (options: ChannelOpenOptions) => Stream;
  /** The item a notification of the stream carries, or `undefined` for another notification. */
  readonly itemOf: (method: string, params: unknown) => Item | undefined;
  /** How many items the engine may send before the program has read any. */
  readonly credits: number;
  /** How many items are read before it grants as many credits again. */
  readonly regrant: number;
  /** Stops reading: the stream is cancelled and the next read fails with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What the stream's notifications reach: never the iterator, so releasing it unread lets it go. */
interface Reading<Item> {
  received: Item[];
  ended: boolean;
  /** The reads waiting for an item or the stream's end. */
  waiting: (() => void)[];
}

/** Wakes every read `reading` keeps waiting. */
function wake(reading: Reading<unknown>): void {
  const waiting = reading.waiting;
  reading.waiting = [];
  for (const resolve of waiting) resolve();
}

/** Cancels the stream of a sequence released unread. */
const released = new FinalizationRegistry<AbortController>((stop) => {
  stop.abort(new ClosedError("the program released the sequence unread", {}));
});

/** A stream read to its end with `for await`, which closes at once however the reading stops. */
export class Sequence<Item> implements AsyncIterableIterator<Item, undefined>, Disposable {
  readonly #reading: Reading<Item> = { received: [], ended: false, waiting: [] };
  readonly #stop = new AbortController();
  readonly #stream: Stream;
  readonly #options: SequenceOptions<Item>;
  /** Items read since credits were last granted. */
  #read = 0;
  /** Whether the last read handed out an item, which the next read counts toward credits. */
  #handedOut = false;
  #closed = false;
  /** The last read asked for: each read starts once the one before it is answered. */
  #reads: Promise<unknown> = Promise.resolve();

  /** @internal A Product's package makes it, for the method that reads the sequence. */
  constructor(options: SequenceOptions<Item>) {
    this.#options = options;
    const reading = this.#reading;
    const signal =
      options.signal === undefined
        ? this.#stop.signal
        : AbortSignal.any([options.signal, this.#stop.signal]);
    this.#stream = options.open({
      signal,
      notified: (method, params) => {
        const item = options.itemOf(method, params);
        if (item !== undefined) {
          reading.received.push(item);
          wake(reading);
        }
      },
    });
    const settled = (): void => {
      reading.ended = true;
      wake(reading);
    };
    void this.#stream.result.then(settled, settled);
    this.#stream.grant(options.credits);
    released.register(this, this.#stop, this);
  }

  /** Reads the next item, or ends once the stream has ended and every item is read. */
  next(): Promise<IteratorResult<Item, undefined>> {
    const read = this.#reads.then(async () => await this.#next());
    this.#reads = read.catch(() => undefined);
    return read;
  }

  async #next(): Promise<IteratorResult<Item, undefined>> {
    const reading = this.#reading;
    if (this.#handedOut) {
      this.#handedOut = false;
      this.#read += 1;
      if (this.#read >= this.#options.regrant && !reading.ended && !this.#closed) {
        this.#stream.grant(this.#read);
        this.#read = 0;
      }
    }
    for (;;) {
      if (this.#closed) return { done: true, value: undefined };
      // Items already received are not read once the program stops reading.
      this.#options.signal?.throwIfAborted();
      const item = reading.received.shift();
      if (item !== undefined) {
        this.#handedOut = true;
        return { done: false, value: item };
      }
      if (reading.ended) {
        try {
          // Answers the stream's end, or fails with its error.
          await this.#stream.result;
        } finally {
          this.#finish();
        }
        return { done: true, value: undefined };
      }
      await new Promise<void>((resolve) => {
        reading.waiting.push(resolve);
      });
    }
  }

  /** Stops reading, as `break` does: the stream is cancelled. */
  async return(): Promise<IteratorResult<Item, undefined>> {
    this.close();
    return { done: true, value: undefined };
  }

  /** Stops reading at once: the stream is cancelled, and a read still waiting ends. Idempotent. */
  close(): void {
    if (this.#closed) return;
    const ended = this.#reading.ended;
    this.#finish();
    if (!ended) {
      this.#stop.abort(new ClosedError("the program stopped reading the sequence", {}));
    }
    wake(this.#reading);
  }

  [Symbol.dispose](): void {
    this.close();
  }

  [Symbol.asyncIterator](): this {
    return this;
  }

  #finish(): void {
    this.#closed = true;
    this.#reading.received = [];
    released.unregister(this);
  }
}
