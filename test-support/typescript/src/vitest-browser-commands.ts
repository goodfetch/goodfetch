// The browser commands of `@goodfetch/test-support/vitest-browser`, which a
// Vitest Browser Mode project registers as `browser.commands` (ADR 0068).
// They run in Vitest's Node process, on the Playwright provider's page:
// colour scheme and reduced motion through `page.emulateMedia`, the page's
// own values read first so the reset gives them back (`null` would turn the
// emulation off, not restore the project's); locale, time zone and online
// status through a Chrome DevTools Protocol session of the provider's
// context, whose overrides Playwright itself uses for `locale` and
// `timezoneId`: the locale as `Intl` takes it and, through the user agent
// override's `acceptLanguage`, as `navigator.language` gives it. Online status is `Network.overrideNetworkState`, not
// `context.setOffline`: going offline would also cut the page's own
// connection to Vitest, and the run would hang. Playwright has no way to set
// these three once a context exists on Firefox or WebKit.
/// <reference types="@vitest/browser-playwright" />
import type { CDPSession, Page } from "playwright";
import type { BrowserCommand, BrowserCommandContext } from "vitest/node";
import type { BrowserState } from "./browser-state.ts";
import { TestSupportError } from "./errors.ts";

/** What the page reads of the state a DevTools session overrides. */
interface Overridden {
  readonly locale: string;
  readonly language: string;
  readonly timeZone: string;
  readonly online: boolean;
}

/** Each page's DevTools session, kept while it holds an override, with what the page read before it. */
const overrides = new WeakMap<
  Page,
  { readonly session: CDPSession; readonly before: Overridden }
>();

/** How long the reset waits for the page to read its state back. */
const RESET_DEADLINE_MILLISECONDS = 10_000;

/** A page's colour scheme and reduced motion, as `emulateMedia` takes them. */
interface Media {
  readonly colorScheme: "light" | "dark" | "no-preference";
  readonly reducedMotion: "reduce" | "no-preference";
}

/** Each page's media before a test overrode it, kept until the reset gives it back. */
const projectMedia = new WeakMap<Page, Media>();

/** Sets a test's browser state on the provider's page. */
const browserState: BrowserCommand<[BrowserState]> = async (context, state) => {
  const page = playwrightPage(context);
  if (
    (state.colorScheme !== undefined || state.reducedMotion !== undefined) &&
    !projectMedia.has(page)
  ) {
    projectMedia.set(page, await mediaOf(page));
  }
  if (state.colorScheme !== undefined) {
    await page.emulateMedia({ colorScheme: state.colorScheme });
  }
  if (state.reducedMotion !== undefined) {
    await page.emulateMedia({ reducedMotion: state.reducedMotion });
  }
  if (state.locale === undefined && state.timeZone === undefined && state.online === undefined) {
    return;
  }
  const session = await overrideSession(page);
  if (state.locale !== undefined) {
    await session.send("Emulation.setLocaleOverride", { locale: state.locale });
    const userAgent = await page.evaluate(() => navigator.userAgent);
    await session.send("Emulation.setUserAgentOverride", {
      userAgent,
      acceptLanguage: state.locale,
    });
  }
  if (state.timeZone !== undefined) {
    await session.send("Emulation.setTimezoneOverride", { timezoneId: state.timeZone });
  }
  if (state.online !== undefined) {
    await session.send("Network.overrideNetworkState", {
      offline: !state.online,
      latency: 0,
      downloadThroughput: -1,
      uploadThroughput: -1,
    });
  }
};

/** Gives the page back the project's browser state. */
const browserStateReset: BrowserCommand = async (context) => {
  const page = playwrightPage(context);
  const media = projectMedia.get(page);
  if (media !== undefined) {
    projectMedia.delete(page);
    await page.emulateMedia(media);
  }
  const kept = overrides.get(page);
  if (kept !== undefined) {
    // Detaching the session lifts the overrides it set, and only those, but
    // the page reads them lifted only a moment later: wait until it reads
    // what it read before them, or the next test would start in this one's
    // state.
    overrides.delete(page);
    await kept.session.detach();
    await readsBack(page, kept.before);
  }
};

/** What the page reads now of the state a DevTools session overrides. */
async function overridden(page: Page): Promise<Overridden> {
  return page.evaluate(() => ({
    locale: new Intl.NumberFormat().resolvedOptions().locale,
    language: navigator.language,
    timeZone: new Intl.DateTimeFormat().resolvedOptions().timeZone,
    online: navigator.onLine,
  }));
}

/** Waits until the page reads `before` again; `browser_state_not_reset` past the deadline. */
async function readsBack(page: Page, before: Overridden): Promise<void> {
  const deadline = Date.now() + RESET_DEADLINE_MILLISECONDS;
  for (;;) {
    const now = await overridden(page);
    if (JSON.stringify(now) === JSON.stringify(before)) {
      return;
    }
    if (Date.now() > deadline) {
      throw new TestSupportError(
        "browser_state_not_reset",
        `the page still reads ${JSON.stringify(now)}, not the project's ${JSON.stringify(before)}, ${RESET_DEADLINE_MILLISECONDS} ms after the reset`,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

/** The commands to register as the project's `browser.commands`. */
export const browserCommands: {
  readonly browserState: BrowserCommand<[BrowserState]>;
  readonly browserStateReset: BrowserCommand;
} = { browserState, browserStateReset };

/** The colour scheme and reduced motion the page matches now. */
async function mediaOf(page: Page): Promise<Media> {
  return page.evaluate(() => ({
    colorScheme: matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : matchMedia("(prefers-color-scheme: light)").matches
        ? "light"
        : "no-preference",
    reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "reduce"
      : "no-preference",
  }));
}

function playwrightPage(context: BrowserCommandContext): Page {
  if (context.provider.name !== "playwright") {
    throw new TestSupportError(
      "provider_unsupported",
      `test support's browser commands run on Vitest's Playwright provider, not ${context.provider.name}`,
    );
  }
  return context.page;
}

async function overrideSession(page: Page): Promise<CDPSession> {
  const kept = overrides.get(page);
  if (kept !== undefined) {
    return kept.session;
  }
  const browser = page.context().browser()?.browserType().name();
  if (browser !== "chromium") {
    throw new TestSupportError(
      "provider_unsupported",
      `a locale, time zone or online status is set per test only on Chromium; on ${browser ?? "this browser"}, set \`locale\`, \`timezoneId\` or \`offline\` in the project's provider context options`,
    );
  }
  const before = await overridden(page);
  const session = await page.context().newCDPSession(page);
  overrides.set(page, { session, before });
  return session;
}
