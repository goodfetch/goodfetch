// Seam B (spec #325): `coversKnownFeatures`, on in every test of test
// support's Playwright `test`, fails a test whose `covers` annotation names
// a Feature the inventory does not list (ADR 0072). The repository's
// configuration passes the inventory's IDs. These tests add their
// annotations as they run, so the listing the Feature scanner reads holds
// none: they declare no Feature. `refuseUnknownCovers`'s own tests show the
// failure's code.
import { test } from "@goodfetch/test-support/playwright";

test.fail("fails a test whose covers annotation names no Feature of the inventory", () => {
  test.info().annotations.push({ type: "covers", description: "story:0-0" });
});

test("passes a test whose annotations name no Feature", () => {
  test.info().annotations.push({ type: "issue", description: "https://example.invalid/1" });
});
