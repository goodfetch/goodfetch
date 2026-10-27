// What the Console conformance cases ask of their target beside the page
// (ADRs 0031, 0068 and 0096): a run started and finished there (on a
// Daemon, with its executable's commands in the test's home; behind a
// gateway, with the protocol's, as the Console starts one), a Run's stored
// file, the page's WebSocket carried through the test as a network or a
// proxy carries it, and the Console's own Client reaching the target's
// endpoint with the page's sign-in, as configuration.
import { createHash } from "node:crypto";
import { access, mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { BrowserContext, Page, WebSocketRoute } from "@playwright/test";
import { expect, test } from "@goodfetch/test-support/conformance";
import { command } from "@goodfetch/test-support/daemon-calls";
import type { ServedDaemon } from "@goodfetch/test-support/playwright";
import { WebSocket } from "ws";
import { PRODUCT_VERSION } from "@goodfetch/collie-client/browser";
import { PROTOCOL_VERSION } from "@goodfetch/collie-client/generated";
import { createConsoleClient, type ConsoleClient, type Transport } from "@goodfetch/console/client";

/** The banner of a page with no Connection to its engine. */
export const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** What a case needs that the Stub gateway does not serve yet, each its reason to skip there. */
export const NOT_ON_THE_STUB_GATEWAY = {
  fanIn:
    "the Stub gateway sends a subscription naming no run only one engine's events, not every engine's in one order",
  restart: "the Stub gateway cannot be restarted on its port under a page",
  sizeCapped: "the Stub gateway's engines are not on the test's size-capped filesystem",
  bodies: "the Stub gateway serves no `/bodies/` and no content origin of a Run's files",
  eventStream: "the Stub gateway serves no Server-Sent Events plus `POST` fallback",
} as const;

/** Skips the running case behind a gateway, for what it needs there that is `why`. */
export function skipBehindGateway(daemon: ServedDaemon, why: string): void {
  test.skip(daemon.gateway !== undefined, why);
}

/** Each Product's name, as the Console shows it. */
export const PRODUCT_NAME = { collie: "Collie", foxhound: "Foxhound" } as const;

/**
 * Starts the Run `runId` on the test's target: with the executable's `run
 * start` in the test's home, or behind a gateway with `run.start` as the
 * Console sends it, which the gateway puts on an engine of the Product the
 * start names.
 */
export async function startRun(daemon: ServedDaemon, runId: string): Promise<void> {
  if (daemon.gateway === undefined) {
    await command(daemon, ["run", "start", "--run-id", runId]);
    return;
  }
  const console = await ProgramConnection.open(daemon, asConsole(daemon));
  try {
    await console.start("run.start", { run_id: runId });
  } finally {
    console.close();
  }
}

/** Finishes the Run `runId` on the test's target, reached as `startRun` reaches it. */
export async function finishRun(daemon: ServedDaemon, runId: string): Promise<void> {
  if (daemon.gateway === undefined) {
    await daemon.run(["run", "finish", runId]);
    return;
  }
  const console = await ProgramConnection.open(daemon, asConsole(daemon));
  try {
    await console.call("run.finish", { run_id: runId });
  } finally {
    console.close();
  }
}

/** Who a program's Connection says it is in its `hello`. */
export interface Speaking {
  /** Its surface: `console` for the Console, `sdk_typescript` for a Client program. */
  readonly surface: "console" | "sdk_typescript";
  /** The Product it needs. */
  readonly needs: "collie" | "foxhound";
}

/** The Console of the target's build, as its `hello` names it. */
export function asConsole(daemon: ServedDaemon): Speaking {
  return { surface: "console", needs: daemon.product };
}

/** The cookie a program gets from a gateway for its token at `POST /sign-in`. */
async function gatewayCookie(daemon: ServedDaemon): Promise<string> {
  const token = daemon.gateway?.token;
  if (token === undefined) {
    throw new Error("invariant: only a gateway signs a program in with its token");
  }
  const answer = await fetch(new URL("/sign-in", daemon.url), { method: "POST", body: token });
  const cookie = answer.headers.get("set-cookie")?.split(";")[0];
  if (answer.status !== 204 || cookie === undefined) {
    throw new Error(`the gateway's sign-in answered ${String(answer.status)}`);
  }
  return cookie;
}

/**
 * A Connection of the test's own to a gateway's `/protocol`, signed in with
 * the gateway's token: JSON-RPC over its WebSocket, as a Client program or
 * the Console speaks it, each answer the whole reply.
 */
export class ProgramConnection {
  /** Its `hello`'s whole reply. */
  readonly hello: unknown;
  readonly #socket: WebSocket;
  readonly #waiting: Map<number, (reply: unknown) => void>;
  #next = 1;

  private constructor(
    socket: WebSocket,
    hello: unknown,
    waiting: Map<number, (reply: unknown) => void>,
  ) {
    this.#socket = socket;
    this.hello = hello;
    this.#waiting = waiting;
  }

  /**
   * Opens one as `speaking`, once its `hello` is answered, signed in with
   * `cookie`, else with the gateway's token.
   */
  static async open(
    daemon: ServedDaemon,
    speaking: Speaking,
    cookie?: string,
  ): Promise<ProgramConnection> {
    const origin = new URL(daemon.url).origin;
    const url = new URL("/protocol", origin);
    url.protocol = "ws:";
    const socket = new WebSocket(url, {
      headers: { Cookie: cookie ?? (await gatewayCookie(daemon)), Origin: origin },
    });
    const waiting = new Map<number, (reply: unknown) => void>();
    const { promise: helloed, resolve } = Promise.withResolvers<unknown>();
    waiting.set(0, resolve);
    socket.on("message", (data, binary) => {
      if (binary) {
        return;
      }
      const text = new TextDecoder().decode(Array.isArray(data) ? Buffer.concat(data) : data);
      const reply: unknown = JSON.parse(text);
      const id = field(reply, "id");
      if (typeof id === "number") {
        waiting.get(id)?.(reply);
        waiting.delete(id);
      }
    });
    await new Promise<void>((opened, failed) => {
      socket.once("open", () => opened());
      socket.once("error", failed);
    });
    socket.send(
      JSON.stringify({
        jsonrpc: "2.0",
        id: 0,
        method: "hello",
        params: {
          protocol_version: PROTOCOL_VERSION,
          surface: speaking.surface,
          needs: speaking.needs,
          product_version: PRODUCT_VERSION,
        },
      }),
    );
    return new ProgramConnection(socket, await helloed, waiting);
  }

  /** Calls `method` with `params`: its whole reply, its `result` or its `error`. */
  async call(method: string, params: unknown): Promise<unknown> {
    const id = this.#next;
    this.#next += 1;
    const { promise, resolve } = Promise.withResolvers<unknown>();
    this.#waiting.set(id, resolve);
    this.#socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    return promise;
  }

  /** Calls a start and answers the Run ID its result names. */
  async start(method: string, params: unknown): Promise<string> {
    const answer = await this.call(method, params);
    const runId = field(field(answer, "result"), "run_id");
    if (typeof runId !== "string") {
      throw new Error(`${method} started no run: ${JSON.stringify(answer)}`);
    }
    return runId;
  }

  /** Closes its WebSocket. */
  close(): void {
    this.#socket.close();
  }
}

/** The field `name` of `value`, if it is an object that has one. */
export function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** One event the engine sent: the subscription it went to, its position, its name and its data. */
export interface Sent {
  readonly subscription: unknown;
  readonly position: number;
  readonly event: string;
  readonly data: unknown;
}

/** The JSON of a message, if it is text. */
function parsed(message: string | Buffer): unknown {
  return typeof message === "string" ? JSON.parse(message) : undefined;
}

/** The events a `$/events` message carries. */
function eventsOf(message: unknown): Sent[] {
  if (field(message, "method") !== "$/events") {
    return [];
  }
  const params = field(message, "params");
  const events = field(params, "events");
  return Array.isArray(events)
    ? events.map((event: unknown) => ({
        subscription: field(params, "id"),
        position: Number(field(event, "position")),
        event: String(field(event, "event")),
        data: field(event, "data"),
      }))
    : [];
}

/** One WebSocket the page opened, which the test carries to the target. */
export class Carried {
  /** The messages the page sent, as JSON. */
  readonly fromPage: unknown[] = [];
  /** The events the engine sent the page. */
  readonly events: Sent[] = [];
  readonly #page: WebSocketRoute;
  #server: WebSocketRoute | undefined;

  constructor(route: WebSocketRoute, held: Promise<void> | undefined) {
    this.#page = route;
    const waiting: (string | Buffer)[] = [];
    route.onMessage((message) => {
      this.fromPage.push(parsed(message));
      if (this.#server === undefined) {
        waiting.push(message);
      } else {
        this.#server.send(message);
      }
    });
    route.onClose(() => this.#server?.close());
    const connect = (): void => {
      const server = route.connectToServer();
      this.#server = server;
      server.onMessage((message) => {
        this.events.push(...eventsOf(parsed(message)));
        route.send(message);
      });
      server.onClose(() => void route.close());
      for (const message of waiting.splice(0)) {
        server.send(message);
      }
    };
    if (held === undefined) {
      connect();
    } else {
      void held.then(connect);
    }
  }

  /** The params of the page's `events.subscribe` calls on it. */
  get subscriptions(): unknown[] {
    return this.fromPage
      .filter((message) => field(message, "method") === "events.subscribe")
      .map((message) => field(message, "params"));
  }

  /** Drops it, as a network does: both sides close. */
  async drop(): Promise<void> {
    await this.#page.close();
    await this.#server?.close();
  }
}

/** Every WebSocket of the Console's protocol a page opens, carried by the test. */
export class Carrier {
  readonly connections: Carried[] = [];
  #held: Promise<void> | undefined;

  /** Carries every WebSocket `page` opens to `/protocol`. */
  static async on(page: Page): Promise<Carrier> {
    const carrier = new Carrier();
    await page.routeWebSocket(/\/protocol$/, (route) => {
      carrier.connections.push(new Carried(route, carrier.#held));
    });
    return carrier;
  }

  /** Holds each WebSocket opened from now on before it reaches the target, until `release` is called. */
  hold(): () => void {
    const { promise, resolve } = Promise.withResolvers<void>();
    this.#held = promise;
    return () => {
      this.#held = undefined;
      resolve();
    };
  }

  /** The last WebSocket the page sent a `method` call on, once it did. */
  async through(method: string): Promise<Carried> {
    const sent = (): Carried | undefined =>
      this.connections.findLast(({ fromPage }) =>
        fromPage.some((message) => field(message, "method") === method),
      );
    await expect.poll(() => sent() !== undefined).toBe(true);
    const carried = sent();
    if (carried === undefined) {
      throw new Error(`invariant: the page called ${method}`);
    }
    return carried;
  }

  /** The first WebSocket after `earlier` the page subscribed through, once it did. */
  async subscribedAfter(earlier: Carried): Promise<Carried> {
    const later = (): Carried | undefined =>
      this.connections
        .slice(this.connections.indexOf(earlier) + 1)
        .find(({ subscriptions }) => subscriptions.length > 0);
    await expect.poll(() => later() !== undefined).toBe(true);
    const found = later();
    if (found === undefined) {
      throw new Error("invariant: the page subscribed again");
    }
    return found;
  }
}

/** The runs the Runs screen lists, each as `run: lifecycle`, in Run ID order. */
export async function listed(page: Page): Promise<string[]> {
  const rows = await page
    .getByRole("row")
    .filter({ has: page.getByRole("rowheader") })
    .evaluateAll((found) =>
      found.map(
        (row) =>
          `${row.querySelector("th code")?.textContent ?? ""}: ${row.querySelector("td")?.textContent ?? ""}`,
      ),
    );
  return rows.toSorted();
}

/** Opens the Runs screen of the signed-in page once it listed the runs, answering the WebSocket its list came through. */
export async function openRuns(
  page: Page,
  daemon: ServedDaemon,
  carrier: Carrier,
): Promise<Carried> {
  await page.goto(new URL("/runs", daemon.url).href);
  await expect(
    page.getByRole("table").or(page.getByText("No run matches this filter.")),
  ).toBeVisible();
  return carrier.through("run.list");
}

/**
 * Stores `bytes` as a body of the Run `runId`, as the engine does: a file of
 * its folder's `bodies/`, named by their lowercase hex SHA-256. Answers the
 * body's handle, `<run_id>/<body>`.
 */
export async function storeBody(
  daemon: ServedDaemon,
  runId: string,
  bytes: Uint8Array,
): Promise<string> {
  const id = createHash("sha256").update(bytes).digest("hex");
  // Behind a gateway, each engine has a home of its own.
  const engines = daemon.gateway?.engines;
  const homes =
    engines === undefined
      ? [daemon.home]
      : (await readdir(engines)).map((engine) => join(engines, engine));
  const majors = (
    await Promise.all(
      homes.map(async (home) =>
        (await readdir(home).catch(() => [])).map((major) => join(home, major)),
      ),
    )
  ).flat();
  for (const major of majors) {
    const folder = join(major, "runs", runId);
    const exists = await access(folder).then(
      () => true,
      () => false,
    );
    if (exists) {
      await mkdir(join(folder, "bodies"), { recursive: true });
      await writeFile(join(folder, "bodies", id), bytes);
      return `${runId}/${id}`;
    }
  }
  throw new Error(`no home of the test's target holds a Run ${runId}`);
}

/** The `Cookie` header the context sends to the Console. */
export async function cookieHeader(context: BrowserContext, url: string): Promise<string> {
  const cookies = await context.cookies(url);
  return cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
}

/** A message a Client's transport carried, and which way. */
export interface Carriage {
  readonly from: "client" | "engine";
  readonly message: unknown;
}

/**
 * Connections over a WebSocket to the target's `/protocol`, with the
 * sign-in's `cookie` and the Console's own `Origin`, as the page's carry
 * them: the Console's endpoint and authentication, as configuration. Every
 * message either way lands in `carried`.
 */
export function signedInTransport(
  daemon: ServedDaemon,
  cookie: string,
  carried: Carriage[],
): Transport {
  const origin = new URL(daemon.url).origin;
  const url = new URL("/protocol", origin);
  url.protocol = "ws:";
  return {
    open(listener) {
      const socket = new WebSocket(url, { headers: { Cookie: cookie, Origin: origin } });
      const waiting: string[] = [];
      socket.on("open", () => {
        for (const text of waiting.splice(0)) {
          socket.send(text);
        }
      });
      socket.on("message", (data, binary) => {
        if (!binary) {
          const text = new TextDecoder().decode(Array.isArray(data) ? Buffer.concat(data) : data);
          carried.push({ from: "engine", message: JSON.parse(text) });
          listener.message(text);
        }
      });
      socket.on("close", () => listener.close());
      socket.on("error", () => socket.terminate());
      return {
        send(text) {
          carried.push({ from: "client", message: JSON.parse(text) });
          if (socket.readyState === WebSocket.OPEN) {
            socket.send(text);
          } else if (socket.readyState === WebSocket.CONNECTING) {
            waiting.push(text);
          }
        },
        close() {
          socket.close();
        },
      };
    },
  };
}

/** The Console's own Client of the target's build, on the target's endpoint with the page's sign-in. */
export function consoleClient(
  daemon: ServedDaemon,
  cookie: string,
  carried: Carriage[] = [],
): ConsoleClient {
  return createConsoleClient({
    transport: signedInTransport(daemon, cookie, carried),
    bodyFetcher: async () => {
      throw new Error("no case of the Client reads a body");
    },
    visibility: { visible: true, watch: () => () => undefined },
    product: daemon.product,
  });
}

/** Waits until `client` is connected. */
export async function connected(client: ConsoleClient): Promise<void> {
  await expect.poll(() => client.state).toBe("connected");
}

/** The error code of a failed call, from its rejection. */
export async function failureCode(call: Promise<unknown>): Promise<unknown> {
  try {
    await call;
  } catch (error) {
    return typeof error === "object" && error !== null && "code" in error ? error.code : error;
  }
  throw new Error("the call answered where it should have failed");
}

/** Restarts the test's Daemon on the port its Console had, as a restart does: its new engine holds none of the old positions. */
export async function restartedOnItsPort(daemon: ServedDaemon): Promise<void> {
  for (const major of await readdir(daemon.home)) {
    const file = join(daemon.home, major, "configuration.toml");
    const text = await readFile(file, "utf8").catch(() => undefined);
    if (text?.includes("console.address") === true) {
      await writeFile(
        file,
        text.replace(
          /console\.address = "127\.0\.0\.1:0"/,
          `console.address = "127.0.0.1:${String(daemon.port)}"`,
        ),
      );
    }
  }
  await command(daemon, ["daemon", "stop"]);
  await command(daemon, ["daemon", "start"]);
}
