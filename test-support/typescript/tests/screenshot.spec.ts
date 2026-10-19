// Seam B (spec #326): the shared screenshot helper refuses a page that lacks
// any of ADR 0073's fixed settings (the fixed instant, `locale: 'en-US'`,
// `timezoneId: 'UTC'`, the project's viewport, `deviceScaleFactor: 1` and
// `reducedMotion: 'reduce'`), naming each missing one, before it compares
// anything. Each case fixes every setting but one.
import type { Page, PlaywrightTestOptions } from "@playwright/test";
import { expect, test } from "../src/playwright.ts";
import { screenshot, UnsettledScreenshotError, type FixedSetting } from "../src/screenshot.ts";
import { FIXED_SETTINGS } from "../src/visual-config.ts";

/** TESTING.md's start time of every test. */
const INSTANT = new Date("2026-01-05T09:00:00Z");

/** The settings `screenshot` refused on `page`, or `undefined` when it refused none. */
async function refused(page: Page): Promise<readonly FixedSetting[] | undefined> {
  await page.setContent("<p>Unsettled</p>");
  try {
    await screenshot(page, "unsettled.png");
  } catch (error) {
    if (error instanceof UnsettledScreenshotError) {
      return error.settings;
    }
  }
  return undefined;
}

interface ContextCase {
  readonly name: string;
  readonly use: Partial<PlaywrightTestOptions>;
  readonly settings: readonly FixedSetting[];
}

const contextCases: readonly ContextCase[] = [
  { name: "a locale other than en-US", use: { locale: "fr-FR" }, settings: ["locale"] },
  {
    name: "a time zone other than UTC",
    use: { timezoneId: "Asia/Kathmandu" },
    settings: ["timezoneId"],
  },
  {
    name: "a device scale factor of 2",
    use: { deviceScaleFactor: 2 },
    settings: ["deviceScaleFactor"],
  },
  {
    name: "motion that is not reduced",
    use: { reducedMotion: "no-preference" },
    settings: ["reducedMotion"],
  },
];

test.describe("screenshot refuses a page", () => {
  test.use({ ...FIXED_SETTINGS });

  for (const { name, use, settings } of contextCases) {
    test.describe(`with ${name}`, () => {
      test.use(use);

      test("naming that setting", async ({ page }) => {
        await page.clock.setFixedTime(INSTANT);

        expect(await refused(page)).toEqual(settings);
      });
    });
  }

  test("with a viewport the test set itself, naming the viewport", async ({ page }) => {
    await page.clock.setFixedTime(INSTANT);
    await page.setViewportSize({ width: 400, height: 160 });

    expect(await refused(page)).toEqual(["viewport"]);
  });

  test("whose instant is not fixed, naming the instant", async ({ page }) => {
    expect(await refused(page)).toEqual(["instant"]);
  });
});
