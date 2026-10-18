import { expect, test } from "@playwright/test";

test.describe("sample", () => {
  test("steady (a+b)", () => {
    expect(1).toBe(1);
  });

  test("flaky", () => {
    expect(1).toBe(1);
  });
});
