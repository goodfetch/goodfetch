// What the Console needs from its target's events and Connections (ADRs
// 0031 and 0068), on the page as users get it: the Runs screen lists the
// Run index; a page that lost its Connection is read-only under a banner,
// reconnects by itself and resubscribes after the last position it read,
// or gets a snapshot when the engine no longer holds it; every lossless
// event reaches the page, in position order; and with WebSockets refused
// the Console works over Server-Sent Events plus `POST`. The test carries
// the page's WebSocket as a network or a proxy does, and only drops or
// holds the target's own messages.
import { expect, test } from "@goodfetch/test-support/conformance";
import { eventClasses } from "@goodfetch/console/generated/operations";
import {
  BANNER,
  Carrier,
  field,
  finishRun,
  listed,
  openRuns,
  PRODUCT_NAME,
  NOT_ON_THE_STUB_GATEWAY,
  restartedOnItsPort,
  skipBehindGateway,
  startRun,
} from "./target.ts";

// The page of the case of a restart stays open across it.
test.use({ namedConsolePort: true });

/** How long a page may take to reconnect to a restarted Daemon: its waits between attempts go up to 10 s. */
const RECONNECTED = 30_000;

test(
  "the-runs-screen-lists-every-run-of-the-index",
  {
    annotation: [
      { type: "capability", description: "runs.list" },
      { type: "covers", description: "story:12-10" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await startRun(daemon, "running");
    await startRun(daemon, "finished");
    await finishRun(daemon, "finished");
    await signIn.withLink(page);

    await page.goto(new URL("/runs", daemon.url).href);

    await expect.poll(() => listed(page)).toEqual(["finished: finished", "running: running"]);
  },
);

test(
  "a-resubscription-resumes-after-the-last-position-read",
  {
    annotation: [
      { type: "capability", description: "events.resumption" },
      { type: "covers", description: "story:12-123" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.fanIn);
    const carrier = await Carrier.on(page);
    await signIn.withLink(page);
    const first = await openRuns(page, daemon, carrier);
    await expect.poll(() => first.subscriptions.length).toBeGreaterThan(0);
    await startRun(daemon, "first");
    await expect.poll(() => listed(page)).toEqual(["first: running"]);
    const lastRead = first.events.at(-1);

    const release = carrier.hold();
    await first.drop();
    await startRun(daemon, "second");
    release();

    await expect.poll(() => listed(page)).toEqual(["first: running", "second: running"]);
    const second = await carrier.subscribedAfter(first);
    expect(second.subscriptions.at(-1)).toEqual({
      after: { engine_instance: expect.any(String), position: lastRead?.position },
    });
    expect(second.events[0]?.position).toBe((lastRead?.position ?? 0) + 1);
    expect(second.events.map(({ event }) => event)).not.toContain("events.resync");
  },
);

test(
  "a-position-the-engine-no-longer-holds-answers-a-snapshot",
  {
    annotation: [
      { type: "capability", description: "events.resumption" },
      { type: "covers", description: "story:12-123" },
    ],
  },
  async ({ page, daemon, signIn, failOnErrors }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.restart);
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

    await expect
      .poll(
        () =>
          carrier.connections.some(({ events }) =>
            events.some(({ event }) => event === "events.resync"),
          ),
        { timeout: RECONNECTED },
      )
      .toBe(true);
    await startRun(daemon, "second");
    await expect.poll(() => listed(page)).toContain("second: running");
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
  "every-lossless-event-reaches-the-page-in-position-order",
  {
    annotation: [
      { type: "capability", description: "events.lossless" },
      { type: "covers", description: "story:12-14" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.fanIn);
    const runs = ["run-1", "run-2", "run-3", "run-4", "run-5", "run-6"];
    const carrier = await Carrier.on(page);
    await signIn.withLink(page);
    const carried = await openRuns(page, daemon, carrier);
    await expect.poll(() => carried.subscriptions.length).toBeGreaterThan(0);

    await Promise.all(runs.map(async (runId) => startRun(daemon, runId)));
    for (const runId of runs) {
      await finishRun(daemon, runId);
    }

    await expect.poll(() => listed(page)).toEqual(runs.map((runId) => `${runId}: finished`));
    // The Runs screen's own subscription, beside the side menu's.
    const subscription = carried.events.findLast(
      ({ event }) => event === "run.axes_changed",
    )?.subscription;
    const read = carried.events.filter((sent) => sent.subscription === subscription);
    const positions = read.map(({ position }) => position);
    expect(positions).toEqual(positions.toSorted((a, b) => a - b));
    expect(new Set(positions).size).toBe(positions.length);
    const lossless = read.filter(({ event }) => field(eventClasses, event) === "lossless");
    for (const runId of runs) {
      const ofRun = lossless.filter(({ data }) => JSON.stringify(data).includes(`"${runId}"`));
      // Its start and its end, each a change of its axes.
      expect(ofRun.length, runId).toBeGreaterThanOrEqual(2);
    }
  },
);

test(
  "a-dropped-connection-turns-the-page-read-only-until-it-reconnects",
  {
    annotation: [
      { type: "capability", description: "reconnection" },
      { type: "covers", description: "story:12-125" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const carrier = await Carrier.on(page);
    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    const explain = page.getByRole("button", { name: "Explain" });
    await expect(explain).toBeEnabled();
    const first = await carrier.through("config.explain");

    // The banner alone: the page's other alerts, such as the Operator
    // policy's read failing with `person_required` behind the Stub gateway,
    // come and go with the calls each Connection answers.
    const banner = page.getByRole("alert").filter({ hasText: BANNER });
    const release = carrier.hold();
    await first.drop();

    await expect(banner).toHaveText(BANNER);
    await expect(explain).toBeDisabled();
    release();
    await expect(banner).toHaveCount(0);
    await expect(explain).toBeEnabled();
    expect(carrier.connections.length).toBeGreaterThan(1);
  },
);

test(
  "with-websockets-refused-the-console-works-over-server-sent-events-plus-post",
  {
    annotation: [
      { type: "capability", description: "event-stream-fallback" },
      { type: "covers", description: "story:12-124" },
      { type: "covers", description: "story:12-110" },
    ],
  },
  async ({ page, daemon, signIn, failOnErrors }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.eventStream);
    await page.routeWebSocket(/\/protocol$/, (route) => void route.close());
    const posted: { url: string; csrf: string | undefined }[] = [];
    page.on("request", (request) => {
      if (request.method() === "POST" && request.url().includes("/protocol/messages/")) {
        posted.push({ url: request.url(), csrf: request.headers()["x-csrf-token"] });
      }
    });

    await signIn.withLink(page);
    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      new RegExp(`^Connected to ${PRODUCT_NAME[daemon.product]} `),
    );
    await page.goto(new URL("/runs", daemon.url).href);
    await startRun(daemon, "first");
    await expect.poll(() => listed(page)).toEqual(["first: running"]);

    expect(posted.length).toBeGreaterThan(0);
    expect(posted.every(({ csrf }) => csrf !== undefined && csrf !== "")).toBe(true);
    const stream = posted.at(-1)?.url ?? "";
    const refused = await page.evaluate(
      async (url) => (await fetch(url, { method: "POST", body: "{}" })).status,
      stream,
    );
    expect(refused).toBe(403);
    await expect.poll(() => failOnErrors.errors.length).toBeGreaterThan(0);
    await failOnErrors.take();
  },
);
