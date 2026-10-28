// The Console and the Daemon's life on Collie's executable built from the
// commit (spec #591, stories 6 and 8; ADRs 0031 and 0041), each test with a
// Daemon and home of its own on port 0: a visible Console tab keeps the
// Daemon from stopping idle and a hidden one does not; and the Console's
// first launch offers a monthly Limit for the install, which accepted shows
// in the Usage ledger, and which declined is not offered again.
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { UsageListResult } from "@goodfetch/collie-client/generated";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";

/** The idle time the tests give the Daemon, in milliseconds. */
const IDLE = 2000;

/** How long a Daemon may take to stop once nothing keeps it: its idle time, then its stop. */
const STOPS = 30_000;

/**
 * Sets `daemon.idle_timeout` to `IDLE` in the test home's user configuration
 * and pins the Console's address to the port it serves on now, then
 * restarts the Daemon, which reads both at its start.
 */
async function restartedWithAShortIdleTime(daemon: ServedDaemon): Promise<void> {
  let written = false;
  for (const major of await readdir(daemon.home)) {
    const file = join(daemon.home, major, "configuration.toml");
    const text = await readFile(file, "utf8").catch(() => undefined);
    if (text?.includes("console.address") === true) {
      await writeFile(
        file,
        text.replace(
          /console\.address = "127\.0\.0\.1:0"/,
          `console.address = "127.0.0.1:${String(daemon.port)}"\ndaemon.idle_timeout = ${String(IDLE)}`,
        ),
      );
      written = true;
    }
  }
  expect(written, "the test's home has a user configuration").toBe(true);
  for (const args of [
    ["daemon", "stop"],
    ["daemon", "start"],
  ]) {
    const output = await daemon.run(args);
    expect(output.code, output.stderr).toBe(0);
  }
}

/** Whether the test home's Daemon runs, as `daemon status --json` says. */
async function running(daemon: ServedDaemon): Promise<boolean> {
  const output = await daemon.run(["daemon", "status", "--json"]);
  expect(output.code, output.stderr).toBe(0);
  const status: unknown = JSON.parse(output.stdout);
  return typeof status === "object" && status !== null && "pid" in status && status.pid !== null;
}

test(
  "a visible Console tab keeps the Daemon running past its idle time, and a hidden one does not",
  { annotation: [{ type: "covers", description: "story:12-6" }] },
  async ({ page, daemon, signIn }) => {
    await restartedWithAShortIdleTime(daemon);
    await signIn.withLink(page);
    await expect(page.getByText("Connected to Collie", { exact: false })).toBeVisible();

    // The Daemon runs on, the tab visible, past three idle times. Each
    // `daemon status` is a Client of its own, so the checks come further
    // apart than the idle time: between two of them, only the tab is.
    const shown = Date.now();
    await expect
      .poll(async () => (await running(daemon)) && Date.now() - shown > 3 * IDLE, {
        timeout: STOPS,
        intervals: [3 * IDLE],
      })
      .toBe(true);

    // The page turns hidden, as a tab behind another does; headless Chromium
    // keeps every page visible, and no Playwright or CDP call hides one.
    await page.evaluate(() => {
      // oxlint-disable-next-line no-restricted-properties -- the only way to hide a page in headless Chromium; the Console reads `document.visibilityState` and its change event (ADR 0031).
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        get: () => "hidden",
      });
      document.dispatchEvent(new Event("visibilitychange"));
    });

    await expect.poll(() => running(daemon), { timeout: STOPS, intervals: [3 * IDLE] }).toBe(false);
  },
);

/** The Overview's offer of a monthly install Limit. */
function offer(page: Page) {
  return page.getByRole("region", { name: "Cap this install's spending" });
}

/** The Limits `usage list --json` lists in the test's ledger. */
async function limits(daemon: ServedDaemon): Promise<UsageListResult["limits"]> {
  const output = await daemon.run(["usage", "list", "--json"]);
  expect(output.code, output.stderr).toBe(0);
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `usage list --json` prints `goodfetch-protocol`'s type, which the executable's own tests check against the bundle.
  return (JSON.parse(output.stdout) as UsageListResult).limits;
}

test(
  "the first launch offers a monthly install Limit, which accepted shows in the ledger",
  { annotation: [{ type: "covers", description: "story:12-8" }] },
  async ({ page, daemon, signIn }) => {
    await signIn.withLink(page);
    await expect(offer(page)).toBeVisible();

    await page.getByRole("textbox", { name: "Each month, at most (USD)" }).fill("25");
    await page.getByRole("button", { name: "Set the monthly Limit" }).click();

    await expect(page.getByRole("status").filter({ hasText: "spending is capped" })).toBeVisible();
    const set = await limits(daemon);
    expect(
      set.map(({ limit }) => [
        limit.scope.kind,
        limit.usage_kind,
        limit.currency,
        limit.period?.every,
        limit.amount,
        limit.set_by,
      ]),
    ).toEqual([["install", "paid_solves", "USD", "month", 25_000_000, "person"]]);
    await page.goto(new URL("/settings", daemon.url).href);
    await expect(
      page
        .getByRole("table", { name: "Usage against each Limit" })
        .getByRole("rowheader", { name: "Install" }),
    ).toBeVisible();
  },
);

test(
  "a declined offer of a monthly install Limit is not made again on the next visit",
  { annotation: [{ type: "covers", description: "story:12-8" }] },
  async ({ page, daemon, signIn }) => {
    await signIn.withLink(page);
    await expect(offer(page)).toBeVisible();

    await page.getByRole("button", { name: "Not now" }).click();
    await expect(offer(page)).toHaveCount(0);
    await page.reload();
    await expect(page.getByText("Connected to Collie", { exact: false })).toBeVisible();

    await expect(offer(page)).toHaveCount(0);
    expect(await limits(daemon)).toEqual([]);
  },
);
