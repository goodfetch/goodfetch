// A Vitest Browser Mode project of two shuffled tests in one file, run by
// tests/hover-reset.test.ts with a chosen order seed, with or without the
// shared setup's `hoverReset` (GOODFETCH_FIXTURE_HOVER_RESET=on or off). It
// prints one `ran: <test> <state>` line per test, in the order they ran, and
// writes its caches under GOODFETCH_FIXTURE_OUTPUT, never beside itself.
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig, type ViteUserConfig } from "vitest/config";
import type { Reporter, TestCase } from "vitest/node";

const setup = fileURLToPath(new URL("../../../src/vitest-browser-setup.ts", import.meta.url));
const output = process.env["GOODFETCH_FIXTURE_OUTPUT"];
if (output === undefined) {
  throw new Error("GOODFETCH_FIXTURE_OUTPUT names the folder this fixture writes in");
}

const runOrder: Reporter = {
  onTestCaseResult(testCase: TestCase) {
    process.stdout.write(`ran: ${testCase.name} ${testCase.result().state}\n`);
  },
};

const config: ViteUserConfig = defineConfig({
  cacheDir: join(output, "vite"),
  test: {
    include: ["hover.test.ts"],
    retry: 0,
    sequence: { shuffle: { tests: true } },
    setupFiles: process.env["GOODFETCH_FIXTURE_HOVER_RESET"] === "on" ? [setup] : [],
    reporters: [runOrder],
    attachmentsDir: join(output, "attachments"),
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: "chromium" }],
      screenshotFailures: false,
    },
  },
});

export default config;
