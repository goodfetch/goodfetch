import { expect, test } from "@playwright/test";

test.describe("steady", () => {
  test("unchanged", () => {
    expect(1).toBe(1);
  });

  test("flaky", () => {
    expect(1).toBe(1);
  });
});
