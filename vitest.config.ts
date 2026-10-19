// Vitest for the whole pnpm workspace (TESTING.md: No retries, Before merge,
// Time budgets; ADRs 0061, 0062 and 0064). Unit tests sit beside their module
// and take the Quick suite's caps; integration tests under `tests/` take the
// caps above it. CI runs with `CI` set, so a missing or mismatched snapshot
// fails and none is written.
//
// Screenshot tests (`*.visual.test.ts[x]`) form the `visual` project, which
// test support's shared visual config gives only inside the pinned Playwright
// image (ADR 0073; ci/visual/).
import { browserCommands } from "@goodfetch/test-support/vitest-browser/commands";
import { VITEST_VISUAL_TESTS, visualConfig } from "@goodfetch/test-support/visual-config";
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
const visualTests = VITEST_VISUAL_TESTS;

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

const visual = visualConfig.vitest(process.env, playwright, { exclude: outside });

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
    ...(visual.update === undefined ? {} : { update: visual.update }),
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
      ...visual.projects,
    ],
  },
});

export default config;
