// Seam B (spec #326): axe audits every page a served test reaches for WCAG
// 2.2 AA and best practices (ADR 0073). The `axe` fixture fails a served
// test whose page has a violation at its end, `expectAccessible` audits a
// state on the way, and "incomplete" results go to the report without
// failing.
import { expect, test as base } from "@playwright/test";
import { expectAccessible, test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = base.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });
const servedTest = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

// That the `axe` fixture fails a test whose page has a violation, by its
// Error code, is `fixture-failures.test.ts`'s.
servedTest.describe("the axe fixture", () => {
  servedTest("passes a test whose page is accessible", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("accessible"));

    await expect(page.getByRole("heading", { name: "Accessible" })).toBeVisible();
  });

  servedTest(
    "attaches a page's incomplete results to the report and passes the test",
    async ({ page, cspPages }, testInfo) => {
      await page.goto(cspPages.url("incomplete"));

      await expectAccessible(page);

      const attachment = testInfo.attachments.find(({ name }) => name === "axe-incomplete");
      const incomplete: unknown = JSON.parse(attachment?.body?.toString("utf8") ?? "null");
      expect(incomplete).toMatchObject({
        url: cspPages.url("incomplete"),
        incomplete: [{ id: "color-contrast" }],
      });
    },
  );
});

test.describe("expectAccessible", () => {
  test("fails with axe_violations on a page with a violation", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("inaccessible"));

    const audited = expectAccessible(page);

    await expect(audited).rejects.toMatchObject({ code: "axe_violations" });
  });

  test("passes an accessible page", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("accessible"));

    const audited = expectAccessible(page);

    await expect(audited).resolves.toBeUndefined();
  });
});
