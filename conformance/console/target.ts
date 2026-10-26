// What the Console conformance cases ask of their target beside the page
// (ADRs 0031 and 0068): its executable's commands in the test's home, a
// Run's stored file, the page's WebSocket carried through the test as a
// network or a proxy carries it, and the Console's own Client reaching the
// target's endpoint with the page's sign-in, as configuration.
import { createHash } from "node:crypto";
import { access, mkdir, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { BrowserContext, Page, WebSocketRoute } from "@playwright/test";
import { expect } from "@goodfetch/test-support/conformance";
import { command } from "@goodfetch/test-support/daemon-calls";
import type { ServedDaemon } from "@goodfetch/test-support/playwright";
import { WebSocket } from "ws";
import {
  createConsoleClient,
  type ConsoleClient,
  type Transport,
} from "@goodfetch/console/src/client/client.ts";

/** The banner of a page with no Connection to its engine. */
export const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** Starts the Run `runId` in the test's home with the executable's `run start`. */
export async function startRun(daemon: ServedDaemon, runId: string): Promise<void> {
  await command(daemon, ["run", "start", "--run-id", runId]);
}

/** The field `name` of `value`, if it is an object that has one. */
export function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** One event the engine sent: its position, its name and its data. */
export interface Sent {
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
  const events = field(field(message, "params"), "events");
  return Array.isArray(events)
    ? events.map((event: unknown) => ({
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
  for (const major of await readdir(daemon.home)) {
    const folder = join(daemon.home, major, "runs", runId);
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
  throw new Error(`the test's home holds no Run ${runId}`);
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

/** The Console's own Client of Collie's build, on the target's endpoint with the page's sign-in. */
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
    product: "collie",
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
