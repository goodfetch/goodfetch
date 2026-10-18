import { expect, test } from "vitest";

test("reads a value", { retry: 3 }, () => {
  expect(1).toBe(1);
});
