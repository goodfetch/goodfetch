// Reconnecting without losing anything (spec #592; ADR 0031), on Collie's
// executable built from the commit, under its production headers, each test
// with a Daemon and home of its own on a named port. The tests stand between the
// page and the Daemon with Playwright's `routeWebSocket`, as a network or a
// proxy does: a dropped or stalled WebSocket turns the page read-only under
// a banner, its actions disabled, until it reconnects by itself and
// resubscribes after the last event it read, missing no lossless event; a
// Daemon restarted, whose engine holds none of the old positions, answers
// with a resync snapshot, on which the Runs screen reads its list again; and
// with WebSockets refused the Console works over Server-Sent Events plus
// `POST`, each `POST` refused without its CSRF token.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import type { Page, WebSocketRoute } from "@playwright/test";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import { effectiveSettings } from "#tests/effective-settings.ts";

const run = promisify(execFile);

// The page of the test of a restart stays open across it.
test.use({ namedConsolePort: true });

/** The banner of a page with no Connection to its engine. */
const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** How long the Console waits on a silent Connection before it asks, then before it gives up. */
const STALL = 10_000;

/** How long a page may take to reconnect to a restarted Daemon: its waits between attempts go up to 10 s. */
const RECONNECTED = 30_000;

/** Runs the executable of the test's Daemon with `args` in its home. */
async function command(daemon: ServedDaemon, args: readonly string[]): Promise<string> {
  const { stdout } = await run(daemon.executable, [...args], { env: { ...daemon.env } });
  return stdout;
}

/** Starts the Run `runId` in the test's home with the executable's `run start`. */
async function startRun(daemon: ServedDaemon, runId: string): Promise<void> {
  await command(daemon, ["run", "start", "--run-id", runId]);
}

/** One event the engine sent: its position and its name. */
interface Sent {
  readonly position: number;
  readonly event: string;
}

/** The JSON of a WebSocket message, if it is text. */
function parsed(message: string | Buffer): unknown {
  return typeof message === "string" ? JSON.parse(message) : undefined;
}

/** The field `name` of `value`, if it is an object that has one. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
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
      }))
    : [];
}

/** One WebSocket the page opened, which the test carries to the Daemon. */
class Carried {
  /** The messages the page sent, as JSON. */
  readonly fromPage: unknown[] = [];
  /** The events the engine sent the page. */
  readonly events: Sent[] = [];
  #stalled = false;
  readonly #page: WebSocketRoute;
  #server: WebSocketRoute | undefined;

