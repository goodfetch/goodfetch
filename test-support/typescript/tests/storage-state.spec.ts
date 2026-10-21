// Seam B (spec #326): the `context` and `request` fixtures, the only places a
// served test's browser contexts come from, refuse a `storageState`, so no
// test starts with a cookie the Console did not set (ADR 0068). Each refused
// test is expected to fail; `refuseStorageState`'s own tests show the
// failure's code.
import { test } from "@goodfetch/test-support/playwright";

const SIGNED_IN = {
  cookies: [
    {
      name: "goodfetch_session",
      value: "forged",
      domain: "127.0.0.1",
      path: "/",
      expires: -1,
      httpOnly: true,
      secure: false,
      sameSite: "Strict" as const,
    },
  ],
  origins: [],
};

test.describe("the context fixture", () => {
  test.describe("with a storage state", () => {
    test.use({ storageState: SIGNED_IN });

    test.fail("fails the test", async ({ page }) => {
      await page.goto("about:blank");
    });
  });

  test.describe("with a storage state in contextOptions", () => {
    test.use({ contextOptions: { storageState: SIGNED_IN } });

    test.fail("fails the test", async ({ page }) => {
      await page.goto("about:blank");
    });
  });
});

test.describe("the request fixture", () => {
  test.describe("with a storage state", () => {
    test.use({ storageState: SIGNED_IN });

    test.fail("fails the test", async ({ request }) => {
      await request.dispose();
    });
  });
});

test.describe("without a storage state", () => {
  test("starts with no cookie", async ({ context }) => {
    test.expect(await context.cookies()).toEqual([]);
  });
});
