// The pinned image's own visual test (ADR 0073): a fixed page, with no font
// or image of its own, renders to the same pixels on every run of the
// `visual` project. Its Reference screenshot comes only from the
// `update-screenshots` workflow.
import { test } from "./playwright.ts";
import { screenshot } from "./screenshot.ts";

/** TESTING.md's start time of every test. */
const INSTANT = new Date("2026-01-05T09:00:00Z");

const PAGE = `<!doctype html>
<html lang="en">
  <body style="margin: 0; background: #ffffff">
    <main style="padding: 16px; font: 16px sans-serif; color: #1a1a1a">
      <h1 style="margin: 0 0 8px; font-size: 20px">Reference screenshot</h1>
      <p style="margin: 0 0 8px">Rendered in the pinned Playwright image.</p>
      <div style="width: 120px; height: 24px; background: #0b6e4f; border-radius: 4px"></div>
    </main>
  </body>
</html>`;

test.describe("screenshot", () => {
  test("fixed page renders the same pixels in the pinned image", async ({ page }) => {
    await page.clock.setFixedTime(INSTANT);
    await page.setContent(PAGE);

    await screenshot(page, "fixture-page.png");
  });
});
