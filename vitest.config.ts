// Vitest for the whole pnpm workspace (TESTING.md: No retries, Before merge,
// Time budgets; ADRs 0061, 0062 and 0064). Unit tests sit beside their module
// and take the Quick suite's caps; integration tests under `tests/` take the
// caps above it. CI runs with `CI` set, so a missing or mismatched snapshot
// fails and none is written.
//
// Screenshot tests (`*.visual.test.ts[x]`) form the `visual` project, which
// test support's shared visual config gives only inside the pinned Playwright
// image (ADR 0073; ci/visual/).
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { CaseLogReporter, clientCaseTags, readClientCases } from "@goodfetch/test-support/case-log";
import { readFeatureIds } from "@goodfetch/test-support/features";
import { browserCommands } from "@goodfetch/test-support/vitest-browser/commands";
import { VITEST_VISUAL_TESTS, visualConfig } from "@goodfetch/test-support/visual-config";
import { lingui } from "@lingui/vite-plugin";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import tailwindcss from "@tailwindcss/vite";
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

// Each Storybook story of the Console renders as a test through Storybook's
// Vitest addon, which runs the a11y addon's axe pass on it, once in light
// and once in dark (TESTING.md, Components and Accessibility): one project
// per colour scheme, each pinning the stories' `theme` global. The addon
// sets the project's `include` to the `stories` globs of the Storybook
// configuration, and ignores any other. It gives no plugin when Vitest does
// not load this file (no `VITEST` in the environment): the project is then
// left out, so it never falls back to Vitest's default `include` and
// collects every other project's tests.
const storybookConfig = join(import.meta.dirname, "console", "collie", ".storybook");
const componentStories = async (
  theme: "light" | "dark",
): Promise<TestProjectInlineConfiguration[]> => {
  const plugins = await storybookTest({ configDir: storybookConfig, initialGlobals: { theme } });
  return plugins.length === 0 ? [] : [componentStoriesProject(theme, plugins)];
};
const componentStoriesProject = (
  theme: "light" | "dark",
  plugins: Awaited<ReturnType<typeof storybookTest>>,
): TestProjectInlineConfiguration => ({
  extends: true,
  plugins,
  test: {
    name: `component-stories-${theme}`,
    exclude: [...outside, visualTests],
    testTimeout: 300_000,
    setupFiles: ["@goodfetch/test-support/vitest-browser/setup"],
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: "chromium", name: `component-stories-${theme} (chromium)` }],
      commands: browserCommands,
    },
  },
});

