// Seam B (spec #326): the in-memory adapter of the protocol (ADR 0068)
// answers the Console's calls with the values a test sets, in any order,
// streams pushed events by position across a dropped Connection, and serves
// bodies by handle. A tiny Client written here drives it through its
// `transport` and `bodyFetcher`, until area 12's Console Client exists.
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";
import {
  createProtocolAdapter,
  failure,
  type Transport,
} from "@goodfetch/test-support/protocol-adapter";

/** A JSON object, as the tiny Client reads one. */
type Fields = Readonly<Record<string, unknown>>;

function isFields(value: unknown): value is Fields {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A Client of the fewest lines: it sends JSON-RPC text and reads what comes back. */
class TinyClient {
  readonly received: Fields[] = [];
  closed = false;
  readonly #connection;
  #waiting: (() => void)[] = [];

  constructor(transport: Transport) {
    this.#connection = transport.open({
      message: (text) => {
        const message: unknown = JSON.parse(text);
        if (!isFields(message)) {
          throw new Error(`the engine sent a message that is no JSON object: ${text}`);
        }
        this.received.push(message);
        this.#wake();
      },
      close: () => {
        this.closed = true;
        this.#wake();
      },
    });
  }

  send(message: Fields): void {
    this.#connection.send(JSON.stringify({ jsonrpc: "2.0", ...message }));
  }

  /** Calls `method` and resolves with its reply. */
  async call(id: number, method: string, params: unknown): Promise<Fields> {
    this.send({ id, method, params });
    return this.until(() => this.reply(id));
  }

  /** The reply to call `id`, once it came. */
  reply(id: number): Fields | undefined {
    return this.received.find((message) => message["id"] === id && !("method" in message));
  }

  /** Subscribes as call `id`, granting `credits` events. */
  subscribe(id: number, params: Fields, credits: number): void {
    this.send({ id, method: "events.subscribe", params });
    this.credit(id, credits);
  }

  credit(id: number, credits: number): void {
    this.send({ method: "$/credit", params: { id, credits } });
  }

  /** Each `$/events` batch of subscription `id` so far, as its events. */
  batches(id: number): Fields[][] {
    return this.received
      .filter((message) => message["method"] === "$/events")
      .map((message) => message["params"])
      .filter(isFields)
      .filter((params) => params["id"] === id)
      .map((params) => (Array.isArray(params["events"]) ? params["events"].filter(isFields) : []));
  }

  /** Every event of subscription `id` so far. */
  events(id: number): Fields[] {
    return this.batches(id).flat();
  }

  /** Waits until subscription `id` has `count` events, and gives them. */
  async eventsUntil(id: number, count: number): Promise<Fields[]> {
    return this.until(() => {
      const events = this.events(id);
      return events.length >= count ? events : undefined;
    });
  }

  /** Waits until `found` gives a value. */
  async until<T>(found: () => T | undefined): Promise<T> {
    for (;;) {
      const value = found();
      if (value !== undefined) {
        return value;
      }
      await new Promise<void>((resolve) => this.#waiting.push(resolve));
    }
  }

  #wake(): void {
    const waiting = this.#waiting;
    this.#waiting = [];
    for (const resolve of waiting) {
      resolve();
    }
  }
}

/** A Run's axes as `run.axes_changed` reports them. */
function axes(run_id: string, lifecycle: "running" | "finished") {
  return { event: "run.axes_changed", data: { run_id, lifecycle } } as const;
}

