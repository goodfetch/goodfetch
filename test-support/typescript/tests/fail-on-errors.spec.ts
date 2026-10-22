// Seam B (spec #326): what a served test's page does wrong fails the test
// (ADR 0068). `watchPageErrors` records each kind of error on a fixture page
// under the production CSP, and passes a clean page; `failOnErrors`, the
// fixture every served test takes, fails a test whose page made one. A known
// issue of a test tool is tolerated only while the action that raises it
// runs, and only the errors it matches.
import { expect, test as base } from "@playwright/test";
import {
  test as served,
  watchPageErrors,
  type KnownIssue,
  type PageError,
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

  test("take answers the errors so far, after which check passes", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("page-error"));
    await expect.poll(() => watch.errors.map((error) => error.kind)).toContain("page");

    const taken = await watch.take();

    expect(taken.map((error) => error.kind)).toContain("page");
    await expect(watch.check()).resolves.toBeUndefined();
  });

  test("check passes when no page made an error", async ({ context, page, cspPages }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("clean"));
    await expect(page.locator("#result")).toHaveText("ready");

    const checked = watch.check();

    await expect(checked).resolves.toBeUndefined();
  });
});

/** A known issue matching the inline style the inserted-style page's script inserts. */
const INSERTED_STYLE: KnownIssue = {
  id: "inserted-style",
  upstream: "a fixture's issue",
  matches: (error) =>
    (error.kind === "csp_violation" && error.text.startsWith("style-src-elem refused inline")) ||
    (error.kind === "csp_report" && error.text.includes('"style-src-elem"')) ||
    error.kind === "console",
};

/** A known issue matching no error. */
const NO_ERROR: KnownIssue = {
  id: "no-error",
  upstream: "a fixture's issue",
  matches: () => false,
};

/** A known issue matching console messages alone. */
const CONSOLE_ONLY: KnownIssue = {
  id: "console-only",
  upstream: "a fixture's issue",
  matches: (error) => error.kind === "console",
};

const kinds = (errors: readonly PageError[]): PageErrorKind[] =>
  errors.map(({ kind }) => kind).toSorted();

test.describe("watchPageErrors' known issues", () => {
  test("tolerates the CSP violation a known issue matches while it runs, with its console message and report", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("inserted-style"));

    await watch.tolerating(INSERTED_STYLE, () => page.getByRole("button").click());

    await expect
      .poll(() => kinds(watch.tolerated))
      .toEqual(["console", "csp_report", "csp_violation"]);
    expect(watch.errors).toEqual([]);
  });

  test("records an error its known issue does not match, while it runs", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("inserted-style"));

    await watch.tolerating(NO_ERROR, () => page.getByRole("button").click());

    await expect
      .poll(() => kinds(watch.errors))
      .toEqual(["console", "csp_report", "csp_violation"]);
    expect(watch.tolerated).toEqual([]);
  });

  test("records an error its known issue matches once it no longer runs", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("inserted-style"));

    await watch.tolerating(INSERTED_STYLE, async () => {});
    await page.getByRole("button").click();

    await expect
      .poll(() => kinds(watch.errors))
      .toEqual(["console", "csp_report", "csp_violation"]);
    expect(watch.tolerated).toEqual([]);
  });

  test("records a console message its known issue matches without the CSP violation it reports", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("inserted-style"));

    await watch.tolerating(CONSOLE_ONLY, () => page.getByRole("button").click());

    await expect
      .poll(() => kinds(watch.errors))
      .toEqual(["console", "csp_report", "csp_violation"]);
    expect(watch.tolerated).toEqual([]);
  });
});

// That `failOnErrors` fails a test whose page made an error, by its Error
// code, is `fixture-failures.test.ts`'s.
servedTest.describe("failOnErrors", () => {
  servedTest(
    "passes a test that took the errors its page made on purpose",
    async ({ page, cspPages, failOnErrors }) => {
      await page.goto(cspPages.url("page-error"));
      await expect.poll(() => failOnErrors.errors.map((error) => error.kind)).toContain("page");

      const taken = await failOnErrors.take();

      expect(taken.map((error) => error.kind)).toContain("page");
    },
  );

  servedTest("passes a test whose page is clean", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("clean"));

    await expect(page.locator("#result")).toHaveText("ready");
  });
});
