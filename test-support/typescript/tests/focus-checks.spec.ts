// Seam B (spec #326): at each stop of a keyboard walk, focus is visible and
// not entirely covered (ADR 0073). `focusChecks` passes a clean fixture page
// under the production CSP, and fails a stop that looks the same focused and
// unfocused and a stop under a sticky header.
import type { TestSupportErrorCode } from "@goodfetch/test-support";
import { expect, focusChecks, test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

const BROKEN_PAGES: readonly { page: string; code: TestSupportErrorCode; screenshots: boolean }[] =
  [
    { page: "focus-invisible", code: "focus_not_visible", screenshots: true },
    { page: "focus-obscured", code: "focus_obscured", screenshots: false },
  ];

test.describe("focusChecks", () => {
  test("passes a page whose every stop shows its focus uncovered", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("keyboard-clean"));

    const checked = focusChecks(page);

    await expect(checked).resolves.toBeUndefined();
  });

  for (const { page: name, code, screenshots } of BROKEN_PAGES) {
    test(`fails with ${code} on the ${name} page`, async ({ page, cspPages, browserName }) => {
      test.skip(
        screenshots && browserName === "webkit",
        "focusChecks takes no screenshot on WebKit, where Playwright's breaks the CSP",
      );
      await page.goto(cspPages.url(name));

      const checked = focusChecks(page);

      await expect(checked).rejects.toMatchObject({ code });
    });
  }
});