// The Release suite renders every story again under each of the Console's
// two pseudo-locales (ADR 0073; TESTING.md, Localization), the browser
// context's locale, which the Console reads its catalog by: `en-XA`
// (accented, longer, bracketed) and `ar-XB` (right to left). Once each
// story ends, test support's setup fails it on clipped text, under `en-XA`
// on visible text neither from the catalog nor inside `translate="no"`,
// and under `ar-XB` on a document that does not run right to left. Stories
// hold no assertion, so they render the same under any catalog, where the
// screens' tests find their elements by their English names.
const pseudoLocales = ["en-XA", "ar-XB"] as const;
const pseudoLocaleStories = async (
  locale: (typeof pseudoLocales)[number],
): Promise<TestProjectInlineConfiguration[]> => {
  const plugins = await storybookTest({
    configDir: storybookConfig,
    initialGlobals: { theme: "light" },
  });
  return plugins.length === 0
    ? []
    : [
        {
          extends: true,
          plugins,
          test: {
            name: `component-stories-${locale}`,
            exclude: [...outside, visualTests],
            testTimeout: 300_000,
            setupFiles: [
              "@goodfetch/test-support/vitest-browser/setup",
              "@goodfetch/test-support/vitest-browser/pseudo-locale",
            ],
            browser: {
              enabled: true,
              headless: true,
              provider: playwright({ contextOptions: { locale } }),
              instances: [{ browser: "chromium", name: `component-stories-${locale} (chromium)` }],
              commands: browserCommands,
            },
          },
        },
      ];
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

const visual = visualConfig.vitest(process.env, playwright, { exclude: outside });

// The order seed (ci/order-seed.sh) is the tree under test's first 8 bytes;
// the constrained test-health run (ADR 0064; ci/test-health/constrained.sh)
// takes a fresh one and detects leaked asynchronous resources. Vitest's
// shuffle adds 1 to its seed per test, which a number past 2^53 cannot hold,
// so every test would draw alike: it takes the seed's low 53 bits.
const orderSeedText = process.env["GOODFETCH_ORDER_SEED"];
const orderSeed =
  orderSeedText === undefined ? undefined : Number(BigInt(orderSeedText) % 2n ** 53n);

// The tags a Journey test carries (ADR 0063; TESTING.md, Journeys): its
// Journey's ID, one per file of the catalog, and its surface, of those
// Vitest runs. `strictTags` refuses any other tag.
const journeyTags = readdirSync(join(import.meta.dirname, "journeys"))
  .filter((name) => /^J-\d{3,9}\.toml$/.test(name))
  .map((name) => ({ name: name.slice(0, -".toml".length) }));
const surfaceTags = [{ name: "sdk-typescript" }];
// A test declares a Feature with a tag of its ID (ADR 0072), one per Feature
// of the inventory, validated against its schema. Vitest takes the IDs as
// they are: its tag names accept `:` and `.` (checked on Vitest 5.0.5).
const featureTags = readFeatureIds(import.meta.dirname).map((name) => ({ name }));
// A Client conformance test carries its case as `client_case/<id>`, one tag
// per case of the case list, and its mode as `attached` or `daemon` (ADR
// 0069). The case-log reporter writes one entry per case each run ran, for
// `goodfetch-conformance write`; `xtask clients check` reads the same tags
// from Vitest's listing.
const caseList = join(import.meta.dirname, "conformance", "clients", "cases.toml");
const caseTags = clientCaseTags(readClientCases(caseList));

// `just coverage` (ADR 0072; TESTING.md, Code coverage) names the folder its
// reports go to: Vitest's `v8` provider then measures every project, in Node
// and in Browser Mode, with no threshold.
const coverageFolder = process.env["GOODFETCH_COVERAGE"];

// The Console's components render their messages through Lingui's macros
// and its catalog, compiled as the Console's build compiles them (ADR 0073),
// and a test that imports the design system's style sheet, as a screen's
// visual test does, gets it through Tailwind's plugin, as the build does.
// A copy of this config in a folder without the Console, as test support's
// own runs of it make, takes no plugin. Foxhound's configuration holds
// Collie's catalog, extracted alike, and its own, so Foxhound's screens'
// stories load Foxhound's catalog too.
const consoleMessages = join(import.meta.dirname, "console", "foxhound", "lingui.config.ts");

const config: ViteUserConfig = defineConfig({
  plugins: existsSync(consoleMessages)
    ? [lingui({ macroTransform: true, configPath: consoleMessages }), tailwindcss()]
    : [],
  test: {
    ...(visual.update === undefined ? {} : { update: visual.update }),
    ...(coverageFolder === undefined
      ? {}
      : {
          coverage: {
            enabled: true,
            provider: "v8",
            reporter: ["text-summary", "html", "lcov"],
            reportsDirectory: join(coverageFolder, "vitest"),
            exclude: [...outside, "**/generated/**"],
          },
        }),
    // Every project extends this, so none collects a file outside the
    // sources, such as another checkout's tests under `.claude/worktrees/`.
    exclude: outside,
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
    tags: [...journeyTags, ...surfaceTags, ...featureTags, ...caseTags],
    strictTags: true,
    // Attachments, a failed browser test's screenshot among them, go beside
    // the JUnit files, never into the source tree.
    attachmentsDir: "target/vitest-attachments",
    // The Quick suite passes `--slowTestThreshold=1000`; Vitest sets it per run, not per project.
    slowTestThreshold: 30_000,
    reporters: [
      "default",
      "junit",
      new CaseLogReporter({ out: "target/case-logs/vitest.jsonl", caseList }),
    ],
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
      ...(existsSync(storybookConfig)
        ? [...(await componentStories("light")), ...(await componentStories("dark"))]
        : []),
      ...(process.env["GOODFETCH_TEST_SUITE"] === "release" ? [componentTextSpacing] : []),
      ...(process.env["GOODFETCH_TEST_SUITE"] === "release" && existsSync(storybookConfig)
        ? (await Promise.all(pseudoLocales.map(pseudoLocaleStories))).flat()
        : []),
      ...visual.projects,
    ],
  },
});

export default config;
