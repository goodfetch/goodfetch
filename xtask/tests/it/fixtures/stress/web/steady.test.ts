import { describe, expect, test } from "vitest";

describe("steady", () => {
  test("unchanged", () => {
    expect(1).toBe(1);
  });

  test("flaky", () => {
    expect(1).toBe(1);
  });
});
