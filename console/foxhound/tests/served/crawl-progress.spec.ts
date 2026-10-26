// A crawl run's progress, live on its page (spec #593, story 217; ADRs
// 0102 and 0118; ADR 0037 / RT-15), on Foxhound's executable built from the
// commit, with a Daemon and home of its own on port 0 and the Proving
// ground beside it. The test serves a keep-alive crawl of three articles of
// the `pacing` family, whose host holds each answer about 4.9 s at the time
// step the test sets, one request at a time on the domain and 2 s between
// starts: the crawl's Progress tab shows the request in flight with its
// phase, tier and attempt, the requests the domain holds by
// `politeness.domain.concurrency` and then by `politeness.domain.delay`,
// each with the domain, when it is next admitted and the setting with its
// value and layer, the program's status message and its handlers against
// their concurrency, then the crawl idle once its work is done, and ended
// once the program finishes it.
import { connectDaemon } from "@goodfetch/test-support/daemon-calls";
import { expect, test } from "@goodfetch/test-support/playwright";

/** The Proving ground's seed. */
const SEED = 897;

/** The `pacing` family's host. */
const HOST = "pacing.test";

/** The time step whose answers the host holds about 4.9 s. */
const SLOW = 245;

const RUN_ID = "slow-crawl";

/** How long the test's handler holds each page, so its phase shows. */
const HANDLING_MS = 1_500;

/** How long a crawl of three slow pages, 2 s apart, may take under load. */
const CRAWLED = 90_000;

test.use({ product: "foxhound", provingGroundSeed: SEED });

test(
  "a crawl run's Progress tab shows its requests in flight, why due requests wait, its status message and handlers, and its idle state, live",
  {
    annotation: [
      { type: "covers", description: "story:12-217" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    test.setTimeout(240_000);
    const provingGround = daemon.provingGround;
    if (provingGround === undefined) {
      throw new Error("the test's Daemon has no Proving ground");
    }
    const stepped = await fetch(`http://${provingGround.address}/time-step`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ step: SLOW }),
    });
    expect(stepped.ok).toBe(true);
    const port = new URL(`http://${provingGround.address}`).port;
    const connection = await connectDaemon(daemon, "foxhound", {
      "crawl_request.handle": async () => {
        await new Promise((resolve) => setTimeout(resolve, HANDLING_MS));
        return { records: [] };
      },
    });
    try {
      await connection.call("crawl.start", {
        run_id: RUN_ID,
        seeds: [1, 2, 3].map((article) => ({
          url: `http://${HOST}:${port}/articles/${String(article)}`,
          label: "page",
        })),
        settings: {
          fetch: { tier: "http" },
          network: { resolve: { [HOST]: [provingGround.resolve[HOST]] } },
          egress: { private_ranges: [HOST] },
          crawl: { handler: { concurrency: 2 }, keep_alive: true },
          politeness: { domain: { concurrency: 1, delay: 2_000 } },
        },
      });
      await connection.call("run.set_progress_message", {
        run_id: RUN_ID,
        message: "Crawling three articles",
      });

      await signIn.withLink(page);
      await page.goto(new URL(`/runs/${RUN_ID}`, daemon.url).href);
      await page.getByRole("tab", { name: "Progress", exact: true }).click();
      const panel = page.getByRole("tabpanel", { name: "Progress", exact: true });

      await expect(panel.getByText("Crawling three articles")).toBeVisible();
      const inFlight = panel.getByRole("table", { name: "Requests in flight" });
      const fetching = inFlight.getByRole("row").filter({ hasText: "Fetching" });
      await expect(fetching).toHaveCount(1, { timeout: CRAWLED });
      await expect(fetching.getByRole("cell")).toHaveText([
        new RegExp(`/articles/\\d$`, "u"),
        "Fetching",
        "http",
        "1",
        /.+/u,
      ]);

      const heldByConcurrency = panel.getByRole("region", {
        name: /^Domain concurrency: \d+ requests? waits?$/u,
      });
      await expect(heldByConcurrency).toBeVisible({ timeout: CRAWLED });
      await expect(heldByConcurrency.getByRole("row").filter({ hasText: HOST })).toContainText(
        "Once a request in flight ends",
      );
      await expect(heldByConcurrency).toContainText(
        "Set by politeness.domain.concurrency: 1, from Run",
      );

      await expect(inFlight.getByRole("cell", { name: "Handling", exact: true })).toBeVisible({
        timeout: CRAWLED,
      });
      await expect(panel.getByRole("region", { name: "Concurrency", exact: true })).toContainText(
        /Handlers running\s*\d of 2/u,
      );

      const heldByDelay = panel.getByRole("region", {
        name: /^Domain delay: \d+ requests? waits?$/u,
      });
      await expect(heldByDelay).toBeVisible({ timeout: CRAWLED });
      await expect(
        heldByDelay.getByRole("row").filter({ hasText: HOST }).locator("time"),
      ).toHaveAttribute("datetime", /^\d{4}-\d\d-\d\dT/u);
      await expect(heldByDelay).toContainText("Set by politeness.domain.delay: 2000, from Run");

      await expect(
        panel.getByText("Idle: the crawl waits for new work, held running by crawl.keep_alive."),
      ).toBeVisible({ timeout: CRAWLED });
      await expect(panel.getByText("No request is in flight.")).toBeVisible();

      await connection.call("run.finish", { run_id: RUN_ID });
      await expect(
        panel.getByText("The crawl ended: nothing is in flight or waits any more."),
      ).toBeVisible({ timeout: CRAWLED });
    } finally {
      connection.close();
    }
  },
);
