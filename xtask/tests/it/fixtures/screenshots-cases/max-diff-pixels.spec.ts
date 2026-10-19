import { expect, test } from "@playwright/test";

test("home", async ({ page }) => {
  await expect(page).toHaveScreenshot("home.png", { maxDiffPixels: 10 });
});
