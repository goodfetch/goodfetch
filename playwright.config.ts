// Playwright Test for the whole pnpm workspace (TESTING.md: No retries and
// Time budgets; ADRs 0064 and 0068). Its tests run above the Quick suite, so
// they take its caps: slow after 30 s, killed at 5 min.
//
// Screenshot tests (`*.visual.spec.ts`) form the `visual` project, which
// test support's shared visual config gives only inside the pinned Playwright
// image (ADR 0073; ci/visual/). No suite writes a Reference screenshot; only
// the `update-screenshots` workflow does.
//
// A test declares a Feature with a `covers` annotation (ADR 0072). The
// configuration reads the Feature inventory, validated against its schema,
// and passes its IDs as `metadata.features`, so test support's `test` fails a
// test whose annotation names an unknown Feature.
import { sep } from "node:path";
import { readFeatureIds } from "@goodfetch/test-support/features";
import { stubGatewayConsole, type ServedOptions } from "@goodfetch/test-support/playwright";
import { defineConfig, type PlaywrightTestConfig } from "@playwright/test";
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
// Pre-merge suite on Chromium alone (TESTING.md, Browsers). Its translation
// project reruns the served tests with test support's translation-emulation
// script on every page, which wraps each new text node in `<font>` as page
// translators do; they must still pass, with no uncaught error (ADR 0073).
// Its elsewhere project reruns them in a browser whose locale and time zone
// are Turkish and Nepal's, whose offset is a quarter hour (ADR 0073): no
// test asserts formatted text, so each passes there as it does here.
//
// A project of this configuration takes Playwright's own options as well as
// the served fixtures' (`Project<ServedOptions>` alone drops Playwright's).
type ServedProject = NonNullable<PlaywrightTestConfig<ServedOptions>["projects"]>[number];
const releaseProjects: ServedProject[] =
  process.env["GOODFETCH_TEST_SUITE"] === "release"
    ? [
        ...(["firefox", "webkit"] as const).map((browserName) => ({
          name: browserName,
          testIgnore: [...ignored, PLAYWRIGHT_VISUAL_TESTS],
          use: { browserName },
        })),
        {
          name: "translation",
          testIgnore: [...ignored, PLAYWRIGHT_VISUAL_TESTS],
          use: { translationEmulation: true },
        },
        {
          name: "elsewhere",
          testIgnore: [...ignored, PLAYWRIGHT_VISUAL_TESTS],
          use: { locale: "tr-TR", timezoneId: "Asia/Kathmandu" },
        },
      ]
    : [];

const visual = visualConfig.playwright(process.env, { ignore: ignored });

// The Console conformance suite (ADRs 0031, 0068 and 0096): its cases run
// only in the projects that name a target, each writing that target's
// `conformance` document, never in the others, which would write none, each
// on a Proving ground. Collie's Daemon serving Collie's build runs every
// case but Foxhound's screens and the routing cases; Foxhound's Daemon
// serving Foxhound's build runs Foxhound's screens; the Stub gateway,
// serving Foxhound's build and running each run on the engine of its
// Product, runs the whole suite (spec #593).
const CONFORMANCE_CASES = "conformance/console/**/*.spec.ts";
const FOXHOUND_SCREENS = "conformance/console/foxhound-screens.spec.ts";
const ROUTING_CASES = "conformance/console/routing.spec.ts";
const conformanceProjects: ServedProject[] = [
  {
    name: "collie-daemon",
    testMatch: CONFORMANCE_CASES,
    testIgnore: [...ignored, FOXHOUND_SCREENS, ROUTING_CASES],
    metadata: { conformanceTarget: "collie-daemon" },
    use: { product: "collie", provingGroundSeed: 944 },
  },
  {
    name: "foxhound-daemon",
    testMatch: FOXHOUND_SCREENS,
    metadata: { conformanceTarget: "foxhound-daemon" },
    use: { product: "foxhound", provingGroundSeed: 1232 },
  },
  {
    name: "stub-gateway",
    testMatch: CONFORMANCE_CASES,
    metadata: { conformanceTarget: "stub-gateway" },
    use: { product: "foxhound", provingGroundSeed: 1232, consoleServer: stubGatewayConsole },
  },
];

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
  // Test support's result document reporter writes each `console` variant's
  // `journey` document, and one `conformance` document per project naming a
  // Console conformance target (`metadata.conformanceTarget`), into
  // GOODFETCH_TEST_RESULT_DOCUMENTS, which `just test` sets (ADRs 0063 and
  // 0068). The suite's case list is `conformance/console/cases.toml`.
  reporter: [
    ["list"],
    ["junit", { outputFile: "target/junit/playwright.xml" }],
    ["@goodfetch/test-support/result-document-reporter"],
  ],
  updateSnapshots: visual.updateSnapshots,
  metadata: { features: readFeatureIds(import.meta.dirname) },
  use: { trace: "retain-on-failure" },
  projects: [
    { name: "default", testIgnore: [...ignored, PLAYWRIGHT_VISUAL_TESTS, CONFORMANCE_CASES] },
    ...conformanceProjects,
    ...releaseProjects,
    ...visual.projects,
  ],
});

export default config;
