import { expect, test } from "@playwright/test";

test("home", async ({ page }) => {
  const loose = { threshold: 0.3 };
  await expect(page).toHaveScreenshot("home.png", loose);
});
