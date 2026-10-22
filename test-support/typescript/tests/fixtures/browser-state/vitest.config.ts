// A Vitest Browser Mode project whose provider sets a colour scheme and
// reduced motion other than Playwright's defaults, run by
// tests/browser-state-reset.test.ts, so `browserStateReset` is seen giving
// back the project's values rather than the defaults. It prints one
// `ran: <test> <state>` line per test, and writes its caches under
// GOODFETCH_FIXTURE_OUTPUT, never beside itself.
import { join } from "node:path";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig, type ViteUserConfig } from "vitest/config";
import type { Reporter, TestCase } from "vitest/node";
import { browserCommands } from "../../../src/vitest-browser-commands.ts";

const output = process.env["GOODFETCH_FIXTURE_OUTPUT"];
if (output === undefined) {
  throw new Error("GOODFETCH_FIXTURE_OUTPUT names the folder this fixture writes in");
}

const runOrder: Reporter = {
  onTestCaseResult(testCase: TestCase) {
    const errors = testCase
      .result()
      .errors?.map((error) => error.message)
      .join("; ");
    process.stdout.write(
      `ran: ${testCase.name} ${testCase.result().state}${errors ? ` (${errors})` : ""}\n`,
    );
  },
};

const config: ViteUserConfig = defineConfig({
  cacheDir: join(output, "vite"),
  test: {
    include: ["reset.test.ts"],
    retry: 0,
    reporters: [runOrder],
    attachmentsDir: join(output, "attachments"),
    browser: {
      enabled: true,
      headless: true,
      provider: playwright({
        contextOptions: { colorScheme: "dark", reducedMotion: "reduce" },
      }),
      instances: [{ browser: "chromium" }],
      commands: browserCommands,
      screenshotFailures: false,
    },
  },
});

export default config;
