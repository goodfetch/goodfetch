// A listener's subscription (ADR 0081): its `on` call is sent when `on` is
// called and stays in flight. A page's events come on that call itself, in
// `$/events` batches, never more than the credits granted; a Session's come
// on the Connection's own Subscription, which every such listener shares,
// and its call only arms the listener in the engine. The listener gets the
// events one at a time, in order, and an awaitable it returns is awaited
// before the next. On its own call, a credit goes back only once it
// settled, so a slow listener slows a lossless producer; on the shared
// Subscription, its own queue takes each event and gives its credit back at
// once, so a slow listener slows no other, and one whose queue is full is
// removed once it got what it held, with `listener_events_lost`. One that
// throws is removed: its call is cancelled and its value hears the failure,
// which nothing here lets reach the host as an unhandled rejection. The
// events of a value an event opened, such as a page's WebSocket, go to
// that value's own listeners (`opened.ts`), in the same order.
import type { Stream } from "./client-connection.ts";
import { ListenerEventsLostError } from "./generated/errors.ts";
import type { Listener, Subscription } from "./handle.ts";
import type { Opened } from "./opened.ts";
import type { ChannelOpenOptions } from "./rpc.ts";
import type { SharedSubscription } from "./shared-subscription.ts";
import { batchOf, type StreamedEvent } from "./stream.ts";

/** The credits a subscription grants when its call is sent: at most this many events wait for its listener. */
export const CREDITS = 16;

/** How many events its listener gets through before the subscription grants as many again. */
export const REGRANT = 8;

/** How many events a listener of published events holds in its own queue when `maxListenerQueue` is left out: as many as the engine holds for a `page.on` call. */
export const MAX_LISTENER_QUEUE = 1_024;

/** What a subscription listens to, and what it tells its value. */
export interface Listening<Payload> {
  /** The event its listener gets, such as `page.console`; other events of the call are skipped. */
  readonly event: string;
  readonly listener: Listener<Payload>;
  /** The values `event` opens, whose own events the call also carries, when it opens one. */
  readonly opened?: Opened | undefined;
  /** Hears, once, what the listener threw, after it was removed for it. */
  failed(cause: unknown): void;
  /** Hears the error that ended the call, such as `page_events_lost`, when the subscription did not. */
  ended(error: unknown): void;
}

/** An event waiting for a listener, and what hears that the listener is through with it. */
interface Waiting {
  readonly event: StreamedEvent;
  readonly settled: () => void;
}

/**
 * Hands the events of `listening.event` it takes to `listening`'s
 * listener, and those of the values it opened to theirs, one at a time,
 * in order, each awaited before the next. It
 * settles every event it takes, of that name or not: once the listener is
 * through with it, or once the delivery stops, even while the listener
 * still awaits it.
 */
export class Delivery<Payload> {
  readonly #listening: Listening<Payload>;
  /** Hears, before `listening.failed`, that the listener threw. */
  readonly #threw: () => void;
  #waiting: Waiting[] = [];
  /** Settles the event the listener has, if it has one. */
  #delivered: (() => void) | undefined;
  #delivering = false;
  #stopped = false;

  constructor(listening: Listening<Payload>, threw: () => void) {
    this.#listening = listening;
    this.#threw = threw;
  }

  /** Takes `event`; `settled` hears once the listener is through with it. */
  take(event: StreamedEvent, settled: () => void): void {
    if (this.#stopped) {
      settled();
      return;
    }
    this.#waiting.push({ event, settled });
    void this.#deliver();
  }

  /** Delivers nothing more: each event still waiting, or still with the listener, is settled. */
  stop(): void {
    this.#stopped = true;
    this.#delivered?.();
    const dropped = this.#waiting;
    this.#waiting = [];
    for (const waiting of dropped) {
      waiting.settled();
    }
  }

