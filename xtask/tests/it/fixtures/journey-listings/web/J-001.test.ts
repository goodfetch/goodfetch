import { expect, test } from "vitest";

test("sdk-typescript", { tags: ["J-001", "sdk-typescript"] }, () => {
  expect(1).toBe(1);
});

test("carries no Journey", () => {
  expect(1).toBe(1);
});
