// Seam B (spec #326): `noRunningAnimations(page)` reads
// `document.getAnimations()` after an action under `reducedMotion: 'reduce'`
// and fails with `animations_running` when one still runs, or with
// `reduced_motion_off` when the page does not prefer reduced motion, where
// passing would prove nothing (ADR 0073).
import { expect, noRunningAnimations, test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

test.describe("noRunningAnimations under reduced motion", () => {
  test.use({ reducedMotion: "reduce" });

  test("passes a page that stops its spinner when motion is reduced", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("motion-reduced"));
    await page.getByRole("button", { name: "Load runs" }).click();
    await expect(page.getByRole("status")).toHaveText("Loading runs");

    await noRunningAnimations(page);
  });

  test("fails with animations_running on a page whose spinner turns when motion is reduced", async ({
    page,
    cspPages,
  }) => {
    await page.goto(cspPages.url("motion-running"));
    await page.getByRole("button", { name: "Load runs" }).click();
    await expect(page.getByRole("status")).toHaveText("Loading runs");

    await expect(noRunningAnimations(page)).rejects.toMatchObject({ code: "animations_running" });
  });
});

test.describe("noRunningAnimations without reduced motion", () => {
  test.use({ reducedMotion: "no-preference" });

  test("fails with reduced_motion_off", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("motion-reduced"));
    await page.getByRole("button", { name: "Load runs" }).click();
    await expect(page.getByRole("status")).toHaveText("Loading runs");

    await expect(noRunningAnimations(page)).rejects.toMatchObject({ code: "reduced_motion_off" });
  });
});
