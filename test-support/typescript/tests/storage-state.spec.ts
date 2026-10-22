// Seam B (spec #326): a served test's `context` starts with no cookie, so no
// test starts with one the Console did not set (ADR 0068). That `context` and
// `request` refuse a `storageState`, by its Error code, is
// `fixture-failures.test.ts`'s.
import { test } from "@goodfetch/test-support/playwright";

test.describe("without a storage state", () => {
  test("starts with no cookie", async ({ context }) => {
    test.expect(await context.cookies()).toEqual([]);
  });
});
