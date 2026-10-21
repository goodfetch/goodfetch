// The Playwright fixtures of `@goodfetch/test-support` (ADR 0068), through
// `test.extend`: a served test imports `test` from
// `@goodfetch/test-support/playwright` and gets its browser context, pages
// and API requests from here, the one place that creates them.
//
// - `context` and `request`: Playwright's own, refused when the test turns
//   on `bypassCSP`, so every test runs under the production CSP, or gives a
//   `storageState`, so no test starts with a cookie the Console did not
//   set. Test files cannot make their own: oxlint refuses `newContext`,
//   `browser.newPage`, `launchPersistentContext`, `launch`, `connect` and
//   `connectOverCDP`, and `addCookies`, `setStorageState` and
//   `document.cookie`.
// - `failOnErrors`, on in every test: fails it when a page logged a
//   `console` error, threw, broke the CSP or had a same-origin request fail.
// - `coversKnownFeatures`, on in every test: fails it when a `covers`
//   annotation names a Feature the inventory does not list (ADR 0072).
// - `translationEmulation`, an option off by default: adds the
//   translation-emulation script to every page, as the Release suite's
//   translation project does (ADR 0073).
// - `daemon`: the test's own Daemon, started by `goodfetch-test-env
//   --daemon` in its own home and run by the executable of `product`
//   (Collie's unless the screen is Foxhound's, ADR 0094), or by `engine`
//   for a skew run, with the Console
//   `consoleServer` serves beside it on its own port, and the Proving ground
//   of `provingGroundSeed` when set. All of it stops at teardown, leaving no
//   runtime file and no process.
// - `signIn`: the ways a person signs in to the test's Console (the
//   executable's single-user link, single sign-on through the worker's
//   local OIDC provider), with Playwright's virtual authenticator installed
//   on the context for passkeys and step-up. No fixture writes a cookie.
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
import { startOidcProvider, type OidcClient } from "./oidc-provider.ts";
import { ProcessRecord } from "./process-record.ts";
import type { Product } from "./product-env.ts";
import { startServedDaemon, type ConsoleServer, type ServedDaemon } from "./served-daemon.ts";
import { manifestWithEngine } from "./test-env.ts";
import { signInTo, type SignIn } from "./sign-in.ts";
import { refuseStorageState } from "./storage-state.ts";
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
export { refuseStorageState, type StorageStateOptions } from "./storage-state.ts";
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
export {
  OIDC_PERSON,
  startOidcProvider,
  type OidcClient,
  type OidcPerson,
  type StartedOidcProvider,
} from "./oidc-provider.ts";
export {
  startServedDaemon,
  type ConsoleServer,
  type ConsoleStart,
  type ServedConsole,
  type ServedDaemon,
  type ServedDaemonStart,
  type StartedServedDaemon,
} from "./served-daemon.ts";
export { signInTo, type SignIn } from "./sign-in.ts";

/** The options of test support's Playwright `test`, which a project or `test.use` sets. */
export interface ServedOptions {
  /**
   * Adds the translation-emulation script to every page of the test's
   * context (`emulateTranslation`), as the Release suite's translation
   * project does. Off by default.
   */
  translationEmulation: boolean;
  /** The Product whose executable runs the test's Daemon and serves its Console build: Collie's unless the screen is Foxhound's (ADR 0094). */
  product: Product;
  /** The seed of the Proving ground `daemon` starts beside the Daemon; none when unset. */
  provingGroundSeed: number | undefined;
  /** What serves the Console beside the test's Daemon, until the executables serve it (area 12). */
  consoleServer: ConsoleServer | undefined;
  /** The test tool manifest's path: `GOODFETCH_TEST_TOOLS`, which `just test` sets. */
  toolManifest: string | undefined;
  /** The executable that runs the test's Daemon in place of the manifest's of `product`, for a skew run (ADR 0066); the manifest's when unset. */
  engine: string | undefined;
}

/** What a served test's worker shares between its tests. */
export interface ServedWorkerFixtures {
  /** The worker's OIDC provider, which each test's Console signs people in through. */
  oidcProvider: OidcClient;
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
  /** The test's own Daemon and the Console it serves. */
  daemon: ServedDaemon;
  /** Signs a person in to the test's Console, with the virtual authenticator installed. */
  signIn: SignIn;
}

/** Playwright's `test` with test support's fixtures. */
export const test: TestType<
  PlaywrightTestArgs & PlaywrightTestOptions & ServedFixtures,
  PlaywrightWorkerArgs & PlaywrightWorkerOptions & ServedWorkerFixtures
> = base.extend<ServedFixtures, ServedWorkerFixtures>({
  translationEmulation: [false, { option: true }],
  product: ["collie", { option: true }],
  provingGroundSeed: [undefined, { option: true }],
  consoleServer: [undefined, { option: true }],
  toolManifest: [process.env["GOODFETCH_TEST_TOOLS"], { option: true }],
  engine: [undefined, { option: true }],
  oidcProvider: [
    // oxlint-disable-next-line no-empty-pattern -- Playwright reads a fixture's dependencies from its first parameter's pattern; this one has none.
    async ({}, use) => {
      const provider = await startOidcProvider();
      await use(provider.client);
      await provider.close();
    },
    { scope: "worker" },
  ],
  daemon: async (
    { product, provingGroundSeed, consoleServer, toolManifest, engine, oidcProvider },
    use,
    testInfo,
  ) => {
    const processes = new ProcessRecord();
    try {
      const started = await startServedDaemon({
        processes,
        testName: testInfo.titlePath.join(" "),
        toolManifest:
          engine === undefined
            ? toolManifest
            : manifestWithEngine(
                toolManifest,
                product,
                engine,
                testInfo.outputPath("test-tools.json"),
              ),
        product,
        provingGroundSeed,
        consoleServer,
        oidc: oidcProvider,
      });
      await use(started.daemon);
      await started.close();
    } finally {
      await processes.close();
    }
  },
  signIn: async ({ context, daemon, oidcProvider }, use) => {
    await use(await signInTo(context, daemon, oidcProvider));
  },
  context: async (
    { context, bypassCSP, contextOptions, storageState, translationEmulation },
    use,
  ) => {
    refuseBypassCSP({ bypassCSP, contextOptions });
    refuseStorageState({ storageState, contextOptions });
    if (translationEmulation) {
      await emulateTranslation(context);
    }
    await use(context);
  },
  request: async ({ request, bypassCSP, contextOptions, storageState }, use) => {
    refuseBypassCSP({ bypassCSP, contextOptions });
    refuseStorageState({ storageState, contextOptions });
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