describe("createProtocolAdapter", () => {
  test("answers a call with the value the test set", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("sample.echo", { text: "hello hello" });
    const client = new TinyClient(adapter.transport);

    const reply = await client.call(1, "sample.echo", { text: "hello" });

    expect(reply).toEqual({ jsonrpc: "2.0", id: 1, result: { text: "hello hello" } });
  });

  test("answers calls in whatever order they come, each from its own params", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("sample.echo", ({ text }) => ({ text: `${text} ${text}` }));
    adapter.answer("run.start", ({ run_id }) => ({
      run_id: run_id ?? "generated",
      started_as: "new",
      lifecycle: "running",
      needs: "collie",
    }));
    const client = new TinyClient(adapter.transport);

    const replies = await Promise.all([
      client.call(1, "run.start", { run_id: "nightly" }),
      client.call(2, "sample.echo", { text: "woof" }),
    ]);

    expect(replies.map((reply) => reply["result"])).toEqual([
      { run_id: "nightly", started_as: "new", lifecycle: "running", needs: "collie" },
      { text: "woof woof" },
    ]);
  });

  test("answers a failure with its Error code's class and retryable from the catalog", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.finish", failure("run_not_found", { run_id: "gone" }));
    const client = new TinyClient(adapter.transport);

    const reply = await client.call(1, "run.finish", { run_id: "gone" });

    expect(reply["error"]).toMatchObject({
      code: -32001,
      data: { code: "run_not_found", retryable: false, run_id: "gone" },
    });
  });

  test("records every message the Client sent, in order", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("sample.echo", { text: "a a" });
    const client = new TinyClient(adapter.transport);

    await client.call(1, "hello", {
      protocol_version: "0.1",
      surface: "console",
      product_version: "0.1.0",
    });
    await client.call(2, "sample.echo", { text: "a" });

    expect(adapter.requests).toEqual([
      {
        method: "hello",
        id: 1,
        params: { protocol_version: "0.1", surface: "console", product_version: "0.1.0" },
      },
      { method: "sample.echo", id: 2, params: { text: "a" } },
    ]);
  });

  test.for([
    {
      name: "a result breaking its schema",
      answer: { run_id: "Not A Run ID", started_as: "new", lifecycle: "running", needs: "collie" },
    },
    { name: "a failure no catalog Error code names", answer: failure("not_a_code") },
  ] as const)("refuses, as it is set, $name", ({ answer }) => {
    const adapter = createProtocolAdapter();

    const setting = (): void => adapter.answer("run.start", answer);

    expect(setting).toThrow(expect.objectContaining({ code: "answer_invalid" }));
  });

  test("refuses, as it is set, an operation of Foxhound's on Collie's engine", () => {
    const adapter = createProtocolAdapter();

    // @ts-expect-error -- Collie's generated types refuse it too; a test that casts past them still fails.
    const setting = (): void => adapter.answer("sample.crawl", { visited: 1 });

    expect(setting).toThrow(expect.objectContaining({ code: "answer_invalid" }));
  });

  test("answers an operation of Foxhound's on Foxhound's engine", async () => {
    // Foxhound's tests pass its generated `FoxhoundOperationTypes`; this one names the entry it needs.
    const adapter = createProtocolAdapter<{
      "sample.crawl": { params: unknown; result: { visited: number } };
    }>({ product: "foxhound" });
    adapter.answer("sample.crawl", { visited: 2 });
    const client = new TinyClient(adapter.transport);

    const reply = await client.call(1, "sample.crawl", { echo: { text: "a" }, pages: 2 });

    expect(reply["result"]).toEqual({ visited: 2 });
  });

  test("takes, as it is set, a result of its schema", () => {
    const adapter = createProtocolAdapter();

    const setting = (): void => adapter.answer("sample.echo", { text: "fine" });

    expect(setting).not.toThrow();
  });
});

