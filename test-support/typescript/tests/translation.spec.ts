// Seam B (spec #326): the translation-emulation script wraps every new text
// node in a `<font>` element, as page translators do (ADR 0073), so a page
// that keeps references to its text nodes and later removes one from the
// parent it put it in throws, while a page that keeps none still works. The
// Release suite's translation project turns it on through the served
// fixture's `translationEmulation` option.
//
// The pages that crash run on Playwright's own `test`, with the page error
// watch read here, since the served `test` would fail them at teardown.
import { expect, test as base } from "@playwright/test";
import {
  emulateTranslation,
  test as served,
  watchPageErrors,
} from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = base.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });
const servedTest = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

test.describe("under the translation-emulation script", () => {
  test("a page that removes a text node it kept a reference to throws", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await emulateTranslation(context);
    await page.goto(cspPages.url("translation-references"));

    await page.getByRole("button", { name: "Hide details" }).click();

    await expect(watch.check()).rejects.toMatchObject({ code: "page_errors" });
    expect(watch.errors).toContainEqual(
      expect.objectContaining({ kind: "page", text: expect.stringContaining("NotFoundError") }),
    );
  });

  test("a page that keeps no reference to a text node works, its text wrapped in font", async ({
    context,
    page,
    cspPages,
  }) => {
    const watch = await watchPageErrors(context);
    await emulateTranslation(context);
    await page.goto(cspPages.url("translation-clean"));
    await expect(page.locator("#details > font")).toHaveText("Details shown");

    await page.getByRole("button", { name: "Hide details" }).click();

    await expect(page.locator("#result")).toHaveText("hidden");
    await expect(page.locator("#details")).toBeEmpty();
    await watch.check();
  });

  test("leaves text inside translate=no alone", async ({ context, page, cspPages }) => {
    await emulateTranslation(context);
    await page.goto(cspPages.url("translation-clean"));
    await expect(page.locator("#details > font")).toHaveText("Details shown");

    const resultChildren = await page
      .locator("#result")
      .evaluate((element) => [...element.childNodes].map((node) => node.nodeName));

    expect(resultChildren).toEqual(["#text"]);
  });
});

test("without the script, the page that keeps text node references works", async ({
  context,
  page,
  cspPages,
}) => {
  const watch = await watchPageErrors(context);
  await page.goto(cspPages.url("translation-references"));

  await page.getByRole("button", { name: "Hide details" }).click();

  await expect(page.locator("#result")).toHaveText("hidden");
  await watch.check();
});

servedTest.describe("the translationEmulation option", () => {
  servedTest.describe("on", () => {
    servedTest.use({ translationEmulation: true });

    servedTest("adds the script to the served test's pages", async ({ page, cspPages }) => {
      await page.goto(cspPages.url("translation-clean"));

      await expect(page.locator("#details > font")).toHaveText("Details shown");
    });
  });

  servedTest.describe("off", () => {
    servedTest.use({ translationEmulation: false });

    servedTest("leaves the served test's pages as they are", async ({ page, cspPages }) => {
      await page.goto(cspPages.url("translation-clean"));
      await expect(page.locator("#details")).toHaveText("Details shown");

      await expect(page.locator("font")).toHaveCount(0);
    });
  });
});
