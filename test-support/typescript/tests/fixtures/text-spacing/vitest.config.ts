// A Vitest Browser Mode project set up as the Release suite's
// `component-text-spacing` project is, run by tests/text-spacing-setup.test.ts:
// two screen-like tests that assert nothing themselves. It prints one
// `ran: <test> <state> <codes>` line per test, `<codes>` being the `code` of
// each error that failed it, and writes its caches under
// GOODFETCH_FIXTURE_OUTPUT, never beside itself.
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig, type ViteUserConfig } from "vitest/config";
import type { Reporter, TestCase } from "vitest/node";

const setup = fileURLToPath(new URL("../../../src/vitest-browser-setup.ts", import.meta.url));
const textSpacing = fileURLToPath(
  new URL("../../../src/vitest-browser-text-spacing.ts", import.meta.url),
);
const output = process.env["GOODFETCH_FIXTURE_OUTPUT"];
if (output === undefined) {
  throw new Error("GOODFETCH_FIXTURE_OUTPUT names the folder this fixture writes in");
}

/** The `code` of an error Vitest serialized, when it has one. */
function codeOf(error: object): string {
  return "code" in error && typeof error.code === "string" ? error.code : "no-code";
}

const outcomes: Reporter = {
  onTestCaseResult(testCase: TestCase) {
    const result = testCase.result();
    const codes = (result.errors ?? []).map(codeOf).join(",");
    process.stdout.write(`ran: ${testCase.name} ${result.state} ${codes}\n`);
  },
};

const config: ViteUserConfig = defineConfig({
  cacheDir: join(output, "vite"),
  test: {
    include: ["screens.test.ts"],
    retry: 0,
    setupFiles: [setup, textSpacing],
    reporters: [outcomes],
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
