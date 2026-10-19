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
//
// A failed test's trace is kept beside the JUnit files, by the repository's
// `playwright.config.ts`.
import {
  test as base,
  type PlaywrightTestArgs,
  type PlaywrightTestOptions,
  type PlaywrightWorkerArgs,
  type PlaywrightWorkerOptions,
  type TestType,
} from "@playwright/test";
import { refuseBypassCSP } from "./bypass-csp.ts";
import { refuseUnknownCovers } from "./features.ts";
import { watchPageErrors, type PageErrorWatch } from "./page-errors.ts";

export { refuseBypassCSP, type CspOptions } from "./bypass-csp.ts";
export {
  watchPageErrors,
  type PageError,
  type PageErrorKind,
  type PageErrorWatch,
} from "./page-errors.ts";

/** What a served test takes from test support's Playwright `test`, beside Playwright's own. */
export interface ServedFixtures {
  /** The errors of the test's pages so far; the test fails at teardown if there is one. */
  failOnErrors: PageErrorWatch;
  /**
   * Fails the test at teardown when one of its `covers` annotations names a
   * Feature the inventory does not list; the configuration passes the
   * inventory's IDs as `metadata.features`.
   */
  coversKnownFeatures: void;
}

/** Playwright's `test` with test support's fixtures. */
export const test: TestType<
  PlaywrightTestArgs & PlaywrightTestOptions & ServedFixtures,
  PlaywrightWorkerArgs & PlaywrightWorkerOptions
> = base.extend<ServedFixtures>({
  context: async ({ context, bypassCSP, contextOptions }, use) => {
    refuseBypassCSP({ bypassCSP, contextOptions });
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
});

export { expect } from "@playwright/test";
