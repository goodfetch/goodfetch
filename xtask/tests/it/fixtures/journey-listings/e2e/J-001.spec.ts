import { expect, test } from "@playwright/test";

test("console", { tag: ["@J-001", "@console"] }, () => {
  expect(1).toBe(1);
});
