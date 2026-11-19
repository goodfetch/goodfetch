// The Connection's own Subscription (ADRs 0031 and 0081): one
// `events.subscribe`, sent when the first listener whose events the engine
// publishes on its event stream joins, such as a Session's, and cancelled
// once the last leaves. Each event goes to the readers that take it, and its
// credit goes back once each of them is through with it, so a slow listener
// slows the stream; the engine then falls behind to `events.resync` rather
// than wait, and the events in between are gone, which is logged.
import type { Stream } from "./client-connection.ts";
import { CREDITS, once, regranting } from "./listeners.ts";
import type { Logger } from "./logger.ts";
import type { ChannelOpenOptions } from "./rpc.ts";
import { batchOf, type StreamedEvent } from "./stream.ts";

/** The event that says the engine no longer held the Subscription's position: the events since are gone. */
const RESYNC = "events.resync";

/** What reads the Connection's Subscription for one listener. */
export interface Reader {
  /** Whether it takes `event`, such as a page of its Session. */
  takes(event: StreamedEvent): boolean;
  /** Takes `event`; `settled` hears once its listener is through with it. */
  take(event: StreamedEvent, settled: () => void): void;
  /** Hears that the Subscription ended, which it no longer reads: with the error that ended it, if one did. */
  ended(failure: { readonly error: unknown } | undefined): void;
}

/** The Subscription in flight, and what cancels it. */
interface Open {
  readonly stream: Stream;
  readonly cancel: AbortController;
  /** Counts one event settled, granting more credits as they add up. */
  readonly settle: () => void;
}

/** One Connection's Subscription, shared by the listeners that read it. */
export class SharedSubscription {
  readonly #open: (options: ChannelOpenOptions) => Stream;
  readonly #logger: Logger | undefined;
  #readers = new Set<Reader>();
  #current: Open | undefined;

  /** `open` sends `events.subscribe`; `logger` hears that the stream fell behind. */
  constructor(open: (options: ChannelOpenOptions) => Stream, logger: Logger | undefined) {
    this.#open = open;
    this.#logger = logger;
  }

  /**
   * Adds `reader`, sending the Subscription with its credits when none is in
   * flight; the function returned removes it, and cancels the Subscription
   * once no reader is left. Removing it again changes nothing.
   */
  join(reader: Reader): () => void {
    this.#current ??= this.#subscribe();
    this.#readers.add(reader);
    return () => {
      if (this.#readers.delete(reader) && this.#readers.size === 0) {
        const current = this.#current;
        this.#current = undefined;
        current?.cancel.abort();
      }
    };
  }

  #subscribe(): Open {
    const cancel = new AbortController();
    const stream = this.#open({
      signal: cancel.signal,
      notified: (method, params) => {
        if (method === "$/events" && !cancel.signal.aborted) {
          this.#received(current, batchOf(params));
        }
      },
    });
    const current: Open = { stream, cancel, settle: regranting(stream, cancel.signal) };
    stream.result.then(
      () => this.#ended(current, undefined),
      (error: unknown) => this.#ended(current, { error }),
    );
    stream.grant(CREDITS);
    return current;
  }

  /** Hands each of `events` to the readers that take it, and its credit back once each is through with it. */
  #received(current: Open, events: readonly StreamedEvent[]): void {
    for (const event of events) {
      if (event.event === RESYNC) {
        this.#fellBehind();
      }
      const takers = [...this.#readers].filter((reader) => reader.takes(event));
      let waiting = takers.length;
      const settled = (): void => {
        waiting -= 1;
        if (waiting === 0) {
          current.settle();
        }
      };
      if (takers.length === 0) {
        current.settle();
      }
      for (const reader of takers) {
        reader.take(event, once(settled));
      }
    }
  }

  /** Logs that the engine no longer held the Subscription's position; a logger that throws loses it, since nothing else would say so. */
  #fellBehind(): void {
    try {
      this.#logger?.warn(
        "The Connection's Subscription fell behind: the events published since its last read are gone, so a Session's listener may have missed pages its `pages()` lists",
        { event: RESYNC },
      );
    } catch {
      // Nowhere else to say it.
    }
  }

  /** The Subscription ended without its cancel: each reader hears it, and the next to join sends another. */
  #ended(current: Open, failure: { readonly error: unknown } | undefined): void {
    if (current.cancel.signal.aborted) {
      return;
    }
    current.cancel.abort();
    if (this.#current === current) {
      this.#current = undefined;
    }
    const readers = this.#readers;
    this.#readers = new Set();
    for (const reader of readers) {
      reader.ended(failure);
    }
  }
}
