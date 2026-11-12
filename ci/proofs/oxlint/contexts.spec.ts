// Uses each property oxlint must refuse in a test file (ADR 0068): every
// browser and context comes from @goodfetch/test-support/playwright, no test
// sleeps, none redefines a property, and none writes a cookie or storage
// state its page did not set. ci/proofs/run.sh expects one
// `eslint(no-restricted-properties)` error per refused use, and none for
// `context.newPage()`, which stays allowed.
import { chromium, expect, request, test } from "@playwright/test";

test("uses every refused property", async ({ browser, context, page }) => {
  await browser.newContext({ bypassCSP: true });
  await request.newContext();
  await browser.newPage();
  await chromium.launchPersistentContext("", {});
  await chromium.launch();
  await chromium.connect("ws://127.0.0.1:1");
  await chromium.connectOverCDP("http://127.0.0.1:1");
  await page.waitForTimeout(1_000);
  Object.defineProperty(navigator, "onLine", { value: false });
  Object.defineProperties(navigator, { onLine: { value: false } });
  Reflect.defineProperty(document, "hidden", { value: true });
  await context.addCookies([{ name: "session", value: "forged", url: "http://127.0.0.1" }]);
  await context.setStorageState({ cookies: [], origins: [] });
  await page.evaluate(() => {
    document.cookie = "session=forged";
  });
  await context.newPage();
  expect(page).toBeDefined();
});