describe("ProtocolAdapter events", () => {
  test("streams pushed events to a subscription, never more than its credits", async () => {
    const adapter = createProtocolAdapter();
    const client = new TinyClient(adapter.transport);
    client.subscribe(1, {}, 2);
    adapter.push(axes("one", "running"), 1);
    adapter.push(axes("two", "running"), 2);
    adapter.push(axes("one", "finished"), 3);
    const beforeCredit = (await client.eventsUntil(1, 2)).map((event) => event["position"]);

    client.credit(1, 1);

    const afterCredit = (await client.eventsUntil(1, 3)).map((event) => event["position"]);
    expect({ beforeCredit, afterCredit }).toEqual({ beforeCredit: [1, 2], afterCredit: [1, 2, 3] });
  });

  test("after drop(), a resumed subscription continues after its last position", async () => {
    const adapter = createProtocolAdapter();
    const first = new TinyClient(adapter.transport);
    first.subscribe(1, {}, 10);
    adapter.push(axes("one", "running"), 1);
    adapter.push(axes("two", "running"), 2);
    await first.eventsUntil(1, 2);
    adapter.drop();
    adapter.push(axes("one", "finished"), 3);
    await first.until(() => (first.closed ? true : undefined));

    const resumed = new TinyClient(adapter.transport);
    resumed.subscribe(1, { after: 2 }, 10);

    expect(await resumed.eventsUntil(1, 1)).toEqual([
      { position: 3, event: "run.axes_changed", data: { run_id: "one", lifecycle: "finished" } },
    ]);
  });

  test("after forgetPositions(), a resumed subscription restarts from events.resync", async () => {
    const adapter = createProtocolAdapter();
    const first = new TinyClient(adapter.transport);
    first.subscribe(1, {}, 1);
    adapter.push(axes("one", "running"), 1);
    adapter.push(axes("one", "finished"), 2);
    await first.eventsUntil(1, 1);
    adapter.drop();
    adapter.forgetPositions();

    const resumed = new TinyClient(adapter.transport);
    resumed.subscribe(1, { after: 1 }, 10);
    adapter.push(axes("two", "running"), 4);

    expect(await resumed.eventsUntil(1, 2)).toEqual([
      {
        position: 3,
        event: "events.resync",
        data: { runs: [{ run_id: "one", lifecycle: "finished" }] },
      },
      { position: 4, event: "run.axes_changed", data: { run_id: "two", lifecycle: "running" } },
    ]);
  });

  test("answers a cancelled subscription with call_cancelled", async () => {
    const adapter = createProtocolAdapter();
    const client = new TinyClient(adapter.transport);
    client.subscribe(1, {}, 10);

    client.send({ method: "$/cancel", params: { id: 1 } });

    const reply = await client.until(() => client.reply(1));
    expect(reply["error"]).toMatchObject({
      code: -32006,
      data: { code: "call_cancelled", retryable: false, by: "client" },
    });
  });

  test.for([
    { name: "a payload breaking its schema", event: axes("Not A Run ID", "running"), position: 1 },
    { name: "a position that does not go up", event: axes("one", "running"), position: 0 },
  ])("refuses to push $name", ({ event, position }) => {
    const adapter = createProtocolAdapter();

    const pushing = (): void => adapter.push(event, position);

    expect(pushing).toThrow(expect.objectContaining({ code: "event_invalid" }));
  });

  test("refuses to push an event the catalog lacks", () => {
    const adapter = createProtocolAdapter();

    // @ts-expect-error -- the generated types refuse it too; a test that casts past them still fails.
    const pushing = (): void => adapter.push({ event: "run.vanished", data: {} }, 1);

    expect(pushing).toThrow(expect.objectContaining({ code: "event_invalid" }));
  });

  test("refuses to push events.resync, which it sends itself", () => {
    const adapter = createProtocolAdapter();

    // @ts-expect-error -- the adapter's types leave it out too; a test that casts past them still fails.
    const pushing = (): void => adapter.push({ event: "events.resync", data: { runs: [] } }, 1);

    expect(pushing).toThrow(expect.objectContaining({ code: "event_invalid" }));
  });

  test("pushes an event of the catalog at a position past the last", () => {
    const adapter = createProtocolAdapter();
    adapter.push(axes("one", "running"), 1);

    const pushing = (): void => adapter.push(axes("one", "finished"), 2);

    expect(pushing).not.toThrow();
  });
});

describe("ProtocolAdapter bodies", () => {
  test("serves the bytes of a body by its handle", async () => {
    const adapter = createProtocolAdapter();
    adapter.serve("screenshot-1.png", new Uint8Array([137, 80, 78, 71]));

    const bytes = await adapter.bodyFetcher("screenshot-1.png");

    expect(bytes).toEqual(new Uint8Array([137, 80, 78, 71]));
  });
});

const vitest = join(
  dirname(createRequire(import.meta.url).resolve("vitest/package.json")),
  "vitest.mjs",
);
const fixture = fileURLToPath(new URL("fixtures/protocol-adapter/", import.meta.url));

// Each case runs one test of the fixture project of fixtures/protocol-adapter/
// in its own Vitest, and reads how it ended and the codes that failed it.
describe("ProtocolAdapter failing the test", () => {
  test.for([
    { name: "an unanswered call", ran: "ran: an unanswered call failed call_unanswered" },
    {
      name: "an answer breaking its schema",
      ran: "ran: an answer breaking its schema failed answer_invalid",
    },
    {
      name: "a request breaking its schema",
      ran: "ran: a request breaking its schema failed request_invalid",
    },
    { name: "a body never served", ran: "ran: a body never served failed body_unserved" },
    {
      name: "a call answered as the bundle allows",
      ran: "ran: a call answered as the bundle allows passed ",
    },
  ])("$name", async ({ name, ran }, { processes, onTestFinished }) => {
    const output = await mkdtemp(join(tmpdir(), "goodfetch-protocol-adapter-"));
    onTestFinished(() => rm(output, { recursive: true, force: true }));
    const run = processes.spawn(
      process.execPath,
      [vitest, "run", "--config", "vitest.config.ts", "--root", ".", `--testNamePattern=^${name}$`],
      { env: { ...process.env, GOODFETCH_FIXTURE_OUTPUT: output }, cwd: fixture },
    );
    let stdout = "";
    run.child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
      stdout += chunk;
    });
    run.child.stderr.resume();

    await run.exit;

    expect(stdout.split("\n").filter((line) => line.startsWith("ran: "))).toEqual([ran]);
  });
});
