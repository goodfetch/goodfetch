import { expect, test } from "vitest";

test("reads a value", { retry: { handlerFailed: true, count: 3 } }, () => {
  expect(1).toBe(1);
});
