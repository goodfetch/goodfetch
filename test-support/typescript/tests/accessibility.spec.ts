// Seam B (spec #326): axe audits every page a served test reaches for WCAG
// 2.2 AA and best practices (ADR 0073). The `axe` fixture fails a served
// test whose page has a violation at its end, `expectAccessible` audits a
// state on the way, "incomplete" results go to the report without failing,
// and an exception suppresses only its rule on its selector on the pages it
// names, and fails when its selector matches nothing on a page it names
// where its rule ran.
import { expect, test as base } from "@playwright/test";
import {
  auditAccessibility,
  expectAccessible,
  test as served,
  type AxeException,
} from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = base.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });
const servedTest = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

function exception(
  rule: string,
  selector: string,
  pages: readonly string[] = ["/*"],
): AxeException {
  return { rule, selector, pages, reason: "a fixture's false positive", issue: 1701 };
}

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

test.describe("auditAccessibility's exceptions", () => {
  test("an exception suppresses its rule on the elements its selector matches", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("two-images"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("image-alt", "#first, #second")],
      testInfo,
    });

    await expect(audited).resolves.toBeUndefined();
  });

  test("an exception leaves its rule's violations on other elements", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("two-images"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("image-alt", "#first")],
      testInfo,
    });

    await expect(audited).rejects.toMatchObject({ code: "axe_violations" });
  });

  test("an exception leaves other rules' violations on its elements", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("two-images"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("color-contrast", "img")],
      testInfo,
    });

    await expect(audited).rejects.toMatchObject({ code: "axe_violations" });
  });

  test("an exception whose selector matches nothing fails where its rule ran", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("accessible"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("document-title", "#absent")],
      testInfo,
    });

    await expect(audited).rejects.toMatchObject({ code: "axe_exception_unmatched" });
  });

  test("an exception whose elements no longer violate its rule fails where its rule ran", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("accessible"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("document-title", "h1")],
      testInfo,
    });

    await expect(audited).rejects.toMatchObject({ code: "axe_exception_unmatched" });
  });

  for (const selector of ["*", "body *", ":not(#first)"]) {
    test(`an exception whose selector \`${selector}\` takes in the whole page is refused`, async ({
      page,
      cspPages,
    }, testInfo) => {
      await page.goto(cspPages.url("two-images"));

      const audited = auditAccessibility(page, {
        exceptions: [exception("image-alt", selector)],
        testInfo,
      });

      await expect(audited).rejects.toMatchObject({ code: "axe_exception_too_broad" });
    });
  }

  test("an exception suppresses its rule on the pages it names", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("two-images"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("image-alt", "#first, #second", ["/two-images.html"])],
      testInfo,
    });

    await expect(audited).resolves.toBeUndefined();
  });

  test("an exception leaves its rule's violations on a page it does not name", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("two-images"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("image-alt", "#first, #second", ["/accessible.html"])],
      testInfo,
    });

    await expect(audited).rejects.toMatchObject({ code: "axe_violations" });
  });

  test("an exception whose selector matches nothing is not checked on a page it does not name", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("accessible"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("document-title", "#absent", ["/two-images.html"])],
      testInfo,
    });

    await expect(audited).resolves.toBeUndefined();
  });

  test("an exception whose rule did not run on a page is not checked there", async ({
    page,
    cspPages,
  }, testInfo) => {
    await page.goto(cspPages.url("accessible"));

    const audited = auditAccessibility(page, {
      exceptions: [exception("video-caption", "#absent")],
      testInfo,
    });

    await expect(audited).resolves.toBeUndefined();
  });
});