  // Never rejects: what the listener throws is caught, and the rest cannot.
  async #deliver(): Promise<void> {
    if (this.#delivering) {
      return;
    }
    this.#delivering = true;
    try {
      for (
        let next = this.#waiting.shift();
        next !== undefined && !this.#stopped;
        next = this.#waiting.shift()
      ) {
        const delivered = once(next.settled);
        this.#delivered = delivered;
        try {
          if (next.event.event === this.#listening.event) {
            // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine payloads are not validated beyond `hello`'s protocol version (STYLE.md): the event's is the catalog's.
            await this.#listening.listener(next.event.data as Payload, next.event.dropped ?? 0);
          } else if (this.#listening.opened?.takes(next.event) === true) {
            await this.#listening.opened.deliver(next.event);
          }
        } catch (error) {
          this.#threw();
          this.stop();
          this.#listening.failed(error);
          return;
        } finally {
          this.#delivered = undefined;
          delivered();
        }
      }
    } finally {
      this.#delivering = false;
    }
  }
}

/**
 * What counts the events a call's listeners settled, and grants `call`
 * as many credits again every `REGRANT`, until `signal` aborts.
 */
export function regranting(call: Pick<Stream, "grant">, signal: AbortSignal): () => void {
  let settled = 0;
  return () => {
    settled += 1;
    if (settled === REGRANT && !signal.aborted) {
      settled = 0;
      call.grant(REGRANT);
    }
  };
}

/** `run`, which does nothing when called again. */
function once(run: () => void): () => void {
  let ran = false;
  return () => {
    if (!ran) {
      ran = true;
      run();
    }
  };
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
  const delivery = new Delivery(listening, close);
  const call = open({
    signal: cancel.signal,
    notified: (method, params) => {
      if (method !== "$/events" || cancel.signal.aborted) {
        return;
      }
      for (const event of batchOf(params)) {
        delivery.take(event, regrant);
      }
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
  const regrant = regranting(call, cancel.signal);
  call.grant(CREDITS);

  function close(): void {
    if (!cancel.signal.aborted) {
      cancel.abort();
      delivery.stop();
    }
  }

  return { close, [Symbol.dispose]: close };
}

/**
 * Registers `listening` on `shared`, the Connection's Subscription, for
 * the events `mine` says are its value's, then sends the call `open` opens,
 * which arms it in the engine; the `Subscription` returned cancels that
 * call and leaves `shared`. The listener's own queue holds at most
 * `shared.listenerQueue` events, the one it has included, and gives each
 * event's credit back as it takes it. Past that bound, it loses each event
 * it would take until it got those it held, then it is removed, and
 * `listening` hears `listener_events_lost` naming how many it lost.
 */
export function subscribePublished<Payload>(
  open: (options: ChannelOpenOptions) => Stream,
  shared: SharedSubscription,
  mine: (data: unknown) => boolean,
  listening: Listening<Payload>,
): Subscription {
  const cancel = new AbortController();
  const delivery = new Delivery(listening, close);
  const bound = shared.listenerQueue;
  /** The events its queue holds, the one its listener has included. */
  let held = 0;
  /** The events it lost since its queue was full. */
  let lost = 0;
  // First, so that the Subscription is in flight before the engine arms the listener.
  const leave = shared.join({
    takes: (event) => event.event === listening.event && mine(event.data),
    take: (event) => {
      if (lost > 0 || held >= bound) {
        lost += 1;
        return;
      }
      held += 1;
      delivery.take(event, () => {
        held -= 1;
        if (held === 0 && lost > 0) {
          lostEvents();
        }
      });
    },
    ended: (failure) => {
      close();
      if (failure !== undefined) {
        listening.ended(failure.error);
      }
    },
  });
  open({ signal: cancel.signal }).result.then(
    // An `on` that answered, as when its Session closed, arms nothing more;
    // what the listener already took still reaches it.
    leave,
    (error: unknown) => {
      if (!cancel.signal.aborted) {
        close();
        listening.ended(error);
      }
    },
  );

  function close(): void {
    if (!cancel.signal.aborted) {
      cancel.abort();
      // First, so that the events it settles grant nothing on a Subscription it cancels.
      leave();
      delivery.stop();
    }
  }

  /** The listener got what it held while its queue was full: it is removed, and hears what it lost. */
  function lostEvents(): void {
    if (cancel.signal.aborted) {
      return;
    }
    close();
    listening.ended(
      new ListenerEventsLostError(
        `the listener of \`${listening.event}\` held ${bound} events, as many as its queue takes, so it was removed and lost ${lost} more`,
        {
          fix: "Make the listener return sooner, or raise connect()'s maxListenerQueue, then listen again with `on`.",
          details: { event: listening.event, lost },
        },
      ),
    );
  }

  return { close, [Symbol.dispose]: close };
}
