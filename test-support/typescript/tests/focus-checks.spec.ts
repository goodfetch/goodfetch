// Seam B (spec #326): at each stop of a keyboard walk, focus is visible and
// not entirely covered (ADR 0073). `focusChecks` passes a clean fixture page
// under the production CSP, and fails a stop that looks the same focused and
// unfocused, a stop under a sticky header, and a stop whose area changes
// while focus stays put, on every browser.
import type { TestSupportErrorCode } from "@goodfetch/test-support";
import { expect, focusChecks, test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

const BROKEN_PAGES: readonly { page: string; code: TestSupportErrorCode }[] = [
  { page: "focus-invisible", code: "focus_not_visible" },
  { page: "focus-obscured", code: "focus_obscured" },
  { page: "focus-unstable", code: "focus_not_visible_unstable" },
];

test.describe("focusChecks", () => {
  test("passes a page whose every stop shows its focus uncovered", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("keyboard-clean"));

    const checked = focusChecks(page);

    await expect(checked).resolves.toBeUndefined();
  });

  test("passes a stop taller than the viewport, checked over its visible part", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("focus-tall"));

    const checked = focusChecks(page);

    await expect(checked).resolves.toBeUndefined();
  });

  for (const { page: name, code } of BROKEN_PAGES) {
    test(`fails with ${code} on the ${name} page`, async ({ page, cspPages }) => {
      await page.goto(cspPages.url(name));

      const checked = focusChecks(page);

      await expect(checked).rejects.toMatchObject({ code });
    });
  }
});
