// Seam B (spec #326): what axe cannot see about keyboard use is tested the
// same way on every screen (ADR 0073). `keyboardWalk` passes a clean fixture
// page under the production CSP, giving its stops' names for a snapshot, and
// fails each page that breaks one of its rules.
import type { TestSupportErrorCode } from "@goodfetch/test-support";
import { expect, keyboardWalk, test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

const BROKEN_PAGES: readonly { page: string; code: TestSupportErrorCode }[] = [
  { page: "keyboard-trap", code: "focus_trapped" },
  { page: "keyboard-focus-lost", code: "focus_lost" },
  { page: "keyboard-overlay-lost", code: "focus_not_returned" },
  { page: "keyboard-overlay-unopened", code: "overlay_not_opened" },
  { page: "keyboard-unreachable", code: "focus_unreachable" },
];

test.describe("keyboardWalk", () => {
  test("gives the role and accessible name of each stop of a clean page in Tab order", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("keyboard-clean"));

    const stops = await keyboardWalk(page);

    expect(stops).toEqual([
      { role: "link", name: "Docs" },
      { role: "button", name: "Actions" },
      { role: "button", name: "Settings" },
      { role: "textbox", name: "Email" },
      { role: "button", name: "Save" },
    ]);
  });

  for (const { page: name, code } of BROKEN_PAGES) {
    test(`fails with ${code} on the ${name} page`, async ({ page, cspPages }) => {
      await page.goto(cspPages.url(name));

      const walked = keyboardWalk(page);

      await expect(walked).rejects.toMatchObject({ code });
    });
  }
});
