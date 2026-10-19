import { expect, test } from "@playwright/test";

test.describe("the sample page", () => {
  test.describe.configure({ mode: "serial", retries: 1 });

  test("shows its title", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveTitle("Sample");
  });
});
