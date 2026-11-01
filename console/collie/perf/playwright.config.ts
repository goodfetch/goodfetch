// The Console's lab metrics for the Performance series (ADRs 0071 and 0076),
// apart from every suite: they time, so no suite runs them, and only `just
// console-lighthouse` reads this configuration. Its one file takes the
// served fixtures' Daemon, from the test tool manifest GOODFETCH_TEST_TOOLS
// names.
import type { ServedOptions } from "@goodfetch/test-support/playwright";
import { defineConfig, type PlaywrightTestConfig } from "@playwright/test";

const config: PlaywrightTestConfig<ServedOptions> = defineConfig<ServedOptions>({
  testDir: import.meta.dirname,
  testMatch: "*.perf.ts",
  outputDir: "../../../target/perf-series/playwright",
  retries: 0,
  workers: 1,
  timeout: 600_000,
  reporter: [["list"]],
});

export default config;
