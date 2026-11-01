// The storage entries of "Needs you" served by Collie's executable (spec
// #592; ADR 0122; pitfall O03's "Needs you" part), on a size-capped
// filesystem of the test's own holding its Goodfetch home and run folders,
// with `storage.low_free` and `storage.critical_free` set against its size
// and filler files to cross them: the filesystem at `low`, then at
// `critical`, is listed with its folder and counted in the side menu, the
// run the critical level stops is listed beside it, freeing space takes the
// level off the list and the run's resume takes the run off, each as it
// happens, with no reload.
import { appendFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";

const MIB = 1 << 20;

/** The size-capped filesystem's size, and the two levels against it. */
const SIZE = 64 * MIB;
const LEVELS = `storage.low_free = ${String(40 * MIB)}\nstorage.critical_free = ${String(20 * MIB)}\n`;

/** Free space between the two levels, and below `critical`. */
const LOW_ROOM = 30 * MIB;
const CRITICAL_ROOM = 10 * MIB;

/** How long a level change may take to reach the page: a sample a second, then a run's next safe point. */
const SAMPLED = 30_000;

test.use({ sizeCapped: SIZE });

/** Adds `settings` to the user configuration of the test's home, which the governor reads again at each sample. */
async function configure(daemon: ServedDaemon, settings: string): Promise<void> {
  const capped = daemon.sizeCapped;
  if (capped === undefined) {
    throw new Error("the test's folder is on no size-capped filesystem");
  }
  const home = capped.outside(daemon.home);
  for (const entry of await readdir(home, { withFileTypes: true })) {
    if (
      entry.isDirectory() &&
      (await readdir(join(home, entry.name))).includes("configuration.toml")
    ) {
      await appendFile(join(home, entry.name, "configuration.toml"), settings);
    }
  }
}

/** Runs the executable's `args` in the test's home, failing the test when it fails. */
async function run(daemon: ServedDaemon, args: readonly string[]): Promise<void> {
  const output = await daemon.run(args);
  expect(output.code, output.stderr).toBe(0);
}

/** The side menu's link to "Needs you". */
function menuLink(page: Page) {
  return page.getByRole("navigation", { name: "Console" }).getByRole("link", { name: /Needs you/ });
}

/** What "Needs you" lists. */
function items(page: Page) {
  return page.getByRole("list", { name: "Needs you" }).getByRole("listitem");
}

test.describe("on a size-capped filesystem", () => {
  test.skip(
    process.platform !== "linux",
    "a size-capped filesystem is a tmpfs on Linux; macOS's and Windows's small disk image is not made yet",
  );

  test(
    "Needs you lists the low and critical storage levels with their folder and the run they stopped, each leaving once resolved",
    {
      annotation: [
        { type: "covers", description: "story:12-23" },
        { type: "covers", description: "story:12-25" },
        { type: "covers", description: "story:12-32" },
        { type: "covers", description: "screen:/needs-you" },
        { type: "covers", description: "operation:needs_you.list" },
        { type: "covers", description: "event:needs_you.changed" },
      ],
    },
    async ({ page, daemon, signIn }) => {
      const capped = daemon.sizeCapped;
      expect(capped).toBeDefined();
      if (capped === undefined) {
        return;
      }
      await configure(daemon, LEVELS);
      await run(daemon, ["run", "start", "--run-id", "filled"]);
      await signIn.withLink(page);
      await page.goto(new URL("/needs-you", daemon.url).href);
      await expect(page.getByText("Nothing needs you now.")).toBeVisible();

      await capped.fill(daemon.home, LOW_ROOM);

      await expect(items(page)).toHaveCount(1, { timeout: SAMPLED });
      await expect(items(page).nth(0)).toContainText("low storage level");
      await expect(items(page).nth(0)).toContainText(daemon.home);
      await expect(menuLink(page)).toHaveText("Needs you1");

      await capped.fill(daemon.home, CRITICAL_ROOM);

      const stopped = items(page).filter({ hasText: "filled" });
      await expect(stopped).toHaveCount(1, { timeout: SAMPLED });
      await expect(stopped).toContainText("its disk filled");
      const level = items(page).filter({ hasText: "critical storage level" });
      await expect(level).toContainText(daemon.home);
      await expect(menuLink(page)).toHaveText("Needs you2");

      await capped.free();

      await expect(level).toHaveCount(0, { timeout: SAMPLED });
      await expect(items(page)).toHaveCount(1);
      await expect(menuLink(page)).toHaveText("Needs you1");

      await run(daemon, ["run", "resume", "filled"]);

      await expect(page.getByText("Nothing needs you now.")).toBeVisible({ timeout: SAMPLED });
      await expect(menuLink(page)).toHaveText("Needs you");
    },
  );
});
