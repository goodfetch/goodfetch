// Vitest for the whole pnpm workspace (TESTING.md: No retries, Before merge,
// Time budgets; ADRs 0061, 0062 and 0064). Unit tests sit beside their module
// and take the Quick suite's caps; integration tests under `tests/` take the
// caps above it. CI runs with `CI` set, so a missing or mismatched snapshot
// fails and none is written.
//
// Screenshot tests (`*.visual.test.ts[x]`) form the `visual` project, which
// exists only inside the pinned Playwright image (ADR 0073; ci/visual/), where
// ci/visual/run.sh sets GOODFETCH_VISUAL_IMAGE. Reference screenshots are
// named without the platform, and each one a test reads is recorded under
// target/visual/read/ for the orphan check (`xtask screenshots orphans`).
import { appendFileSync, mkdirSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { playwright } from "@vitest/browser-playwright";
import {
  defineConfig,
  type TestProjectInlineConfiguration,
  type ViteUserConfig,
} from "vitest/config";

const outside = [
  "**/node_modules/**",
  ".*/**",
  "**/fixtures/**",
  "rnd/**",
  "ci/proofs/**",
  "target/**",
];
const visualTests = "**/*.visual.test.{ts,tsx}";

const visual: TestProjectInlineConfiguration = {
  extends: true,
  test: {
    name: "visual",
    include: [visualTests],
    exclude: outside,
    testTimeout: 300_000,
    // A failed comparison's actual and diff images, uploaded by the visual job.
    attachmentsDir: "target/vitest-attachments",
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: "chromium" }],
      expect: {
        toMatchScreenshot: {
          resolveScreenshotPath: (reference) => {
            const path = resolve(
              reference.root,
              reference.testFileDirectory,
              reference.screenshotDirectory,
              reference.testFileName,
              `${reference.arg}-${reference.browserName}${reference.ext}`,
            );
            const records = join(reference.root, "target", "visual", "read");
            mkdirSync(records, { recursive: true });
            const line = relative(reference.root, path).split(sep).join("/");
            appendFileSync(join(records, `vitest-${process.pid}.txt`), `${line}\n`);
            return path;
          },
        },
      },
    },
  },
};

const config: ViteUserConfig = defineConfig({
  test: {
    retry: 0,
    restoreMocks: true,
    unstubEnvs: true,
    unstubGlobals: true,
    sequence: { shuffle: { tests: true } },
    passWithNoTests: true,
    // The Quick suite passes `--slowTestThreshold=1000`; Vitest sets it per run, not per project.
    slowTestThreshold: 30_000,
    reporters: ["default", "junit"],
    outputFile: { junit: "target/junit/vitest.xml" },
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          include: ["**/src/**/*.test.{ts,tsx}"],
          exclude: [...outside, visualTests],
          testTimeout: 10_000,
        },
      },
      {
        extends: true,
        test: {
          name: "integration",
          include: ["**/tests/**/*.test.{ts,tsx}"],
          exclude: [...outside, visualTests],
          testTimeout: 300_000,
        },
      },
      ...(process.env["GOODFETCH_VISUAL_IMAGE"] === undefined ? [] : [visual]),
    ],
  },
});

export default config;
