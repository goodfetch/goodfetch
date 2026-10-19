// A fixture workspace's runner config. Its tolerances (maxDiffPixels and
// threshold among them) come from the shared visual config; the slow-test
// report's threshold is no screenshot tolerance.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testMatch: "**/*.spec.ts",
  reportSlowTests: { max: 0, threshold: 30_000 },
});
