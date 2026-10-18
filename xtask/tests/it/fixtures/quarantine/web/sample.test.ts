import { describe, expect, test } from "vitest";

describe("sample", () => {
  test("steady (a+b)", () => {
    expect(1).toBe(1);
  });

  test("flaky", () => {
    expect(1).toBe(1);
  });
});
