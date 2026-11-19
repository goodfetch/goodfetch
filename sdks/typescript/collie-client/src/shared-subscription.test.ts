// The listeners whose events the engine publishes on its event stream, such
// as a Session's (ADR 0081), on calls faked here: the Connection's one
// Subscription is sent before the first listener's `on` and cancelled once
// the last leaves, each listener gets only its own value's events, and an
// event's credit goes back once each listener's own queue took it, so a
// stalled listener delays no other and, past its queue's bound, ends with
// `listener_events_lost` once it got the events it held.
import { describe, expect, test } from "vitest";
import type { Stream } from "./client-connection.ts";
import type { Listener } from "./handle.ts";
import { ListenerEventsLostError } from "./generated/errors.ts";
import { MAX_LISTENER_QUEUE, subscribePublished } from "./listeners.ts";
import type { Logger } from "./logger.ts";
import type { ChannelOpenOptions } from "./rpc.ts";
import { SharedSubscription } from "./shared-subscription.ts";
import type { StreamedEvent } from "./stream.ts";

/** A call faked here: what it was sent with, its grants, and how it answers. */
interface Faked {
  readonly method: string;
  readonly granted: number[];
  readonly options: ChannelOpenOptions;
  answer(): void;
  end(error: unknown): void;
  cancelled(): boolean;
}

/** The calls sent, in order, and what sends one named `method`. */
function engine() {
  const sent: Faked[] = [];
  const open =
    (method: string) =>
    (options: ChannelOpenOptions): Stream => {
      const granted: number[] = [];
      const { promise, resolve, reject } = Promise.withResolvers<unknown>();
      // As the Channel does, a cancelled call rejects with its signal's reason.
      options.signal?.addEventListener("abort", () => reject(options.signal?.reason));
      sent.push({
        method,
        granted,
        options,
        answer: () => resolve({}),
        end: reject,
        cancelled: () => options.signal?.aborted === true,
      });
      return { result: promise, grant: (credits) => granted.push(credits) };
    };
  return { sent, open };
}

/** Ignores what it is given. */
function ignored(): void {}

/** A logger that keeps its warnings. */
function warnings(): { readonly logger: Logger; readonly warned: string[] } {
  const warned: string[] = [];
  return {
    warned,
    logger: {
      debug: ignored,
      info: ignored,
      warn: (message) => warned.push(message),
      error: ignored,
    },
  };
}

/** A Connection's Subscription on faked calls, and what listens on it to a Session's `page`. */
function connection(logger?: Logger, listenerQueue = MAX_LISTENER_QUEUE) {
  const calls = engine();
  const shared = new SharedSubscription(calls.open("events.subscribe"), logger, listenerQueue);
  const failures: unknown[] = [];
  const endings: unknown[] = [];
  const listen = (session: string, listener: Listener<unknown>) =>
    subscribePublished(
      calls.open("session.on"),
      shared,
      (data) =>
        typeof data === "object" && data !== null && Reflect.get(data, "session_id") === session,
      {
        event: "session.page",
        listener,
        failed: (cause) => failures.push(cause),
        ended: (error) => endings.push(error),
      },
    );
  return {
    ...calls,
    listen,
    failures,
    endings,
    /** Sends one `$/events` batch on the Subscription in flight, the last sent. */
    publish(...events: StreamedEvent[]): void {
      calls.sent
        .findLast((call) => call.method === "events.subscribe")
        ?.options.notified?.("$/events", { id: 1, events });
    },
  };
}

let position = 0;

/** A page `page` opened in `session`, at the next position. */
function opened(page: string, session: string): StreamedEvent {
  position += 1;
  return { position, event: "session.page", data: { page_id: page, session_id: session } };
}

