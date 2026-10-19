import { expect, test } from "@playwright/test";
import { one } from "./helper.ts";

test.describe("imports", () => {
  test("helper", () => {
    expect(one).toBe(1);
  });
});
