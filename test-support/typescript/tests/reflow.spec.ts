// Seam B (spec #326): `reflow(page)` lays the page out at 320 × 256 CSS
// pixels (WCAG 1.4.10) and fails with `horizontal_scroll` when the page, or a
// region of it, scrolls sideways outside a data table or code block, which
// may scroll in their own container (ADR 0073).
import { expect, reflow, test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

test.describe("reflow", () => {
  test("passes a page whose only sideways scroll is in its table and code block", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("reflow-clean"));

    await reflow(page);
  });

  test("the clean page's table and code block do scroll sideways at 320 CSS pixels", async ({
    page,
    cspPages,
  }) => {
    await page.setViewportSize({ width: 320, height: 256 });
    await page.goto(cspPages.url("reflow-clean"));

    const scrolls = await page.evaluate(() =>
      ["runs", "command"].map((id) => {
        const element = document.getElementById(id);
        return element !== null && element.scrollWidth > element.clientWidth;
      }),
    );

    expect(scrolls).toEqual([true, true]);
  });

  test("fails with horizontal_scroll on a page wider than 320 CSS pixels", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("reflow-page-scroll"));

    await expect(reflow(page)).rejects.toMatchObject({ code: "horizontal_scroll" });
  });

  test("fails with horizontal_scroll on a region that scrolls sideways outside a table or code block", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("reflow-region-scroll"));

    await expect(reflow(page)).rejects.toMatchObject({ code: "horizontal_scroll" });
  });

  test("gives the page back its viewport", async ({ page, cspPages }) => {
    await page.setViewportSize({ width: 1024, height: 700 });
    await page.goto(cspPages.url("reflow-clean"));

    await reflow(page);

    expect(page.viewportSize()).toEqual({ width: 1024, height: 700 });
  });
});
