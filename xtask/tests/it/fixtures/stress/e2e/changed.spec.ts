import { expect, test } from "@playwright/test";

test.describe("changed", () => {
  test("kept", () => {
    expect(1).toBe(1);
  });

  test("renamed before", () => {
    expect(1).toBe(1);
  });
});
