// A crawl run's request log, live on its page (spec #593, story 219; ADRs
// 0079, 0102 and 0109), on Foxhound's executable built from the commit,
// with a Daemon and home of its own on port 0 and the Proving ground beside
// it. The test serves a keep-alive crawl of four articles of the `pacing`
// family and holds its handler until the log is open, so each outcome shows
// as it commits: one article succeeds, and three fail with `handler_failed`
// carrying the exception a TypeScript, a Python and a Rust handler send.
// The log's filters run in the engine, and each failed attempt shows the
// exception's type, message and stack, none for Rust.
import { connectDaemon, EngineRequestFailure } from "@goodfetch/test-support/daemon-calls";
import { expect, test } from "@goodfetch/test-support/playwright";

/** The Proving ground's seed. */
const SEED = 897;

/** The `pacing` family's host. */
const HOST = "pacing.test";

const RUN_ID = "failing-crawl";

/** How long a crawl of four pages may take under load. */
const CRAWLED = 90_000;

const TYPESCRIPT_STACK =
  "TypeError: Cannot read properties of undefined (reading 'price')\n    at handle (/app/crawl.ts:12:18)";
const PYTHON_STACK =
  "Traceback (most recent call last):\n  File \"/app/crawl.py\", line 9, in handle\nKeyError: 'price'";

/** The exception each article's handler fails with, as each SDK sends it; none for the first. */
const EXCEPTIONS: Readonly<Record<string, Readonly<Record<string, string>>>> = {
  "2": {
    exception_type: "TypeError",
    exception_message: "Cannot read properties of undefined (reading 'price')",
    exception_stack: TYPESCRIPT_STACK,
  },
  "3": {
    exception_type: "KeyError",
    exception_message: "'price'",
    exception_stack: PYTHON_STACK,
  },
  "4": {
    exception_type: "BoxError",
    exception_message: "invalid digit found in string",
    exception_stack: "",
  },
};

/** The article number of a `crawl_request.handle`'s request. */
function articleOf(params: unknown): string {
  const request =
    typeof params === "object" && params !== null && "crawl_request" in params
      ? params.crawl_request
      : undefined;
  const url =
    typeof request === "object" && request !== null && "url" in request ? request.url : "";
  return String(url).split("/").at(-1) ?? "";
}

test.use({ product: "foxhound", provingGroundSeed: SEED });

test(
  "a crawl run's request log shows each outcome as it commits, filters in the engine, and each handler's exception",
  {
    annotation: [
      { type: "covers", description: "story:12-219" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    test.setTimeout(240_000);
    const provingGround = daemon.provingGround;
    if (provingGround === undefined) {
      throw new Error("the test's Daemon has no Proving ground");
    }
    const port = new URL(`http://${provingGround.address}`).port;
    const { promise: opened, resolve: release } = Promise.withResolvers<void>();
    const connection = await connectDaemon(daemon, "foxhound", {
      "crawl_request.handle": async (params) => {
        await opened;
        const exception = EXCEPTIONS[articleOf(params)];
        if (exception === undefined) {
          return { records: [] };
        }
        throw new EngineRequestFailure(
          "handler_failed",
          `the handler threw ${exception["exception_type"] ?? ""}`,
          exception,
        );
      },
    });
    try {
      await connection.call("crawl.start", {
        run_id: RUN_ID,
        seeds: [1, 2, 3, 4].map((article) => ({
          url: `http://${HOST}:${port}/articles/${String(article)}`,
          label: article === 1 ? "list" : "page",
        })),
        settings: {
          fetch: { tier: "http" },
          network: { resolve: { [HOST]: [provingGround.resolve[HOST]] } },
          egress: { private_ranges: [HOST] },
          crawl: { keep_alive: true },
        },
      });

      await signIn.withLink(page);
      await page.goto(new URL(`/runs/${RUN_ID}`, daemon.url).href);
      await page.getByRole("tab", { name: "Outcomes", exact: true }).click();
      const panel = page.getByRole("tabpanel", { name: "Outcomes", exact: true });
      await expect(
        panel.getByText("Waiting for the first outcome the crawl commits…"),
      ).toBeVisible();

      release();
      await expect(panel.getByText("4 outcomes Following the crawl live.")).toBeVisible({
        timeout: CRAWLED,
      });
      const log = panel.getByRole("table", { name: "Request log" });
      const row = (article: number) =>
        log.getByRole("row").filter({ hasText: `/articles/${String(article)}` });
      await expect(row(1)).toContainText("Succeeded");
      await expect(row(2).locator('dl[aria-label="Attempt 1"]')).toContainText(TYPESCRIPT_STACK);
      await expect(row(3).locator('dl[aria-label="Attempt 1"]')).toContainText("KeyError");
      await expect(row(3).locator("pre")).toHaveText(PYTHON_STACK);
      await expect(row(4).locator('dl[aria-label="Attempt 1"]')).toContainText(
        "BoxErrorException messageinvalid digit found in stringStackNo stack",
      );

      await panel.getByRole("checkbox", { name: "Failed" }).check();
      await panel.getByRole("textbox", { name: "Labels, separated by commas" }).fill("page");
      await panel
        .getByRole("textbox", { name: "Error codes, separated by commas" })
        .fill("handler_failed");
      await panel.getByRole("button", { name: "Apply" }).click();
      await expect(panel.getByText("3 outcomes Following the crawl live.")).toBeVisible();
      await expect(row(1)).toHaveCount(0);

      await connection.call("run.finish", { run_id: RUN_ID });
      await expect(panel.getByText("3 outcomes The crawl ended.")).toBeVisible({
        timeout: CRAWLED,
      });
    } finally {
      connection.close();
    }
  },
);