/** Lets every pending callback and promise run. */
function settled(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

describe("a listener of published events", () => {
  test(
    "shares one Subscription, sent with its credits before the first `on`, and gets only its own value's events",
    { tags: ["story:11-61", "story:11-62"] },
    async () => {
      const first: unknown[] = [];
      const second: unknown[] = [];
      const connected = connection();

      connected.listen("session-1", (page) => {
        first.push(page);
      });
      connected.listen("session-2", (page) => {
        second.push(page);
      });
      connected.publish(
        opened("page-1", "session-1"),
        opened("page-2", "session-2"),
        opened("page-9", "session-9"),
        { position: 99, event: "sample.echoed", data: { session_id: "session-1" } },
      );
      await settled();

      expect(connected.sent.map((call) => call.method)).toEqual([
        "events.subscribe",
        "session.on",
        "session.on",
      ]);
      expect(connected.sent[0]?.granted).toEqual([16]);
      expect(first).toEqual([{ page_id: "page-1", session_id: "session-1" }]);
      expect(second).toEqual([{ page_id: "page-2", session_id: "session-2" }]);
    },
  );

  test(
    "gives a stalled listener's events back as its own queue takes them, so another listener's pages are not delayed",
    { tags: ["story:11-63"] },
    async () => {
      const { promise: never } = Promise.withResolvers<void>();
      const other: unknown[] = [];
      const connected = connection();
      connected.listen("session-1", async () => {
        await never;
      });
      connected.listen("session-2", (page) => {
        other.push(page);
      });

      connected.publish(
        ...Array.from({ length: 16 }, (_, index) => opened(`page-${index}`, "session-1")),
        opened("page-other", "session-2"),
      );
      await settled();

      const [subscription] = connected.sent;
      expect(subscription?.granted).toEqual([16, 8, 8]);
      expect(other).toEqual([{ page_id: "page-other", session_id: "session-2" }]);
      expect(connected.endings).toEqual([]);
    },
  );

  test(
    "removes a listener whose queue is full once it got the events it held, ending it with `listener_events_lost` naming how many it lost",
    { tags: ["story:11-63", "error:listener_events_lost"] },
    async () => {
      const { promise: stalled, resolve: release } = Promise.withResolvers<void>();
      const seen: unknown[] = [];
      const other: unknown[] = [];
      const connected = connection(undefined, 3);
      connected.listen("session-1", async (page) => {
        seen.push(page);
        await stalled;
      });
      connected.listen("session-2", (page) => {
        other.push(page);
      });
      const [subscription, slowOn] = connected.sent;

      connected.publish(
        ...Array.from({ length: 5 }, (_, index) => opened(`page-${index}`, "session-1")),
      );
      await settled();
      connected.publish(opened("page-6", "session-1"), opened("page-other", "session-2"));
      await settled();
      expect(other).toHaveLength(1);
      expect(slowOn?.cancelled()).toBe(false);
      release();
      await settled();

      expect(seen.map((page) => Reflect.get(Object(page), "page_id"))).toEqual([
        "page-0",
        "page-1",
        "page-2",
      ]);
      expect(connected.endings).toHaveLength(1);
      const [ending] = connected.endings;
      expect(ending).toBeInstanceOf(ListenerEventsLostError);
      expect(ending).toMatchObject({
        code: "listener_events_lost",
        details: { event: "session.page", lost: 3 },
      });
      expect(slowOn?.cancelled()).toBe(true);
      expect(subscription?.cancelled()).toBe(false);
    },
  );

  test(
    "cancels the Subscription once its last listener leaves, settling the events it still held, and sends another for the next",
    { tags: ["story:11-62"] },
    async () => {
      const { promise: never } = Promise.withResolvers<void>();
      const connected = connection();
      const held = connected.listen("session-1", async () => {
        await never;
      });
      const other = connected.listen("session-2", () => {});
      connected.publish(
        ...Array.from({ length: 8 }, (_, index) => opened(`page-${index}`, "session-1")),
      );
      await settled();
      const [subscription, heldOn, otherOn] = connected.sent;

      held.close();
      expect(heldOn?.cancelled()).toBe(true);
      expect(subscription?.cancelled()).toBe(false);
      expect(subscription?.granted).toEqual([16, 8]);
      other.close();
      expect(otherOn?.cancelled()).toBe(true);
      expect(subscription?.cancelled()).toBe(true);
      connected.listen("session-1", () => {});

      expect(connected.sent.map((call) => call.method)).toEqual([
        "events.subscribe",
        "session.on",
        "session.on",
        "events.subscribe",
        "session.on",
      ]);
      await settled();
      expect(connected.endings).toEqual([]);
    },
  );

  test(
    "leaves the Subscription once its `on` answers, as when its Session closed, still delivering what it took",
    { tags: ["story:11-62"] },
    async () => {
      const seen: unknown[] = [];
      const connected = connection();
      connected.listen("session-1", (page) => {
        seen.push(page);
      });
      connected.publish(opened("page-1", "session-1"));
      const [subscription, on] = connected.sent;

      on?.answer();
      await settled();

      expect(seen).toEqual([{ page_id: "page-1", session_id: "session-1" }]);
      expect(subscription?.cancelled()).toBe(true);
      expect(connected.endings).toEqual([]);
    },
  );

  test(
    "ends each listener with the error that ended the Subscription, cancelling its `on`",
    { tags: ["story:11-62"] },
    async () => {
      const connected = connection();
      connected.listen("session-1", () => {});
      connected.listen("session-2", () => {});
      const [subscription, first, second] = connected.sent;
      const lost = new Error("the stream failed");

      subscription?.end(lost);
      await settled();

      expect(connected.endings).toEqual([lost, lost]);
      expect(first?.cancelled()).toBe(true);
      expect(second?.cancelled()).toBe(true);
    },
  );

  test(
    "ends each listener without an error when the engine answers the Subscription, as when its stream stops",
    { tags: ["story:11-62"] },
    async () => {
      const connected = connection();
      connected.listen("session-1", () => {});
      const [subscription, on] = connected.sent;

      subscription?.answer();
      await settled();

      expect(on?.cancelled()).toBe(true);
      expect(connected.endings).toEqual([]);
    },
  );

  test(
    "removes a listener that throws, cancelling its `on`, while the others keep the Subscription",
    { tags: ["story:11-63"] },
    async () => {
      const bug = new Error("the listener's bug");
      const kept: unknown[] = [];
      const connected = connection();
      connected.listen("session-1", () => {
        throw bug;
      });
      connected.listen("session-1", (page) => {
        kept.push(page);
      });
      const [subscription, thrown] = connected.sent;

      connected.publish(opened("page-1", "session-1"), opened("page-2", "session-1"));
      await settled();

      expect(connected.failures).toEqual([bug]);
      expect(thrown?.cancelled()).toBe(true);
      expect(subscription?.cancelled()).toBe(false);
      expect(kept).toHaveLength(2);
    },
  );

  test(
    "logs a warning when the engine fell behind, since the events in between are gone",
    { tags: ["story:11-62"] },
    async () => {
      const { logger, warned } = warnings();
      const connected = connection(logger);
      connected.listen("session-1", () => {});

      connected.publish({ position: 7, event: "events.resync", data: {} });
      await settled();

      expect(warned).toHaveLength(1);
      expect(connected.endings).toEqual([]);
    },
  );
});
