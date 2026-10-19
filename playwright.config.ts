// Playwright Test for the whole pnpm workspace (TESTING.md: No retries and
// Time budgets; ADRs 0064 and 0068). Its tests run above the Quick suite, so
// they take its caps: slow after 30 s, killed at 5 min.
//
// Screenshot tests (`*.visual.spec.ts`) form the `visual` project, which
// test support's shared visual config gives only inside the pinned Playwright
// image (ADR 0073; ci/visual/). No suite writes a Reference screenshot; only
// the `update-screenshots` workflow does.
import { sep } from "node:path";
import { defineConfig, type PlaywrightTestConfig, type Project } from "@playwright/test";
import { PLAYWRIGHT_VISUAL_TESTS, visualConfig } from "@goodfetch/test-support/visual-config";

// Playwright matches a glob against a test file's absolute path, from any
// folder (it prefixes `**/`), so a glob of a folder at the repository's root
// would also ignore a checkout inside such a folder (a worktree under
// `.claude/`): those folders are matched from the root, as a pattern.
const root = import.meta.dirname
  .split(sep)
  .join("/")
  .replaceAll(/[.*+?^${}()|[\]\\]/g, "\\$&");
const ignored = [
  "**/node_modules/**",
  "**/fixtures/**",
  new RegExp(`^${root}/(?:\\.[^/]+|rnd|ci/proofs)/`),
];

// The Release suite runs every test on every browser Playwright ships; the
// Pre-merge suite on Chromium alone (TESTING.md, Browsers).
const releaseBrowsers: Project[] =
  process.env["GOODFETCH_TEST_SUITE"] === "release"
    ? (["firefox", "webkit"] as const).map((browserName) => ({
        name: browserName,
        testIgnore: [...ignored, PLAYWRIGHT_VISUAL_TESTS],
        use: { browserName },
      }))
    : [];

const visual = visualConfig.playwright(process.env, { ignore: ignored });

const config: PlaywrightTestConfig = defineConfig({
  testMatch: "**/*.spec.ts",
  testIgnore: ignored,
  // Each test's output, a failed test's trace among it, sits beside the JUnit
  // files, which every job uploads pass or fail (ADR 0068).
  outputDir: "target/junit/playwright",
  retries: 0,
  failOnFlakyTests: true,
  fullyParallel: true,
  timeout: 300_000,
  reportSlowTests: { max: 0, threshold: 30_000 },
  reporter: [["list"], ["junit", { outputFile: "target/junit/playwright.xml" }]],
  updateSnapshots: visual.updateSnapshots,
  use: { trace: "retain-on-failure" },
  projects: [
    { name: "default", testIgnore: [...ignored, PLAYWRIGHT_VISUAL_TESTS] },
    ...releaseBrowsers,
    ...visual.projects,
  ],
});

export default config;
