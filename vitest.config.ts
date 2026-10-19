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
import { browserCommands } from "@goodfetch/test-support/vitest-browser/commands";
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

// A `.test.tsx` file renders or needs the DOM, so it runs in Vitest Browser
// Mode with the Playwright provider (ADR 0068; TESTING.md, Components), never
// in Node. It starts a browser, so it runs above the Quick suite. Test
// support's setup resets hover and browser state before each test, and its
// commands set that state through the provider.
const component: TestProjectInlineConfiguration = {
  extends: true,
  test: {
    name: "component",
    include: ["**/*.test.tsx"],
    exclude: [...outside, visualTests],
    testTimeout: 300_000,
    setupFiles: ["@goodfetch/test-support/vitest-browser/setup"],
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: "chromium" }],
      commands: browserCommands,
    },
  },
};

// The Release suite reruns the Console's screens under WCAG's text-spacing
// values and 200% text (ADR 0073; TESTING.md, Accessibility), a style sheet
// test support's setup injects into each test's page, where `clippedText`
// checks no text is cut off. Served tests cannot take it: the production CSP
// refuses an injected style sheet.
const componentTextSpacing: TestProjectInlineConfiguration = {
  extends: true,
  test: {
    name: "component-text-spacing",
    include: ["console/**/tests/screens/**/*.test.tsx"],
    exclude: [...outside, visualTests],
    testTimeout: 300_000,
    setupFiles: [
      "@goodfetch/test-support/vitest-browser/setup",
      "@goodfetch/test-support/vitest-browser/text-spacing",
    ],
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: "chromium", name: "component-text-spacing (chromium)" }],
      commands: browserCommands,
    },
  },
};

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

// The order seed (ci/order-seed.sh) is the tree under test's first 8 bytes;
// the constrained test-health run (ADR 0064; ci/test-health/constrained.sh)
// takes a fresh one and detects leaked asynchronous resources. Vitest's
// shuffle adds 1 to its seed per test, which a number past 2^53 cannot hold,
// so every test would draw alike: it takes the seed's low 53 bits.
const orderSeedText = process.env["GOODFETCH_ORDER_SEED"];
const orderSeed =
  orderSeedText === undefined ? undefined : Number(BigInt(orderSeedText) % 2n ** 53n);

const config: ViteUserConfig = defineConfig({
  test: {
    retry: 0,
    restoreMocks: true,
    unstubEnvs: true,
    unstubGlobals: true,
    sequence: {
      shuffle: { tests: true },
      ...(orderSeed === undefined ? {} : { seed: orderSeed }),
    },
    detectAsyncLeaks: process.env["GOODFETCH_DETECT_ASYNC_LEAKS"] === "1",
    passWithNoTests: true,
    // Attachments, a failed browser test's screenshot among them, go beside
    // the JUnit files, never into the source tree.
    attachmentsDir: "target/vitest-attachments",
    // The Quick suite passes `--slowTestThreshold=1000`; Vitest sets it per run, not per project.
    slowTestThreshold: 30_000,
    reporters: ["default", "junit"],
    outputFile: { junit: "target/junit/vitest.xml" },
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          include: ["**/src/**/*.test.ts"],
          exclude: [...outside, visualTests],
          testTimeout: 10_000,
        },
      },
      {
        extends: true,
        test: {
          name: "integration",
          include: ["**/tests/**/*.test.ts"],
          exclude: [...outside, visualTests],
          testTimeout: 300_000,
        },
      },
      component,
      ...(process.env["GOODFETCH_TEST_SUITE"] === "release" ? [componentTextSpacing] : []),
      ...(process.env["GOODFETCH_VISUAL_IMAGE"] === undefined ? [] : [visual]),
    ],
  },
});

export default config;
