// The Playwright fixtures of `@goodfetch/test-support` (ADR 0068), through
// `test.extend`: a served test imports `test` from
// `@goodfetch/test-support/playwright` and gets its browser context, pages
// and API requests from here, the one place that creates them.
//
// - `context` and `request`: Playwright's own, refused when the test turns
//   on `bypassCSP`, so every test runs under the production CSP. Test files
//   cannot make their own: oxlint refuses `newContext`, `browser.newPage`,
//   `launchPersistentContext`, `launch`, `connect` and `connectOverCDP`.
// - `failOnErrors`, on in every test: fails it when a page logged a
//   `console` error, threw, broke the CSP or had a same-origin request fail.
// - `coversKnownFeatures`, on in every test: fails it when a `covers`
//   annotation names a Feature the inventory does not list (ADR 0072).
// - `translationEmulation`, an option off by default: adds the
//   translation-emulation script to every page, as the Release suite's
//   translation project does (ADR 0073).
// - `axe`, on in every test: once the test has passed, audits each of its
//   open pages for WCAG 2.2 AA and best practices (ADR 0073), with the
//   repository's exception file, before `failOnErrors` judges the pages, so
//   axe breaking the CSP fails the test too. `expectAccessible(page)` audits
//   a state on the way.
//
// A failed test's trace is kept beside the JUnit files, by the repository's
// `playwright.config.ts`.
import {
  test as base,
  type Page,
  type PlaywrightTestArgs,
  type PlaywrightTestOptions,
  type PlaywrightWorkerArgs,
  type PlaywrightWorkerOptions,
  type TestType,
} from "@playwright/test";
import { auditAccessibility } from "./accessibility.ts";
import { readAxeExceptions } from "./axe-exceptions.ts";
import { refuseBypassCSP } from "./bypass-csp.ts";
import { refuseUnknownCovers } from "./features.ts";
import { watchPageErrors, type PageErrorWatch } from "./page-errors.ts";
import { emulateTranslation } from "./translation.ts";

export {
  AXE_TAGS,
  auditAccessibility,
  expectAccessible,
  type AuditOptions,
} from "./accessibility.ts";
export {
  AXE_EXCEPTIONS_FILE,
  parseAxeExceptions,
  readAxeExceptions,
  type AxeException,
} from "./axe-exceptions.ts";
export { refuseBypassCSP, type CspOptions } from "./bypass-csp.ts";
export { noRunningAnimations } from "./reduced-motion.ts";
export { reflow } from "./reflow.ts";
export { emulateTranslation } from "./translation.ts";
export {
  watchPageErrors,
  type PageError,
  type PageErrorKind,
  type PageErrorWatch,
} from "./page-errors.ts";
export { focusChecks, keyboardWalk, type FocusStop } from "./focus.ts";

/** The options of test support's Playwright `test`, which a project or `test.use` sets. */
export interface ServedOptions {
  /**
   * Adds the translation-emulation script to every page of the test's
   * context (`emulateTranslation`), as the Release suite's translation
   * project does. Off by default.
   */
  translationEmulation: boolean;
}

/** What a served test takes from test support's Playwright `test`, beside Playwright's own. */
export interface ServedFixtures extends ServedOptions {
  /** The errors of the test's pages so far; the test fails at teardown if there is one. */
  failOnErrors: PageErrorWatch;
  /**
   * Fails the test at teardown when one of its `covers` annotations names a
   * Feature the inventory does not list; the configuration passes the
   * inventory's IDs as `metadata.features`.
   */
  coversKnownFeatures: void;
  /** axe's audit of each open page, at the end of a test that passed; the test fails on a violation. */
  axe: void;
}

/** Playwright's `test` with test support's fixtures. */
export const test: TestType<
  PlaywrightTestArgs & PlaywrightTestOptions & ServedFixtures,
  PlaywrightWorkerArgs & PlaywrightWorkerOptions
> = base.extend<ServedFixtures>({
  translationEmulation: [false, { option: true }],
  context: async ({ context, bypassCSP, contextOptions, translationEmulation }, use) => {
    refuseBypassCSP({ bypassCSP, contextOptions });
    if (translationEmulation) {
      await emulateTranslation(context);
    }
    await use(context);
  },
  request: async ({ request, bypassCSP, contextOptions }, use) => {
    refuseBypassCSP({ bypassCSP, contextOptions });
    await use(request);
  },
  failOnErrors: [
    async ({ context }, use) => {
      const watch = await watchPageErrors(context);
      await use(watch);
      await watch.check();
    },
    { auto: true },
  ],
  coversKnownFeatures: [
    // oxlint-disable-next-line no-empty-pattern -- Playwright reads a fixture's dependencies from its first parameter's pattern; this one has none.
    async ({}, use, testInfo) => {
      await use();
      // Checked after the body, so an annotation the test added is checked too.
      const features: unknown = testInfo.config.metadata["features"];
      if (Array.isArray(features)) {
        refuseUnknownCovers(
          testInfo.annotations,
          features.filter((feature): feature is string => typeof feature === "string"),
        );
      }
    },
    { auto: true },
  ],
  axe: [
    // Taking `failOnErrors` sets it up first, so it judges the pages after the audit.
    async ({ context, failOnErrors }, use, testInfo) => {
      await use();
      if (testInfo.status !== "passed") {
        return;
      }
      const exceptions = await readAxeExceptions();
      for (const page of context.pages()) {
        if (page.url() !== "about:blank") {
          await resumeInstalledClock(page);
          await auditAccessibility(page, { exceptions, testInfo });
        }
      }
      void failOnErrors;
    },
    { auto: true },
  ],
});

/**
 * Lets time flow again on a page whose test installed `page.clock`, which
 * may have paused it: axe waits on the page's timers. A page without
 * Playwright's clock is left as it is.
 */
async function resumeInstalledClock(page: Page): Promise<void> {
  if (await page.evaluate(() => "__pwClock" in globalThis)) {
    await page.clock.resume();
  }
}

export { expect } from "@playwright/test";
