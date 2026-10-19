// Seam B (spec #326): what a served test's page does wrong fails the test
// (ADR 0068). `watchPageErrors` records each kind of error on a fixture page
// under the production CSP, and passes a clean page; `failOnErrors`, the
// fixture every served test takes, fails a test whose page made one.
import { expect, test as base } from "@playwright/test";
import {
  test as served,
  watchPageErrors,
  type PageErrorKind,
} from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = base.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });
const servedTest = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

const BROKEN_PAGES: readonly { page: string; kind: PageErrorKind }[] = [
  { page: "inline-script", kind: "csp_violation" },
  { page: "console-error", kind: "console" },
  { page: "page-error", kind: "page" },
  { page: "missing-script", kind: "request_failed" },
  { page: "broken-request", kind: "request_failed" },
];

test.describe("watchPageErrors", () => {
  for (const { page: name, kind } of BROKEN_PAGES) {
    test(`records the ${kind} error of the ${name} page`, async ({ context, page, cspPages }) => {
      const watch = await watchPageErrors(context);

      await page.goto(cspPages.url(name));

      await expect.poll(() => watch.errors.map((error) => error.kind)).toContain(kind);
    });
  }

  test("records the report an inline script sends to the CSP report endpoint", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);

    await page.goto(cspPages.url("inline-script"));

    await expect.poll(() => watch.errors.map((error) => error.kind)).toContain("csp_report");
  });

  test("records nothing on a clean page", async ({ context, page, cspPages }) => {
    const watch = await watchPageErrors(context);

    await page.goto(cspPages.url("clean"));
    await expect(page.locator("#result")).toHaveText("ready");

    expect(watch.errors).toEqual([]);
  });

  test("check fails with page_errors once a page made an error", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("page-error"));

    const checked = watch.check();

    await expect(checked).rejects.toMatchObject({ code: "page_errors" });
  });

  test("check passes when no page made an error", async ({ context, page, cspPages }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("clean"));
    await expect(page.locator("#result")).toHaveText("ready");

    const checked = watch.check();

    await expect(checked).resolves.toBeUndefined();
  });
});

servedTest.describe("failOnErrors", () => {
  for (const { page: name, kind } of BROKEN_PAGES) {
    servedTest.fail(
      `fails a test whose page made a ${kind} error (${name})`,
      async ({ page, cspPages }) => {
        await page.goto(cspPages.url(name));
        await expect(page.locator("main")).toBeVisible();
      },
    );
  }

  servedTest("passes a test whose page is clean", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("clean"));

    await expect(page.locator("#result")).toHaveText("ready");
  });
});
