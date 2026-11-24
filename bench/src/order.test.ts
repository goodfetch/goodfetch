import { expect, test } from "vitest";
import { quantile, shuffle } from "./order.ts";

test("draws the same order from the same seed, and every item once", () => {
  const items = ["a", "b", "c", "d", "e", "f"];

  expect(shuffle(items, 7)).toEqual(shuffle(items, 7));
  expect(shuffle(items, 7).toSorted()).toEqual(items);
  expect(shuffle(items, 7)).not.toEqual(shuffle(items, 8));
});

test("takes the nearest-rank quantile, and none of no values", () => {
  const values = [40, 10, 30, 20];

  expect(quantile(values, 0.5)).toBe(20);
  expect(quantile(values, 0.95)).toBe(40);
  expect(quantile([], 0.5)).toBeNull();
});
