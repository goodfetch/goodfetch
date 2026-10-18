// Playwright Test for the whole pnpm workspace (TESTING.md: No retries and
// Time budgets; ADRs 0064 and 0068). Its tests run above the Quick suite, so
// they take its caps: slow after 30 s, killed at 5 min.
//
// Screenshot tests (`*.visual.spec.ts`) form the `visual` project, which
// exists only inside the pinned Playwright image (ADR 0073; ci/visual/), where
// ci/visual/run.sh sets GOODFETCH_VISUAL_IMAGE. No suite writes a Reference
// screenshot (`updateSnapshots: 'none'`); only the `update-screenshots`
// workflow does, and references are named without the platform.
import { defineConfig, type PlaywrightTestConfig, type Project } from "@playwright/test";

const ignored = ["**/node_modules/**", ".*/**", "**/fixtures/**", "rnd/**", "ci/proofs/**"];

const visual: Project = {
  name: "visual",
  testMatch: "**/*.visual.spec.ts",
  testIgnore: ignored,
  use: {
    browserName: "chromium",
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 1,
    locale: "en-US",
    timezoneId: "UTC",
    reducedMotion: "reduce",
  },
};

const config: PlaywrightTestConfig = defineConfig({
  testMatch: "**/*.spec.ts",
  testIgnore: ignored,
  outputDir: "target/playwright",
  retries: 0,
  failOnFlakyTests: true,
  fullyParallel: true,
  timeout: 300_000,
  reportSlowTests: { max: 0, threshold: 30_000 },
  reporter: [["list"], ["junit", { outputFile: "target/junit/playwright.xml" }]],
  updateSnapshots: "none",
  expect: {
    toHaveScreenshot: {
      pathTemplate: "{snapshotDir}/{testFileDir}/{testFileName}-snapshots/{arg}{-projectName}{ext}",
    },
  },
  projects: [
    { name: "default", testIgnore: [...ignored, "**/*.visual.spec.ts"] },
    ...(process.env["GOODFETCH_VISUAL_IMAGE"] === undefined ? [] : [visual]),
  ],
});

export default config;
