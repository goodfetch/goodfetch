// Seam B (spec #326): the `context` and `request` fixtures, the only places a
// served test's browser contexts come from, refuse `bypassCSP` (ADR 0068).
// Each refused test is expected to fail; `refuseBypassCSP`'s own tests show
// the failure's code.
import { test } from "@goodfetch/test-support/playwright";

test.describe("the context fixture", () => {
  test.describe("with bypassCSP", () => {
    test.use({ bypassCSP: true });

    test.fail("fails the test", async ({ page }) => {
      await page.goto("about:blank");
    });
  });

  test.describe("with bypassCSP in contextOptions", () => {
    test.use({ contextOptions: { bypassCSP: true } });

    test.fail("fails the test", async ({ page }) => {
      await page.goto("about:blank");
    });
  });
});

test.describe("the request fixture", () => {
  test.describe("with bypassCSP", () => {
    test.use({ bypassCSP: true });

    test.fail("fails the test", async ({ request }) => {
      await request.dispose();
    });
  });
});
