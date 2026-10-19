import { describe, expect, test } from "vitest";

describe("changed", () => {
  test("kept", () => {
    expect(1).toBe(1);
  });

  test("renamed before", () => {
    expect(1).toBe(1);
  });
});
