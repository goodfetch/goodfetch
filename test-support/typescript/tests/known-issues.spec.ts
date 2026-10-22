// Seam B (spec #326): the known issues of the test tools (ADR 0068),
// tolerated around the one action that raises them. Playwright's WebKit
// screenshot inserts an inline `<style>` the production CSP refuses; the
// shared screenshot helper tolerates exactly that refusal, and a page's own
// inline style during the same screenshot still fails. WebKit runs in the
// Release suite.
import { expect, test as base } from "@playwright/test";
import { watchPageErrors, type PageError } from "@goodfetch/test-support/playwright";
import { toleratingToolIssues } from "../src/known-issues.ts";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = base.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

const kinds = (errors: readonly PageError[]): string[] => errors.map(({ kind }) => kind).toSorted();

test.describe("a WebKit screenshot under the production CSP", () => {
  test.skip(
    ({ browserName }) => browserName !== "webkit",
    "Playwright inserts its style before a screenshot on WebKit only",
  );

  test("records Playwright's own inline style as its known issue, not as an error", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("accessible"));

    await toleratingToolIssues(page, () => page.screenshot({ animations: "disabled" }));

    await expect
      .poll(() => kinds(watch.tolerated))
      .toEqual(["console", "csp_report", "csp_violation"]);
    expect(watch.errors).toEqual([]);
  });

  test("records the page's own inline style during the screenshot as an error", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await page.goto(cspPages.url("inserted-style"));

    await toleratingToolIssues(page, async () => {
      await page.getByRole("button").click();
      await page.screenshot({ animations: "disabled" });
    });

    await expect
      .poll(() => kinds(watch.errors))
      .toEqual(["console", "csp_report", "csp_violation"]);
    expect(kinds(watch.tolerated)).toEqual(["console", "csp_report", "csp_violation"]);
  });
});