  constructor(route: WebSocketRoute, held: Promise<void> | undefined) {
    this.#page = route;
    const waiting: (string | Buffer)[] = [];
    route.onMessage((message) => {
      if (this.#stalled) {
        return;
      }
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
        if (this.#stalled) {
          return;
        }
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

  /** Stalls it: from now on nothing reaches either side. */
  stall(): void {
    this.#stalled = true;
  }
}

/** Every WebSocket of the Console's protocol a page opens, carried by the test. */
class Carrier {
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

  /** Holds each WebSocket opened from now on before it reaches the Daemon, until `release` is called. */
  hold(): () => void {
    const { promise, resolve } = Promise.withResolvers<void>();
    this.#held = promise;
    return () => {
      this.#held = undefined;
      resolve();
    };
  }

  /**
   * The last WebSocket the page sent a `method` call on, once it did: the
   * one a screen read through. The order the WebSockets opened in names
   * none, since the page before may still have the last one that sent
   * something while the screen's says `hello`.
   */
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
}

/** The runs the Runs screen lists, each as `run: lifecycle`, in Run ID order. */
async function listed(page: Page): Promise<string[]> {
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

/** The first WebSocket after `earlier` the page subscribed through, once it did. */
async function subscribedAfter(carrier: Carrier, earlier: Carried): Promise<Carried> {
  const later = (): Carried | undefined =>
    carrier.connections
      .slice(carrier.connections.indexOf(earlier) + 1)
      .find(({ subscriptions }) => subscriptions.length > 0);
  await expect.poll(() => later() !== undefined).toBe(true);
  const found = later();
  if (found === undefined) {
    throw new Error("invariant: the page subscribed again");
  }
  return found;
}

/**
 * Opens the Runs screen of the signed-in page, once it has listed the runs
 * (none yet), answering the WebSocket its list came through. The first
 * screen's WebSocket subscribes too, for the shell's "Needs you", so the
 * last WebSocket to have subscribed may be that closed one while the Runs
 * screen's still says `hello`.
 */
async function openRuns(page: Page, daemon: ServedDaemon, carrier: Carrier): Promise<Carried> {
  await page.goto(new URL("/runs", daemon.url).href);
  await expect(page.getByText("No run matches this filter.")).toBeVisible();
  return carrier.through("run.list");
}

test(
  "a dropped WebSocket turns the page read-only, then it reconnects after its last event and misses none",
  {
    annotation: [
      { type: "covers", description: "story:12-123" },
      { type: "covers", description: "story:12-125" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const carrier = await Carrier.on(page);
    await signIn.withLink(page);
    const first = await openRuns(page, daemon, carrier);
    await expect.poll(() => first.subscriptions.length).toBeGreaterThan(0);
    await startRun(daemon, "first");
    await expect.poll(() => listed(page)).toEqual(["first: running"]);
    const lastRead = first.events.at(-1);

    const release = carrier.hold();
    await first.drop();
    await expect(page.getByRole("alert")).toHaveText(BANNER);
    await startRun(daemon, "second");
    await startRun(daemon, "third");
    release();

    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect
      .poll(() => listed(page))
      .toEqual(["first: running", "second: running", "third: running"]);
    const second = await subscribedAfter(carrier, first);
    expect(second.subscriptions.at(-1)).toEqual({
      after: { engine_instance: expect.any(String), position: lastRead?.position },
    });
    const resumed = second.events.filter(({ event }) => event === "run.axes_changed");
    expect(resumed.length).toBeGreaterThanOrEqual(2);
    expect(resumed[0]?.position).toBe((lastRead?.position ?? 0) + 1);
    expect(second.events.map(({ event }) => event)).not.toContain("events.resync");
  },
);

test(
  "a stalled WebSocket disables the page's actions under the banner until a new one answers",
  {
    annotation: [
      { type: "covers", description: "story:12-125" },
      { type: "covers", description: "screen:/settings" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await page.clock.install();
    const carrier = await Carrier.on(page);
    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    const explain = page.getByRole("button", { name: "Explain" });
    await expect(effectiveSettings(page)).toBeVisible();
    await expect(explain).toBeEnabled();
    const first = await carrier.through("config.explain");

    const release = carrier.hold();
    first.stall();
    await page.clock.runFor(3 * STALL);
    await expect(page.getByRole("alert")).toHaveText(BANNER);
    await expect(explain).toBeDisabled();
    release();

    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(explain).toBeEnabled();
    expect(carrier.connections.length).toBeGreaterThan(1);
  },
);

/** Restarts the test's Daemon on its Console's named port, as a handover or a restart does. */
async function restartedOnItsPort(daemon: ServedDaemon): Promise<void> {
  await command(daemon, ["daemon", "stop"]);
  await command(daemon, ["daemon", "start"]);
}

test(
  "after a Daemon restart, whose engine holds none of the old positions, the page receives a resync snapshot",
  { annotation: { type: "covers", description: "story:12-123" } },
  async ({ page, daemon, signIn, failOnErrors }) => {
    const carrier = await Carrier.on(page);
    // While the Daemon is stopped, a refused WebSocket turns the Client to
    // Server-Sent Events, which the test cannot read: refusing them keeps
    // the reconnection on a WebSocket it carries.
    await page.route(/\/protocol\/events$/, (route) => route.abort());
    await signIn.withLink(page);
    const first = await openRuns(page, daemon, carrier);
    await expect.poll(() => first.subscriptions.length).toBeGreaterThan(0);
    await startRun(daemon, "first");
    await expect.poll(() => listed(page)).toEqual(["first: running"]);

    await restartedOnItsPort(daemon);

    // The new engine answers the resubscription with its snapshot, on which
    // the screen reads the Run index again; the run started after it is
    // listed too.
    await expect
      .poll(
        () =>
          carrier.connections.some(({ events }) =>
            events.some(({ event }) => event === "events.resync"),
          ),
        {
          timeout: RECONNECTED,
        },
      )
      .toBe(true);
    await expect(page.getByRole("alert")).toHaveCount(0);
    await startRun(daemon, "fourth");
    await expect.poll(() => listed(page)).toContain("fourth: running");
    // Each attempt while the Daemon was stopped failed, as the browser says.
    expect(
      failOnErrors.errors.every(
        ({ text, url }) =>
          /\/protocol|ERR_CONNECTION_REFUSED|ERR_FAILED/.test(text) ||
          url.includes("/protocol/events"),
      ),
    ).toBe(true);
    await failOnErrors.take();
  },
);

test(
  "with WebSockets refused, the Console works over Server-Sent Events plus POST, each POST with its CSRF token",
  {
    annotation: [
      { type: "covers", description: "story:12-124" },
      { type: "covers", description: "story:12-110" },
    ],
  },
  async ({ page, daemon, signIn, failOnErrors }) => {
    await page.routeWebSocket(/\/protocol$/, (route) => void route.close());
    const posted: { url: string; csrf: string | undefined }[] = [];
    page.on("request", (request) => {
      if (request.method() === "POST" && request.url().includes("/protocol/messages/")) {
        posted.push({ url: request.url(), csrf: request.headers()["x-csrf-token"] });
      }
    });

    await signIn.withLink(page);
    await expect(page.getByRole("main").getByRole("status")).toHaveText(/^Connected to Collie /);
    await page.goto(new URL("/runs", daemon.url).href);
    await startRun(daemon, "first");
    await expect.poll(() => listed(page)).toEqual(["first: running"]);
    await page.goto(new URL("/settings", daemon.url).href);
    await expect(effectiveSettings(page)).toBeVisible();

    expect(posted.length).toBeGreaterThan(0);
    expect(posted.every(({ csrf }) => csrf !== undefined && csrf !== "")).toBe(true);
    const stream = posted.at(-1)?.url ?? "";
    const refused = await page.evaluate(
      async (url) => (await fetch(url, { method: "POST", body: "{}" })).status,
      stream,
    );
    expect(refused).toBe(403);
    await expect.poll(() => failOnErrors.errors.length).toBeGreaterThan(0);
    expect(failOnErrors.errors.map(({ kind }) => kind).toSorted()).toEqual([
      "console",
      "request_failed",
    ]);
    await failOnErrors.take();
  },
);
