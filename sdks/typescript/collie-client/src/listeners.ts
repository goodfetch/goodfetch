// A listener's subscription (ADR 0081): its `on` call is sent when `on` is
// called and stays in flight, and the engine sends the event on the call
// itself, in `$/events` batches, never more than the credits granted. The
// listener gets the events one at a time, in order, and an awaitable it
// returns is awaited before the next; a credit goes back only once it
// settled, so a slow listener slows a lossless producer. One that throws is
// removed: its call is cancelled and its value hears the failure, which
// nothing here lets reach the host as an unhandled rejection.
import type { Stream } from "./client-connection.ts";
import type { Listener, Subscription } from "./handle.ts";
import type { ChannelOpenOptions } from "./rpc.ts";
import { batchOf, type StreamedEvent } from "./stream.ts";

/** The credits a subscription grants when its call is sent: at most this many events wait for its listener. */
const CREDITS = 16;

/** How many events its listener gets through before the subscription grants as many again. */
const REGRANT = 8;

/** What a subscription listens to, and what it tells its value. */
export interface Listening<Payload> {
  /** The event its listener gets, such as `page.console`; other events of the call are skipped. */
  readonly event: string;
  readonly listener: Listener<Payload>;
  /** Hears, once, what the listener threw, after it was removed for it. */
  failed(cause: unknown): void;
  /** Hears the error that ended the call, such as `page_events_lost`, when the subscription did not. */
  ended(error: unknown): void;
}

/**
 * Sends the call `open` opens and delivers its events to `listening`'s
 * listener; the `Subscription` returned cancels it.
 */
export function subscribe<Payload>(
  open: (options: ChannelOpenOptions) => Stream,
  listening: Listening<Payload>,
): Subscription {
  const cancel = new AbortController();
  let waiting: StreamedEvent[] = [];
  let delivering = false;
  let delivered = 0;
  const call = open({
    signal: cancel.signal,
    notified: (method, params) => {
      if (method !== "$/events" || cancel.signal.aborted) {
        return;
      }
      waiting.push(...batchOf(params));
      void deliver();
    },
  });
  call.result.then(
    () => undefined,
    (error: unknown) => {
      if (!cancel.signal.aborted) {
        listening.ended(error);
      }
    },
  );
  call.grant(CREDITS);

  // Never rejects: what the listener throws is caught, and the rest cannot.
  async function deliver(): Promise<void> {
    if (delivering) {
      return;
    }
    delivering = true;
    try {
      for (
        let next = waiting.shift();
        next !== undefined && !cancel.signal.aborted;
        next = waiting.shift()
      ) {
        if (next.event === listening.event) {
          try {
            // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine payloads are not validated beyond `hello`'s protocol version (STYLE.md): the event's is the catalog's.
            await listening.listener(next.data as Payload, next.dropped ?? 0);
          } catch (error) {
            close();
            listening.failed(error);
            return;
          }
        }
        delivered += 1;
        if (delivered === REGRANT) {
          delivered = 0;
          call.grant(REGRANT);
        }
      }
    } finally {
      delivering = false;
    }
  }

  function close(): void {
    if (!cancel.signal.aborted) {
      waiting = [];
      cancel.abort();
    }
  }

  return { close, [Symbol.dispose]: close };
}
