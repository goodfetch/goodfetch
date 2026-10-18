// Playwright Test for the whole pnpm workspace (TESTING.md: No retries and
// Time budgets; ADRs 0064 and 0068). Its tests run above the Quick suite, so
// they take its caps: slow after 30 s, killed at 5 min.
import { defineConfig, type PlaywrightTestConfig } from "@playwright/test";

const config: PlaywrightTestConfig = defineConfig({
  testMatch: "**/*.spec.ts",
  testIgnore: ["**/node_modules/**", "rnd/**", "ci/proofs/**"],
  outputDir: "target/playwright",
  retries: 0,
  failOnFlakyTests: true,
  fullyParallel: true,
  timeout: 300_000,
  reportSlowTests: { max: 0, threshold: 30_000 },
  reporter: [["list"], ["junit", { outputFile: "target/junit/playwright.xml" }]],
});

export default config;
