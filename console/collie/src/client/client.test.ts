// The Console's Client against the in-memory adapter of the protocol (spec
// #592's Seam 1; ADR 0068): its `hello`, the raw catalog call, a
// subscription from a position and a resync that replaces a screen's
// state, its three connection states, bodies by handle, and, after a
// dropped Connection, nothing sent again but its subscriptions.
import { ConnectionLostError, type RunAxes } from "@goodfetch/collie-client/generated";
import { test } from "@goodfetch/test-support";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { describe, expect } from "vitest";
import {
  createConsoleClient,
  type ConnectionState,
  type ConsoleClient,
  type PageVisibility,
  type StreamItem,
  type Subscription,
} from "./client.ts";

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
    adapter.push(axes("second", "finished"), 2);
    adapter.forgetPositions();
    const [resync] = await take(subscription, 1);
    runs = fold(runs, resync!);

    expect(resync).toMatchObject({ kind: "resync", position: 3 });
    expect([...runs]).toEqual([
      ["first", "running"],
      ["second", "finished"],
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

  test("fetches a body by its handle", async () => {
    const adapter = createProtocolAdapter();
    adapter.serve("bodies/7a3f", new TextEncoder().encode("<html>body</html>"));
    const { client } = clientOf(adapter);

    const body = await client.body("bodies/7a3f");

    expect(new TextDecoder().decode(body)).toBe("<html>body</html>");
    client.close();
  });

  test("after a drop it fails the call in flight and sends nothing again but its subscription", async () => {
    const adapter = createProtocolAdapter();
    const { promise: calledOnce, resolve: asked } = Promise.withResolvers<void>();
    adapter.answer("sample.echo", () => {
      asked();
      return new Promise<never>(() => {});
    });
    const { client } = clientOf(adapter);
    const subscription = client.subscribe("events");
    await reaches(client, "connected");
    adapter.push(axes("first", "running"), 1);
    await take(subscription, 1);
    const inFlight = client.call("sample.echo", { text: "woof" });
    await calledOnce;
    const sentBefore = adapter.requests.length;

    adapter.drop();
    await expect(inFlight).rejects.toBeInstanceOf(ConnectionLostError);
    await reaches(client, "connected");
    adapter.push(axes("first", "finished"), 2);
    const [resumed] = await take(subscription, 1);

    const sentAfter = adapter.requests.slice(sentBefore);
    const methods = sentAfter.map((request) => request.method);
    expect(methods.filter((method) => method !== "$/credit")).toEqual([
      "hello",
      "events.subscribe",
    ]);
    expect(sentAfter.find((request) => request.method === "events.subscribe")?.params).toEqual({
      after: { engine_instance: FIRST_INSTANCE, position: 1 },
    });
    expect(resumed).toMatchObject({ kind: "event", position: 2 });
    client.close();
  });
});
