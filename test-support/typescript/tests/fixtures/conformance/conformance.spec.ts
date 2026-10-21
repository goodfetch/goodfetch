// Two cases of the fixture case list against the stand-in Console: one
// whose check holds, one whose check fails. The capability `untested` has
// no test.
import { expect, test } from "@goodfetch/test-support/conformance";
import { standInConsole } from "../../console-stand-in.ts";

test.use({ consoleServer: standInConsole() });

test(
  "the-console-names-the-daemons-product",
  { annotation: { type: "capability", description: "passing" } },
  async ({ daemon, page }) => {
    await page.goto(daemon.url);
    await expect(page.getByRole("heading", { level: 1 })).toHaveText(/'s Daemon$/);
  },
);

test(
  "the-console-names-nobody",
  { annotation: { type: "capability", description: "failing" } },
  async ({ daemon, page }) => {
    await page.goto(daemon.url);
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Nobody's Daemon", {
      timeout: 1_000,
    });
  },
);
