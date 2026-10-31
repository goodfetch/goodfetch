// The Console's Client against the in-memory adapter of the protocol (spec
// #592's Seam 1; ADR 0068): its `hello`, the raw catalog call, a
// subscription from a position and a resync that replaces a screen's
// state, its three connection states, bodies by handle, and, after a
// dropped Connection, nothing sent again but its subscriptions.
import {
  ConnectionLostError,
  UnavailableError,
  type RunAxes,
} from "@goodfetch/collie-client/generated";
import { test } from "@goodfetch/test-support";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { describe, expect, onTestFinished, vi } from "vitest";
import {
  createConsoleClient,
  type ConnectionState,
  type ConsoleClient,
  type PageVisibility,
  type StreamItem,
  type Subscription,
} from "./client.ts";

/** How long the Client waits on a silent Connection before it asks, then before it gives up. */
const STALL = 10_000;

/** The engine instance the adapter's first `hello` names. */
const FIRST_INSTANCE = "0000000000000001";

/** A page whose visibility the test sets. */
class Visibility implements PageVisibility {
  visible = true;
  readonly #listeners = new Set<() => void>();

  watch(changed: () => void): () => void {
    this.#listeners.add(changed);
    return () => this.#listeners.delete(changed);
  }

  set(visible: boolean): void {
    this.visible = visible;
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

/** A Client of `adapter`, each state it entered recorded. */
function clientOf(
  adapter: ProtocolAdapter,
  visibility = new Visibility(),
): { client: ConsoleClient; states: ConnectionState[] } {
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility,
    product: "collie",
  });
  const states: ConnectionState[] = [client.state];
  client.watch((state) => states.push(state));
  return { client, states };
}

/** Resolves once `client` is in `state`. */
async function reaches(client: ConsoleClient, state: ConnectionState): Promise<void> {
  if (client.state === state) {
    return;
  }
  await new Promise<void>((resolve) => {
    const stop = client.watch((now) => {
      if (now === state) {
        stop();
        resolve();
      }
    });
  });
}

/** The next `count` items of `subscription`. */
async function take(subscription: Subscription, count: number): Promise<StreamItem[]> {
  const iterator = subscription[Symbol.asyncIterator]();
  const items: StreamItem[] = [];
  while (items.length < count) {
    const next = await iterator.next();
    if (next.done === true) {
      break;
    }
    items.push(next.value);
  }
  return items;
}

function axes(run_id: string, lifecycle: RunAxes["lifecycle"]) {
  return { event: "run.axes_changed", data: { run_id, lifecycle } } as const;
}

function isAxes(data: unknown): data is RunAxes {
  return typeof data === "object" && data !== null && "run_id" in data && "lifecycle" in data;
}

/** A screen's runs, as a subscription's items fold them: a resync replaces them. */
function fold(runs: ReadonlyMap<string, string>, item: StreamItem): Map<string, string> {
  if (item.kind === "resync") {
    return new Map(item.snapshot.runs.map((run) => [run.run_id, run.lifecycle]));
  }
  const next = new Map(runs);
  const { data } = item;
  if (isAxes(data)) {
    next.set(data.run_id, data.lifecycle);
  }
  return next;
}

