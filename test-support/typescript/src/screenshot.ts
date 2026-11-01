// The shared screenshot helper of Playwright's `visual` project (ADR 0073;
// TESTING.md, Visual regression), so a Reference screenshot depends only on
// the code. Every Playwright screenshot test compares through `screenshot`,
// which:
//
// - refuses a page that lacks a fixed setting: the fixed instant
//   (`page.clock.setFixedTime`), `locale: 'en-US'`, `timezoneId: 'UTC'`, the
//   project's viewport, `deviceScaleFactor: 1` and `reducedMotion: 'reduce'`;
// - awaits `document.fonts.ready`;
// - compares with animations disabled and the caret hidden, masking only the
//   Live view's frames (`<goodfetch-live-view>`), pixels Goodfetch does not own;
// - tolerates, during the comparison alone, the known issue of its browser
//   and Playwright version (`known-issues.ts`): on WebKit, the inline style
//   Playwright inserts before each screenshot, which the production CSP
//   refuses;
// - records the Reference screenshot it reads, so the orphan check
//   (`xtask screenshots orphans`) knows which references a run used. A
//   reference compared without this helper is reported as an orphan.
//
// The tolerance is the shared visual config's, never the test's.
import { setTimeout as delay } from "node:timers/promises";
import { dirname } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { TestSupportError } from "./errors.ts";
import { toleratingToolIssues } from "./known-issues.ts";
import { FIXED_SETTINGS, recordReference } from "./visual-config.ts";

/** A Reference screenshot's name: a PNG file, without the platform. */
export type ScreenshotName = `${string}.png`;

/** A setting a Reference screenshot is taken under. */
export type FixedSetting =
  | "instant"
  | "locale"
  | "timezoneId"
  | "viewport"
  | "deviceScaleFactor"
  | "reducedMotion";

/** `screenshot` refused a page that lacks fixed settings; `settings` names each one. */
export class UnsettledScreenshotError extends TestSupportError {
  readonly settings: readonly FixedSetting[];

  constructor(settings: readonly FixedSetting[]) {
    super(
      "screenshot_unsettled",
      `the page lacks fixed settings: ${settings.join(", ")}; fix the instant with page.clock.setFixedTime and keep the project's locale, time zone, viewport, scale and motion`,
    );
    this.name = "UnsettledScreenshotError";
    this.settings = settings;
  }
}

/** The Live view's frames: the only pixels a screenshot masks. */
const LIVE_VIEW = "goodfetch-live-view";

/** Playwright's viewport when a project sets none. */
const DEFAULT_VIEWPORT = { width: 1280, height: 720 } as const;

/**
 * Real time the helper lets pass between two reads of the page's clock: a
 * page whose instant is fixed reads the same time twice.
 */
const CLOCK_PROBE_MILLISECONDS = 5;

/** Compares the page with its Reference screenshot, pixel for pixel. */
export async function screenshot(page: Page, name: ScreenshotName): Promise<void> {
  const unsettled = await unsettledSettings(page);
  if (unsettled.length > 0) {
    throw new UnsettledScreenshotError(unsettled);
  }
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
  const info = test.info();
  const configFile = info.config.configFile;
  if (configFile === undefined) {
    throw new Error("the visual project runs from the repository's playwright.config.ts");
  }
  recordReference(
    dirname(configFile),
    info.snapshotPath(name, { kind: "screenshot" }),
    "playwright",
  );
  await toleratingToolIssues(page, () =>
    expect(page).toHaveScreenshot(name, {
      animations: "disabled",
      caret: "hide",
      mask: [page.locator(LIVE_VIEW)],
    }),
  );
}

/** Each fixed setting the page lacks, in `FixedSetting`'s order. */
async function unsettledSettings(page: Page): Promise<FixedSetting[]> {
  const first = await page.evaluate(() => Date.now());
  await delay(CLOCK_PROBE_MILLISECONDS);
  const read = await page.evaluate(() => ({
    now: Date.now(),
    language: navigator.language,
    locale: Intl.DateTimeFormat().resolvedOptions().locale,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    devicePixelRatio: window.devicePixelRatio,
    reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
  }));
  const projectViewport = test.info().project.use.viewport;
  const expected = projectViewport === undefined ? DEFAULT_VIEWPORT : projectViewport;
  const actual = page.viewportSize();
  const unsettled: FixedSetting[] = [];
  if (read.now !== first) {
    unsettled.push("instant");
  }
  if (read.language !== FIXED_SETTINGS.locale || read.locale !== FIXED_SETTINGS.locale) {
    unsettled.push("locale");
  }
  if (read.timeZone !== FIXED_SETTINGS.timezoneId) {
    unsettled.push("timezoneId");
  }
  if (actual?.width !== expected?.width || actual?.height !== expected?.height) {
    unsettled.push("viewport");
  }
  if (read.devicePixelRatio !== FIXED_SETTINGS.deviceScaleFactor) {
    unsettled.push("deviceScaleFactor");
  }
  if (!read.reducedMotion) {
    unsettled.push("reducedMotion");
  }
  return unsettled;
}
