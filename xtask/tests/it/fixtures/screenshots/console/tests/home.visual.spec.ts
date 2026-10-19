import { expect, test } from "@playwright/test";

/* A comment naming maxDiffPixels or allowedMismatchedPixels sets nothing. */
test("home renders without maxDiffPixels", async ({ page }) => {
  await expect(page).toHaveScreenshot("home.png");
});
