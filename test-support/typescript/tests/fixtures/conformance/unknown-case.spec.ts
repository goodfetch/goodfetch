// A test whose title names no case of its capability in the fixture case
// list: the reporter refuses it and fails the run, though the test passes.
import { test } from "@goodfetch/test-support/conformance";
import { standInConsole } from "../../console-stand-in.ts";

test.use({ consoleServer: standInConsole() });

test(
  "a-case-the-list-does-not-declare",
  { annotation: { type: "capability", description: "passing" } },
  async ({ daemon, page }) => {
    await page.goto(daemon.url);
  },
);
