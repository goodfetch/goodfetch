// The Runs screen at scale (spec #592, story 136; ADR 0071), on Collie's
// executable built from the commit, under its production headers, with a
// Daemon and home of its own on port 0. The test carries the page's
// WebSocket to the Daemon with Playwright's `routeWebSocket`, as a proxy
// does, and stands in for a busy Daemon there: it answers `run.list` with
// 10,000 runs, of which the page renders only the rows in view, and streams
// 10,000 events into the screen's subscription. The JS heap after a forced
// garbage collection, read over the Chrome DevTools Protocol, stays within
// 10 % of its value after the first 1,000 events: a ratio, so the machine
// does not matter. The first reading waits for the screen to have written
// the list to the offline cache, whose 10,000 encryptions in flight would
// otherwise swell it.
import type { Page, WebSocketRoute } from "@playwright/test";
import type { RunIndexRow } from "@goodfetch/collie-client/generated";
import { expect, test } from "@goodfetch/test-support/playwright";

/** How many runs the Run index lists. */
const RUNS = 10_000;

/** How many events the test streams, and after how many it reads the heap first. */
const EVENTS = 10_000;
const FIRST = 1_000;

/**
 * How many events a `$/events` batch carries at most: the test sends one
 * once the page granted credits for that many, half its window.
 */
const BATCH = 32;

/** How far the heap may grow from its reading after the first events (ADR 0071). */
const GROWTH = 1.1;

/**
 * How long the screen may take to show the busy Daemon's list and to write
 * it to the offline cache: as long as the test's own timeout gives it
 * (`playwright.config.ts`), as Playwright reads a timeout of 0. One task of
 * the page's parses the 10,000 runs and starts their 10,000 writes, each
 * signed and encrypted, before the screen renders them, so how long both take
 * follows the host's load, and no figure of the product or the test bounds
 * them.
 */
const LISTED = 0;

/**
 * How long the screen may take to show the last of the events streamed: each
 * credit the page grants crosses the test's WebSocket route, a few
 * milliseconds each.
 */
const STREAMED = 120_000;

/** How many garbage collections each heap reading forces. */
const COLLECTIONS = 3;

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

/** The Run ID of the `index`th run, in the order the list answers them. */
function runId(index: number): string {
  return `run-${String(index).padStart(5, "0")}`;
}

/** The Run index of a busy Daemon: `RUNS` live runs, newest first. */
function runIndex(): RunIndexRow[] {
  return Array.from({ length: RUNS }, (_, index) => ({
    run_id: runId(index),
    location: `runs/${runId(index)}`,
    kind: "collie",
    needs: "collie",
    tags: [],
    lifecycle: "running",
    started_at: "2026-10-01T08:00:00Z",
    presence: "present",
    replayed: false,
  }));
}

/** The `index`th event the test streams: a `run.progress` of one of the runs in view, its status line naming it. */
function event(index: number): unknown {
  return {
    // After any position the Daemon itself reached in the test.
    position: 1_000_000 + index,
    event: "run.progress",
    data: {
      run_id: runId(index % 10),
      position: index,
      at: "2026-10-01T08:05:00Z",
      lifecycle: "running",
      writer: "live",
      epoch: 1,
      pauses: [],
      message: `Event ${String(index)}`,
      last: false,
    },
  };
}

/**
 * One subscription of the page's: the credits of its first grant, its
 * window; those it granted and not yet spent; and how many events it was sent.
 */
interface Subscribed {
  window: number;
  credits: number;
  sent: number;
}

/**
 * The page's WebSocket to the Daemon, carried by the test, which answers
 * every `run.list` with the busy Daemon's index and can stream events into
 * the subscriptions of the page's last WebSocket: the Runs screen's and the
 * shell's "Needs you", which ignores runs' events. It streams as a Daemon
 * does, within the credits each subscription granted (spec #216); once it
 * streams, those credits are the test's, since the Daemon sends no event.
 */
class BusyDaemon {
  readonly #lists = new Set<unknown>();
  #subscriptions = new Map<unknown, Subscribed>();
  #route: WebSocketRoute | undefined;
  #streaming = false;
  #until = 0;

  /** Carries every WebSocket `page` opens to `/protocol`. */
  static async on(page: Page): Promise<BusyDaemon> {
    const busy = new BusyDaemon();
    await page.routeWebSocket(/\/protocol$/, (route) => {
      busy.#route = route;
      busy.#subscriptions = new Map();
      const server = route.connectToServer();
      route.onMessage((message) => {
        const sent = parsed(message);
        const method = field(sent, "method");
        if (method === "run.list") {
          busy.#lists.add(field(sent, "id"));
        } else if (method === "events.subscribe") {
          busy.#subscriptions.set(field(sent, "id"), { window: 0, credits: 0, sent: 0 });
        } else if (method === "$/credit") {
          const params = field(sent, "params");
          const subscribed = busy.#subscriptions.get(field(params, "id"));
          if (subscribed !== undefined) {
            const credits = Number(field(params, "credits"));
            subscribed.window ||= credits;
            subscribed.credits += credits;
          }
          if (busy.#streaming) {
            busy.#pump();
            return;
          }
        }
        server.send(message);
      });
      server.onMessage((message) => {
        const answer = parsed(message);
        const id = field(answer, "id");
        if (id !== undefined && busy.#lists.delete(id) && field(answer, "result") !== undefined) {
          route.send(JSON.stringify({ jsonrpc: "2.0", id, result: { runs: runIndex() } }));
          return;
        }
        route.send(message);
      });
      route.onClose(() => void server.close());
      server.onClose(() => void route.close());
    });
    return busy;
  }

