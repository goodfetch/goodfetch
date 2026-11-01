// A served visual case (ADR 0073): a fixture page served under the
// production CSP, as every Console visual case is, so the repeatability
// measurement covers what a served screenshot meets on each browser, such as
// WebKit's screenshot style the CSP refuses (`known-issues.ts`).
import { test as served } from "@goodfetch/test-support/playwright";
import { screenshot } from "@goodfetch/test-support/screenshot";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

/** TESTING.md's start time of every test. */
const INSTANT = new Date("2026-01-05T09:00:00Z");

test("a page served under the production CSP renders the same pixels in the pinned image", async ({
  page,
  cspPages,
}) => {
  await page.clock.setFixedTime(INSTANT);
  await page.goto(cspPages.url("accessible"));

  await screenshot(page, "served-page.png");
});
