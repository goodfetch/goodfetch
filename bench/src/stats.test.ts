import { expect, test } from "vitest";
import {
  blockBootstrap,
  hodgesLehmann,
  normalQuantile,
  studentTQuantile,
  tost,
  wilcoxonSignedRank,
} from "./stats.ts";

test("takes exact signed-rank p-values from the enumerated null distribution", () => {
  // n = 5, all differences positive: 1 of 32 sign patterns is as extreme, so 2/32 two-sided.
  expect(wilcoxonSignedRank([1, 2, 3, 4, 5]).pValue).toBeCloseTo(2 / 32, 12);
  // n = 10 with W- = 3 (ranks 1 and 2 negative): the subsets of 1..10 summing to at most 3
  // are {}, {1}, {2}, {3} and {1,2}, 5 of 1024.
  const result = wilcoxonSignedRank([-0.1, -0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1]);

  expect(result.method).toBe("exact");
  expect(result.pValue).toBeCloseTo((2 * 5) / 1024, 12);
});

test("takes the one-sample Hodges–Lehmann estimate as the median of the Walsh averages", () => {
  // The Walsh averages of [1, 2, 3, 4, 10] sorted: 1, 1.5, 2, 2, 2.5, 2.5, 3, 3, 3.5, 4, 5.5,
  // 6, 6.5, 7, 10; the 8th of 15 is 3.
  expect(hodgesLehmann([1, 2, 3, 4, 10])).toBe(3);
  expect(hodgesLehmann([4])).toBe(4);
});

test("matches the published normal and Student t tables", () => {
  expect(normalQuantile(0.975)).toBeCloseTo(1.959_964, 6);
  expect(normalQuantile(0.8)).toBeCloseTo(0.841_621, 6);
  expect(studentTQuantile(0.95, 9)).toBeCloseTo(1.833_113, 5);
  expect(studentTQuantile(0.975, 4)).toBeCloseTo(2.776_445, 5);
  expect(studentTQuantile(0.95, 1)).toBeCloseTo(6.313_752, 4);
});

test("declares equivalence by TOST only when the 1 - 2α t interval lies inside the margin", () => {
  // Differences [0, 0.04]: mean 0.02, sd 0.028 284 3, t(0.95, 1) = 6.313 752, half width 0.126 275.
  const result = tost([0, 0.04], 0.2, 0.05);

  expect(result.ci[0]).toBeCloseTo(-0.106_275, 5);
  expect(result.ci[1]).toBeCloseTo(0.146_275, 5);
  expect(result.equivalent).toBe(true);
  expect(tost([0, 0.04], 0.1, 0.05).equivalent).toBe(false);
});

test("draws the same block-bootstrap interval from the same seed, around the estimate, wider with spread", () => {
  const tight = [0.1, 0.11, 0.09, 0.1, 0.12, 0.08];
  const wide = [0.3, -0.1, 0.25, -0.05, 0.4, -0.2];
  const first = blockBootstrap(tight, { resamples: 4000, seed: 7, alpha: 0.05 });
  const spread = blockBootstrap(wide, { resamples: 4000, seed: 7, alpha: 0.05 });

  expect(blockBootstrap(tight, { resamples: 4000, seed: 7, alpha: 0.05 })).toEqual(first);
  expect(first.ci[0]).toBeLessThanOrEqual(first.estimate);
  expect(first.estimate).toBeLessThanOrEqual(first.ci[1]);
  expect(spread.ci[1] - spread.ci[0]).toBeGreaterThan(first.ci[1] - first.ci[0]);
});
