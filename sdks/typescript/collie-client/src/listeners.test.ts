// A listener's subscription (ADR 0081) on a call faked here, which records
// the credits granted and sends `$/events` batches as the engine does: the
// listener gets its events one at a time, in order, a credit goes back only
// once it settled, and one that throws is removed, its call cancelled.
import { describe, expect, test } from "vitest";
import type { Listener } from "./handle.ts";
import { subscribe, type ListenCall, type ListenOptions, type Listening } from "./listeners.ts";
import type { StreamedEvent } from "./stream.ts";

/** A subscription on a faked call: its grants, its failures, and the engine's side of it. */
function subscribed<Payload>(event: string, listener: Listener<Payload>) {
  const granted: number[] = [];
  const failures: unknown[] = [];
  const endings: unknown[] = [];
  let given: ListenOptions | undefined;
  let answer: { resolve(value: unknown): void; reject(error: unknown): void } | undefined;
  const result = new Promise<unknown>((resolve, reject) => {
    answer = { resolve, reject };
  });
  const open = (options: ListenOptions): ListenCall => {
    given = options;
    // As the Channel does, a cancelled call rejects with its signal's reason.
    options.signal.addEventListener("abort", () => answer?.reject(options.signal.reason));
    return { result, grant: (credits) => granted.push(credits) };
  };
  const listening: Listening<Payload> = {
    event,
    listener,
    failed: (cause) => failures.push(cause),
    ended: (error) => endings.push(error),
  };
  const subscription = subscribe(open, listening);
  return {
    subscription,
    granted,
    failures,
    endings,
    /** Sends one `$/events` batch on the call. */
    send(...events: StreamedEvent[]): void {
      given?.notified("$/events", { id: 3, events });
    },
    cancelled: (): boolean => given?.signal.aborted === true,
    end: (error: unknown): void => answer?.reject(error),
  };
}

let position = 0;

/** The event `name` carrying `data`, at the next position. */
function streamed(name: string, data: unknown, dropped?: number): StreamedEvent {
  position += 1;
  return dropped === undefined
    ? { position, event: name, data }
    : { position, event: name, data, dropped };
}

/** Lets every pending callback and promise run. */
function settled(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

/** A promise and what settles it. */
function gate(): { readonly opened: Promise<void>; open(): void } {
  const { promise, resolve } = Promise.withResolvers<void>();
  return { opened: promise, open: () => resolve() };
}

describe("a listener's subscription", () => {
  test(
    "gives its listener the events one at a time, in order, awaiting each before the next",
    { tags: ["story:11-63"] },
    async () => {
      const seen: string[] = [];
      const first = gate();
      const call = subscribed<string>("page.console", async (text) => {
        seen.push(`start ${text}`);
        if (text === "1") await first.opened;
        seen.push(`end ${text}`);
      });

      call.send(streamed("page.console", "1"), streamed("page.console", "2"));
      call.send(streamed("page.console", "3"));
      await settled();
      expect(seen).toEqual(["start 1"]);
      first.open();
      await settled();

      expect(seen).toEqual(["start 1", "end 1", "start 2", "end 2", "start 3", "end 3"]);
    },
  );

  test(
    "grants its credits when sent, and more only once its listener settled the events they cover",
    { tags: ["story:11-63"] },
    async () => {
      const first = gate();
      const call = subscribed<number>("page.console", async (index) => {
        if (index === 0) await first.opened;
      });
      expect(call.granted).toEqual([16]);

      call.send(...Array.from({ length: 8 }, (_, index) => streamed("page.console", index)));
      await settled();
      expect(call.granted).toEqual([16]);
      first.open();
      await settled();

      expect(call.granted).toEqual([16, 8]);
    },
  );

  test(
    "skips the call's other events, which still count toward its credits",
    { tags: ["story:11-62"] },
    async () => {
      const seen: unknown[] = [];
      const call = subscribed("page.web_socket", (socket) => {
        seen.push(socket);
      });

      call.send(
        streamed("page.web_socket", "socket"),
        ...Array.from({ length: 7 }, () => streamed("web_socket.frame_sent", "frame")),
      );
      await settled();

      expect(seen).toEqual(["socket"]);
      expect(call.granted).toEqual([16, 8]);
    },
  );

  test(
    "hands its listener how many telemetry events the engine replaced, and 0 for a lossless one",
    { tags: ["story:11-62"] },
    async () => {
      const dropped: number[] = [];
      const call = subscribed("engine.system_info", (_, count) => {
        dropped.push(count);
      });

      call.send(streamed("engine.system_info", {}, 3), streamed("engine.system_info", {}));
      await settled();

      expect(dropped).toEqual([3, 0]);
    },
  );

  test.for([
    [
      "throws",
      (): never => {
        throw new Error("the listener's bug");
      },
    ],
    [
      "returns an awaitable that fails",
      (): Promise<never> => Promise.reject(new Error("the listener's bug")),
    ],
  ] as const)(
    "removes a listener that %s: its call is cancelled and its value hears it once",
    { tags: ["story:11-63"] },
    async ([, listener]) => {
      let calls = 0;
      const call = subscribed("page.console", () => {
        calls += 1;
        return listener();
      });

      call.send(streamed("page.console", "1"), streamed("page.console", "2"));
      await settled();
      call.send(streamed("page.console", "3"));
      await settled();

      expect(calls).toBe(1);
      expect(call.cancelled()).toBe(true);
      expect(call.failures).toEqual([new Error("the listener's bug")]);
      expect(call.endings).toEqual([]);
    },
  );

  test(
    "closes synchronously: it cancels its call once, and an event still waiting never reaches the listener",
    { tags: ["story:11-62"] },
    async () => {
      const seen: unknown[] = [];
      const first = gate();
      const call = subscribed("page.console", async (text) => {
        seen.push(text);
        await first.opened;
      });

      call.send(streamed("page.console", "1"), streamed("page.console", "2"));
      call.subscription.close();
      call.subscription[Symbol.dispose]();
      first.open();
      await settled();

      expect(call.cancelled()).toBe(true);
      expect(seen).toEqual(["1"]);
      expect(call.endings).toEqual([]);
    },
  );

  test(
    "tells its value the error that ended its call, such as `page_events_lost`",
    { tags: ["story:11-62"] },
    async () => {
      const call = subscribed("page.console", () => {});
      const lost = new Error("page_events_lost");

      call.end(lost);
      await settled();

      expect(call.endings).toEqual([lost]);
    },
  );
});