describe("the Console's Client", () => {
  test("says hello as the Console, in the catalog's form, naming its Product and version", async () => {
    const adapter = createProtocolAdapter();
    const { client } = clientOf(adapter);

    await reaches(client, "connected");

    const [hello] = adapter.requests;
    expect(hello?.method).toBe("hello");
    expect(hello?.params).toMatchObject({
      surface: "console",
      form: "catalog",
      needs: "collie",
      protocol_version: "0.1",
    });
    expect(client.engine).toEqual({
      product: "collie",
      productVersion: "0.1.0",
      engineInstance: FIRST_INSTANCE,
    });
    client.close();
  });

  test("calls an operation of the catalog with its wire params", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("sample.echo", ({ text }) => ({ text: `${text} ${text}` }));
    const { client } = clientOf(adapter);
    await reaches(client, "connected");

    const echoed = await client.call("sample.echo", { text: "woof" });

    expect(echoed).toEqual({ text: "woof woof" });
    // @ts-expect-error: a screen's call to an operation outside the catalog fails the type check.
    expect(() => client.call("no_such.operation", {})).toBeDefined();
    client.close();
  });

  test("a call its signal aborts is cancelled on the engine and rejects with the signal's reason", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.progress", () => new Promise(() => {}));
    const { client } = clientOf(adapter);
    await reaches(client, "connected");
    const abort = new AbortController();

    const reading = client.call(
      "run.progress",
      { run_id: "nightly", after: 1 },
      { signal: abort.signal },
    );
    await vi.waitFor(() => {
      expect(adapter.requests.map((request) => request.method)).toContain("run.progress");
    });
    abort.abort(new Error("the page left"));

    await expect(reading).rejects.toThrow("the page left");
    const read = adapter.requests.find((request) => request.method === "run.progress");
    await vi.waitFor(() => {
      expect(adapter.requests.at(-1)).toEqual({ method: "$/cancel", params: { id: read?.id } });
    });
    client.close();
  });

  test("subscribes from a position, reading only the events after it", async () => {
    const adapter = createProtocolAdapter();
    adapter.push(axes("first", "running"), 1);
    adapter.push(axes("second", "running"), 2);
    adapter.push(axes("first", "finished"), 3);
    const { client } = clientOf(adapter);

    const subscription = client.subscribe("events", {
      after: { engine_instance: FIRST_INSTANCE, position: 1 },
    });
    const items = await take(subscription, 2);

    expect(items).toEqual([
      {
        kind: "event",
        class: "lossless",
        position: 2,
        event: "run.axes_changed",
        data: { run_id: "second", lifecycle: "running" },
        dropped: 0,
      },
      {
        kind: "event",
        class: "lossless",
        position: 3,
        event: "run.axes_changed",
        data: { run_id: "first", lifecycle: "finished" },
        dropped: 0,
      },
    ]);
    expect(subscription.lastRead).toEqual({ engine_instance: FIRST_INSTANCE, position: 3 });
    client.close();
  });

  test("a resync replaces the screen's state with its snapshot", async () => {
    const adapter = createProtocolAdapter();
    const { client } = clientOf(adapter);
    const subscription = client.subscribe("events");
    await reaches(client, "connected");
    adapter.push(axes("first", "running"), 1);
    const [first] = await take(subscription, 1);
    let runs = fold(new Map([["stale", "running"]]), first!);

    adapter.drop();
    adapter.push(axes("second", "running"), 2);
    adapter.push(axes("third", "finished"), 3);
    adapter.forgetPositions();
    const [resync] = await take(subscription, 1);
    runs = fold(runs, resync!);

    // A resync gives the live runs alone: one that ended is in run.list (#1768).
    expect(resync).toMatchObject({ kind: "resync", position: 4 });
    expect([...runs]).toEqual([
      ["first", "running"],
      ["second", "running"],
    ]);
    client.close();
  });

  test("is connected, then read-only while hidden and dropped, then reconnecting and connected again", async () => {
    const adapter = createProtocolAdapter();
    const visibility = new Visibility();
    const { client, states } = clientOf(adapter, visibility);
    await reaches(client, "connected");

    visibility.set(false);
    adapter.drop();
    await reaches(client, "read_only");
    visibility.set(true);
    await reaches(client, "connected");

    expect(states).toEqual(["reconnecting", "connected", "read_only", "reconnecting", "connected"]);
    client.close();
  });

  test(
    "closes its Connection while the page is hidden, so a hidden tab counts as no Client, and opens one once it is visible again",
    { tags: ["story:12-6"] },
    async () => {
      const adapter = createProtocolAdapter();
      const visibility = new Visibility();
      const { client, states } = clientOf(adapter, visibility);
      await reaches(client, "connected");

      visibility.set(false);
      await reaches(client, "read_only");
      visibility.set(true);
      await reaches(client, "connected");

      expect(states).toEqual([
        "reconnecting",
        "connected",
        "read_only",
        "reconnecting",
        "connected",
      ]);
      expect(adapter.requests.filter((request) => request.method === "hello")).toHaveLength(2);
      client.close();
    },
  );

  test(
    "closes a Connection that stops answering and opens another",
    { tags: ["story:12-125"] },
    async () => {
      vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
      onTestFinished(() => {
        vi.useRealTimers();
      });
      const adapter = createProtocolAdapter();
      const { client, states } = clientOf(adapter);
      await reaches(client, "connected");
      const { promise: probed, resolve: probe } = Promise.withResolvers<void>();
      adapter.answer("engine.info", () => {
        probe();
        return new Promise<never>(() => {});
      });

      const reconnecting = reaches(client, "reconnecting");
      for (let tick = 0; tick < 3; tick += 1) {
        vi.advanceTimersByTime(STALL);
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
      await probed;
      await reconnecting;
      await reaches(client, "connected");

      expect(states).toEqual(["reconnecting", "connected", "reconnecting", "connected"]);
      expect(adapter.requests.filter((request) => request.method === "hello")).toHaveLength(2);
      client.close();
    },
  );

  test("a Connection that keeps answering is never closed", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    onTestFinished(() => {
      vi.useRealTimers();
    });
    const adapter = createProtocolAdapter();
    adapter.answer("engine.info", {
      product: "collie",
      product_major: 0,
      product_version: "0.1.0",
      protocol_version: "0.1",
    });
    const { client, states } = clientOf(adapter);
    await reaches(client, "connected");

    for (let tick = 0; tick < 4; tick += 1) {
      vi.advanceTimersByTime(STALL);
      await new Promise((resolve) => setTimeout(resolve, 0));
    }

    expect(states).toEqual(["reconnecting", "connected"]);
    client.close();
  });

  test("fetches a body by its handle", async () => {
    const adapter = createProtocolAdapter();
    adapter.serve("bodies/7a3f", new TextEncoder().encode("<html>body</html>"));
    const { client } = clientOf(adapter);

    const body = await client.body("bodies/7a3f");

    expect(new TextDecoder().decode(body)).toBe("<html>body</html>");
    client.close();
  });

  test(
    "after a drop it sends a read and an idempotent write again, and fails an Effect with connection_lost",
    { tags: ["story:12-123", "error:connection_lost"] },
    async () => {
      const adapter = createProtocolAdapter();
      const asked = new Map<string, number>();
      const { promise: allAsked, resolve: everyCallArrived } = Promise.withResolvers<void>();
      /** Answers `answer` from the second call of `method` on; the first never comes. */
      const secondTime =
        <Answer>(method: string, answer: Answer) =>
        (): Promise<Answer> => {
          const count = (asked.get(method) ?? 0) + 1;
          asked.set(method, count);
          if (asked.size === 3 && count === 1) {
            everyCallArrived();
          }
          return count === 1 ? new Promise<never>(() => {}) : Promise.resolve(answer);
        };
      adapter.answer("sample.echo", secondTime("sample.echo", { text: "woof woof" }));
      adapter.answer("connection.set_log_level", secondTime("connection.set_log_level", {}));
      adapter.answer("daemon.stop", secondTime("daemon.stop", { live_runs: 0 }));
      const { client } = clientOf(adapter);
      const subscription = client.subscribe("events");
      await reaches(client, "connected");
      adapter.push(axes("first", "running"), 1);
      await take(subscription, 1);
      const read = client.call("sample.echo", { text: "woof" });
      const idempotentWrite = client.call("connection.set_log_level", { log_level: "debug" });
      const effect = client.call("daemon.stop", {});
      await allAsked;
      const sentBefore = adapter.requests.length;

      adapter.drop();
      await expect(effect).rejects.toBeInstanceOf(UnavailableError);
      await expect(effect).rejects.toBeInstanceOf(ConnectionLostError);
      await expect(read).resolves.toEqual({ text: "woof woof" });
      await expect(idempotentWrite).resolves.toEqual({});
      adapter.push(axes("first", "finished"), 2);
      const [resumed] = await take(subscription, 1);

      const sentAfter = adapter.requests.slice(sentBefore);
      const methods = sentAfter.map((request) => request.method);
      expect(methods.filter((method) => method !== "$/credit").toSorted()).toEqual([
        "connection.set_log_level",
        "events.subscribe",
        "hello",
        "sample.echo",
      ]);
      expect(methods[0]).toBe("hello");
      expect(sentAfter.find((request) => request.method === "events.subscribe")?.params).toEqual({
        after: { engine_instance: FIRST_INSTANCE, position: 1 },
      });
      expect(resumed).toMatchObject({ kind: "event", position: 2 });
      client.close();
    },
  );

  test(
    "while it has no Connection, a read waits for the next one and an Effect fails at once",
    { tags: ["story:12-125", "error:connection_lost"] },
    async () => {
      const adapter = createProtocolAdapter();
      adapter.answer("sample.echo", ({ text }) => ({ text }));
      adapter.answer("daemon.stop", { live_runs: 0 });
      const visibility = new Visibility();
      const { client } = clientOf(adapter, visibility);
      await reaches(client, "connected");
      visibility.set(false);
      adapter.drop();
      await reaches(client, "read_only");

      const effect = client.call("daemon.stop", {});
      const read = client.call("sample.echo", { text: "woof" });
      await expect(effect).rejects.toBeInstanceOf(ConnectionLostError);
      visibility.set(true);

      await expect(read).resolves.toEqual({ text: "woof" });
      expect(adapter.requests.map((request) => request.method)).not.toContain("daemon.stop");
      client.close();
    },
  );
});