  /** How many subscriptions to the event stream the page's last WebSocket carries. */
  get subscriptions(): number {
    return this.#subscriptions.size;
  }

  /** Streams the events up to `until` (excluded) into each of the page's subscriptions. */
  stream(until: number): void {
    if (this.#route === undefined || this.#subscriptions.size === 0) {
      throw new Error("invariant: the page subscribed before the test streams");
    }
    this.#streaming = true;
    this.#until = until;
    this.#pump();
  }

  /**
   * Whether every subscription read the events up to `until`: it was sent
   * them and granted its window again, a credit for each event it read.
   */
  read(until: number): boolean {
    return [...this.#subscriptions.values()].every(
      ({ window, credits, sent }) => sent >= until && credits >= window,
    );
  }

  /** Sends each subscription the events its credits allow, in batches. */
  #pump(): void {
    const route = this.#route;
    if (route === undefined) {
      return;
    }
    for (const [id, subscribed] of this.#subscriptions) {
      for (;;) {
        const count = Math.min(BATCH, this.#until - subscribed.sent);
        if (count <= 0 || subscribed.credits < count) {
          break;
        }
        const events = Array.from({ length: count }, (_, offset) =>
          event(subscribed.sent + offset),
        );
        subscribed.sent += count;
        subscribed.credits -= count;
        route.send(JSON.stringify({ jsonrpc: "2.0", method: "$/events", params: { id, events } }));
      }
    }
  }
}

/**
 * How many rows the page's offline cache holds: the screen keeps each run it
 * lists there, written after it shows them.
 */
async function offlineRows(page: Page): Promise<number> {
  return page.evaluate(
    async (name) =>
      new Promise<number>((resolve, reject) => {
        const opening = indexedDB.open(name);
        opening.addEventListener("error", () => reject(opening.error ?? new Error(name)));
        opening.addEventListener("success", () => {
          const database = opening.result;
          const counting = database.transaction("rows").objectStore("rows").count();
          counting.addEventListener("success", () => {
            database.close();
            resolve(counting.result);
          });
          counting.addEventListener("error", () => reject(counting.error ?? new Error(name)));
        });
      }),
    "goodfetch-console-offline",
  );
}

/** The Run IDs of the rows the table renders. */
async function renderedRows(page: Page): Promise<string[]> {
  return page
    .getByRole("rowheader")
    .evaluateAll((cells) => cells.map((cell) => cell.querySelector("code")?.textContent ?? ""));
}

test(
  "a 10,000-row Runs list renders only its rows in view, and 10,000 events leave the heap within 10 % of its size after 1,000",
  { annotation: { type: "covers", description: "story:12-136" } },
  async ({ page, daemon, signIn }) => {
    const busy = await BusyDaemon.on(page);
    await signIn.withLink(page);
    await page.goto(new URL("/runs", daemon.url).href);

    // The screen has listed the runs once its table names their count.
    await expect(page.getByRole("table", { name: "Runs" })).toHaveAttribute(
      "aria-rowcount",
      String(RUNS + 1),
      { timeout: LISTED },
    );
    await expect.poll(() => renderedRows(page)).toContain(runId(0));
    const rendered = await renderedRows(page);
    expect(rendered.length).toBeLessThan(100);
    expect(rendered).not.toContain(runId(RUNS - 1));
    // The shell's "Needs you" and the Runs screen.
    await expect.poll(() => busy.subscriptions).toBe(2);

    // The heap is read once the screen's own writes of the list are done.
    await expect.poll(() => offlineRows(page), { timeout: LISTED }).toBeGreaterThanOrEqual(RUNS);
    const heap = await page.context().newCDPSession(page);
    /**
     * The JS heap's used bytes after forced garbage collections, once the
     * page is idle: work still in flight holds memory a collection cannot
     * free, and one collection leaves some that the next frees.
     */
    const collected = async (): Promise<number> => {
      await page.evaluate(
        async () => new Promise((idle) => requestIdleCallback(() => idle(undefined))),
      );
      for (let pass = 0; pass < COLLECTIONS; pass += 1) {
        await heap.send("HeapProfiler.collectGarbage");
      }
      const { usedSize } = await heap.send("Runtime.getHeapUsage");
      return usedSize;
    };
    /** Streams the events up to `until`, once the screen shows the last one. */
    const streamed = async (until: number): Promise<void> => {
      busy.stream(until);
      await expect.poll(() => busy.read(until), { timeout: STREAMED }).toBe(true);
      await expect(page.getByText(`Event ${String(until - 1)}`, { exact: true })).toBeVisible();
    };

    await streamed(FIRST);
    const afterFirst = await collected();
    await streamed(EVENTS);
    const afterAll = await collected();

    expect(afterAll).toBeLessThanOrEqual(afterFirst * GROWTH);
    expect((await renderedRows(page)).length).toBeLessThan(100);
  },
);
