// J-025, "Keep reading the Console through a stopped Daemon and offline,
// then reconnect", on the `console` surface (ADR 0031): a person opens a
// run in the Console, the Daemon stops and the browser goes offline; the
// page turns read-only under its banner, its actions disabled, and keeps
// showing the run's summary, from the browser's offline cache once the
// person moves to the Runs screen and back; brought back online with the
// Daemon started again on its port, the page reconnects by itself and
// shows the run's latest state.
//
// The run's program is the test: the served Daemon has no browser to read
// pages with, so the run reports reading the catalog's first listing page,
// the Journey seed's `catalog/1` task, through its status line.
import type { RunProgress } from "@goodfetch/collie-client/generated";
import { journey } from "@goodfetch/test-support/journey";
import { expect } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";
import { callDaemon } from "@goodfetch/test-support/daemon-calls";

/** The run the Journey reads. */
const RUN_ID = "catalog-run";

/** The banner of a page with no Connection. */
const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** How long a page may take to reconnect to a restarted Daemon: its waits between attempts go up to 10 s. */
const RECONNECTED = 30_000;

/** The term and value of each fact the run's page shows. */
async function facts(page: Page): Promise<Record<string, string>> {
  const terms = await page.getByRole("term").allTextContents();
  const values = await page.getByRole("definition").allTextContents();
  return Object.fromEntries(terms.map((term, index) => [term, values[index] ?? ""]));
}

journey(
  "J-025",
  "console",
  async ({ step, check, cli, daemon, page, context, signIn, failOnErrors }) => {
    const menu = page.getByRole("navigation", { name: "Console" });
    let offlineBanner = "";
    let offlineDisabled: boolean[] = [];
    let offlineFacts: Record<string, string> = {};
    let offlineCopy = false;

    await step("Start a run that reads the catalog", async () => {
      await cli(["run", "start", "--run-id", RUN_ID]);
      await callDaemon(daemon, "run.set_progress_message", {
        run_id: RUN_ID,
        message: "Reading the catalog from its first listing page",
      });
    });

    await step("Open the run in the Console", async () => {
      await signIn.withLink(page);
      await menu.getByRole("link", { name: "Runs" }).click();
      await page.getByRole("link", { name: RUN_ID }).click();
      await expect.poll(async () => (await facts(page))["Lifecycle"]).toBe("running");
    });

    await step("Stop the Daemon", async () => {
      await cli(["daemon", "stop"]);
    });

    await step("Keep reading the run in the Console with the browser offline", async () => {
      await context.setOffline(true);
      await expect(page.getByRole("alert")).toHaveText(BANNER);
      offlineBanner = (await page.getByRole("alert").textContent()) ?? "";
      offlineDisabled = await Promise.all(
        ["Write a Checkpoint", "Stop", "Sign out"].map(async (name) =>
          page.getByRole("button", { name, exact: true }).isDisabled(),
        ),
      );
      await menu.getByRole("link", { name: "Runs" }).click();
      // The Runs screen reads the offline cache after it opens, so the copy
      // shows a moment later: the step waits for it, and records a screen
      // that never shows it for the check to fail.
      offlineCopy = await expect(page.getByText("This browser's offline copy", { exact: false }))
        .toBeVisible()
        .then(
          () => true,
          () => false,
        );
      await page.getByRole("link", { name: RUN_ID }).click();
      await expect.poll(async () => (await facts(page))["Lifecycle"]).toBe("running");
      offlineFacts = await facts(page);
    });

    await step("Bring the browser back online and start the Daemon again", async () => {
      await context.setOffline(false);
      await cli(["daemon", "start"]);
    });

    await check(
      "While the Daemon is stopped, the Console shows a banner saying the page is read-only, and its actions are disabled",
      async () => {
        expect(offlineBanner).toBe(BANNER);
        expect(offlineDisabled).toEqual([true, true, true]);
      },
    );

    await check("The run's summary stays readable while the browser is offline", async () => {
      expect(offlineCopy).toBe(true);
      expect(offlineFacts).toMatchObject({ Lifecycle: "running" });
    });

    await check(
      "Once the Daemon runs again, the Console reconnects by itself, the banner goes, and the run shows its latest state",
      async () => {
        await expect(page.getByRole("alert")).toHaveCount(0, { timeout: RECONNECTED });
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `run progress --json` prints `goodfetch-protocol`'s type.
        const latest = JSON.parse(
          (await cli(["run", "progress", RUN_ID, "--json"])).stdout,
        ) as RunProgress;
        await expect.poll(async () => (await facts(page))["Lifecycle"]).toBe(latest.lifecycle);
        // Each attempt to reconnect while the Daemon was stopped, or the browser offline, failed.
        expect(
          failOnErrors.errors.every(
            ({ text, url }) =>
              /\/protocol|ERR_CONNECTION_REFUSED|ERR_INTERNET_DISCONNECTED|ERR_FAILED/.test(text) ||
              url.includes("/protocol"),
          ),
        ).toBe(true);
        await failOnErrors.take();
      },
    );
  },
  // The open page keeps its address across the Daemon's stop and start.
  { namedConsolePort: true },
);
