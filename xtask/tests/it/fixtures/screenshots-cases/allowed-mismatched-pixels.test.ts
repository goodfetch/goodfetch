import { expect, test } from "vitest";
import { page } from "vitest/browser";

test("button", async () => {
  await expect(page.getByRole("button")).toMatchScreenshot("button", {
    comparatorName: "pixelmatch",
    comparatorOptions: { allowedMismatchedPixels: 4 },
  });
});
