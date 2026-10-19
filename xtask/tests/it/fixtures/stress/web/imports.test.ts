import { describe, expect, test } from "vitest";
import { one } from "./helper.ts";

describe("imports", () => {
  test("helper", () => {
    expect(one).toBe(1);
  });
});
