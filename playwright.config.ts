// Playwright Test for the whole pnpm workspace (TESTING.md: No retries and
// Time budgets; ADRs 0064 and 0068). Its tests run above the Quick suite, so
// they take its caps: slow after 30 s, killed at 5 min.
//
// Screenshot tests (`*.visual.spec.ts`) form the `visual` project, which
// exists only inside the pinned Playwright image (ADR 0073; ci/visual/), where
// ci/visual/run.sh sets GOODFETCH_VISUAL_IMAGE. No suite writes a Reference
// screenshot (`updateSnapshots: 'none'`); only the `update-screenshots`
// workflow does, and references are named without the platform.
//
// A test declares a Feature with a `covers` annotation (ADR 0072). The
// configuration reads the Feature inventory, validated against its schema,
// and passes its IDs as `metadata.features`, so test support's `test` fails a
// test whose annotation names an unknown Feature.
import { sep } from "node:path";
import { readFeatureIds } from "@goodfetch/test-support/features";
import type { ServedOptions } from "@goodfetch/test-support/playwright";
import { defineConfig, type PlaywrightTestConfig, type Project } from "@playwright/test";

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

// The Release suite runs every test on every browser Playwright ships; the
// Pre-merge suite on Chromium alone (TESTING.md, Browsers). Its translation
// project reruns the served tests with test support's translation-emulation
// script on every page, which wraps each new text node in `<font>` as page
// translators do; they must still pass, with no uncaught error (ADR 0073).
const releaseProjects: Project<ServedOptions>[] =
  process.env["GOODFETCH_TEST_SUITE"] === "release"
    ? [
        ...(["firefox", "webkit"] as const).map((browserName) => ({
          name: browserName,
          testIgnore: [...ignored, "**/*.visual.spec.ts"],
          use: { browserName },
        })),
        {
          name: "translation",
          testIgnore: [...ignored, "**/*.visual.spec.ts"],
          use: { translationEmulation: true },
        },
      ]
    : [];

const config: PlaywrightTestConfig<ServedOptions> = defineConfig<ServedOptions>({
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
  updateSnapshots: "none",
  metadata: { features: readFeatureIds(import.meta.dirname) },
  use: { trace: "retain-on-failure" },
  expect: {
    toHaveScreenshot: {
      pathTemplate: "{snapshotDir}/{testFileDir}/{testFileName}-snapshots/{arg}{-projectName}{ext}",
    },
  },
  projects: [
    { name: "default", testIgnore: [...ignored, "**/*.visual.spec.ts"] },
    ...releaseProjects,
    ...(process.env["GOODFETCH_VISUAL_IMAGE"] === undefined ? [] : [visual]),
  ],
});

export default config;
