// What the Console needs from its target for "Needs you" (ADRs 0031, 0068
// and 0122): an item the engine raises reaches the open page as it comes,
// with no reload, is counted in the side menu, and leaves once a person
// dismisses it from the Console. The items come from a size-capped
// filesystem holding the test's Goodfetch home: once its free space is
// below `storage.critical_free`, the target lists that level and stops the
// running Run, which it lists too, dismissable.
import { appendFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "@goodfetch/test-support/conformance";
import { NOT_ON_THE_STUB_GATEWAY, skipBehindGateway } from "./target.ts";

const MIB = 1 << 20;

/** The size-capped filesystem's size, and a `critical` level above all its free space. */
const SIZE = 64 * MIB;
const CRITICAL_EVERYWHERE = `storage.low_free = ${String(128 * MIB)}\nstorage.critical_free = ${String(100 * MIB)}\n`;

/** How long a level change may take to reach the page: a sample a second. */
const SAMPLED = 30_000;

test.use({ sizeCapped: SIZE });

/** What "Needs you" lists. */
function items(page: Page) {
  return page.getByRole("list", { name: "Needs you" }).getByRole("listitem");
}

/** The side menu's link to "Needs you". */
function menuLink(page: Page) {
  return page.getByRole("navigation", { name: "Console" }).getByRole("link", { name: /Needs you/ });
}

test(
  "needs-you-lists-an-item-as-it-comes-and-drops-it-once-dismissed",
  {
    annotation: [
      { type: "capability", description: "needs-you" },
      { type: "covers", description: "story:12-23" },
      { type: "covers", description: "story:12-32" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.sizeCapped);
    test.skip(
      process.platform !== "linux",
      "a size-capped filesystem is a tmpfs on Linux; macOS's and Windows's small disk image is not made yet",
    );
    const capped = daemon.sizeCapped;
    if (capped === undefined) {
      throw new Error("invariant: the test asked for a size-capped filesystem");
    }
    // Inside the filesystem's namespaces, where the test's home is.
    const started = await daemon.run(["run", "start", "--run-id", "filled"]);
    expect(started.code, started.stderr).toBe(0);
    await signIn.withLink(page);
    await page.goto(new URL("/needs-you", daemon.url).href);
    await expect(page.getByText("Nothing needs you now.")).toBeVisible();

    // The governor reads the user configuration again at each sample.
    const home = capped.outside(daemon.home);
    for (const entry of await readdir(home, { withFileTypes: true })) {
      if (
        entry.isDirectory() &&
        (await readdir(join(home, entry.name))).includes("configuration.toml")
      ) {
        await appendFile(join(home, entry.name, "configuration.toml"), CRITICAL_EVERYWHERE);
      }
    }

    const stopped = items(page).filter({ hasText: "filled" });
    await expect(stopped).toHaveCount(1, { timeout: SAMPLED });
    await expect(items(page).filter({ hasText: "critical storage level" })).toHaveCount(1);
    await expect(menuLink(page)).toHaveText("Needs you2");

    await stopped.getByRole("button", { name: "Dismiss" }).click();

    await expect(stopped).toHaveCount(0);
    await expect(items(page)).toHaveCount(1);
    await expect(menuLink(page)).toHaveText("Needs you1");
  },
);
