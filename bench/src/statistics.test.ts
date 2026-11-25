import { expect, test } from "vitest";
import {
  attemptSummary,
  passK,
  regression,
  wilson,
  zeroBound,
  type PairedTask,
} from "./statistics.ts";

test("keeps failed and unstarted attempts in the denominators, never as fast successes", () => {
  const summary = attemptSummary([
    { status: "verified", elapsedMs: 10 },
    { status: "verified", elapsedMs: 30 },
    { status: "failed", elapsedMs: 1000 },
    { status: "rejected", elapsedMs: null },
    { status: "unsupported", elapsedMs: null },
  ]);

  expect(summary.attempts).toBe(5);
  expect(summary.counts.verified).toBe(2);
  expect(summary.counts.rejected).toBe(1);
  expect(summary.verifiedRate).toBe(0.4);
  expect(summary.verifiedLatencyMs.p95).toBe(30);
  expect(summary.startedLatencyMs.p95).toBe(1000);
  expect(summary.zeroFailureUpper95).toBeNull();
});

test("gives the 95% Wilson interval of a verified rate, inside [0, 1] at the edges", () => {
  // Wilson's score interval with z = 1.959 964: 8 of 10 is 0.490 to 0.943; 0 of 10 is 0 to 0.278.
  const some = wilson(8, 10);

  expect(some.rate).toBe(0.8);
  expect(some.ci[0]).toBeCloseTo(0.490_16, 4);
  expect(some.ci[1]).toBeCloseTo(0.943_32, 4);
  expect(wilson(0, 10).ci[0]).toBe(0);
  expect(wilson(0, 10).ci[1]).toBeCloseTo(0.277_53, 4);
  expect(wilson(10, 10).ci[0]).toBeCloseTo(0.722_47, 4);
  expect(wilson(10, 10).ci[1]).toBeCloseTo(1, 12);
});

test("takes pass^k as the mean over tasks of the chance that k trials drawn all pass", () => {
  const tasks = [
    { trials: 3, verified: 3 },
    { trials: 3, verified: 2 },
    { trials: 3, verified: 0 },
  ];

  expect(passK(tasks, 3)).toBeCloseTo(1 / 3, 12);
  expect(passK(tasks, 1)).toBeCloseTo(5 / 9, 12);
  // C(4, 3) / C(5, 3) = 4 / 10.
  expect(passK([{ trials: 5, verified: 4 }], 3)).toBeCloseTo(0.4, 12);
  expect(() => passK([{ trials: 2, verified: 2 }], 3)).toThrow(/fewer than 3 trials/);
});

test("states the bound a zero count can claim at its trial count, about 3 / n", () => {
  expect(zeroBound(100)).toBeCloseTo(0.029_513, 6);
  expect(zeroBound(300)).toBeCloseTo(0.009_936, 6);
  expect(zeroBound(300)).toBeLessThan(3 / 300);
});

const rate = (verified: number, trials = 3) => ({ verified, trials });
const paired = (run: readonly number[], baseline: readonly number[]): PairedTask[] =>
  run.map((verified, index) => ({
    task: `t${index}`,
    run: rate(verified),
    baseline: rate(baseline[index] ?? 0),
  }));

test("measures a regression in percentage points of verified rate, even where a rate is 0", () => {
  // Every task loses one trial in three; two of them pass none, where a log ratio is undefined.
  const result = regression(paired([2, 2, 2, 0, 0, 2], [3, 3, 3, 1, 1, 3]), { delta: 10, seed: 3 });

  expect(result.estimatePp).toBeCloseTo(-100 / 3, 9);
  expect(result.ciPp[1]).toBeLessThan(-10);
  expect(result.outcome).toBe("fails");
});

test("fails the regression rule only when the interval's upper bound is below -Δ", () => {
  const same = regression(paired([3, 2, 1, 3, 0, 2], [3, 2, 1, 3, 0, 2]), { delta: 10, seed: 3 });
  const mixed = regression(paired([3, 1, 3, 2, 0, 3, 2, 3], [2, 3, 3, 3, 1, 2, 3, 3]), {
    delta: 10,
    seed: 3,
  });

  expect(same.outcome).toBe("passes");
  expect(same.estimatePp).toBe(0);
  expect(mixed.ciPp[0]).toBeLessThan(-10);
  expect(mixed.ciPp[1]).toBeGreaterThan(-10);
  expect(mixed.outcome).toBe("inconclusive");
});

test("reports fewer than five paired tasks as insufficient-n, never as a failure", () => {
  const result = regression(paired([0, 0, 0, 0], [3, 3, 3, 3]), { delta: 10, seed: 3 });

  expect(result.outcome).toBe("insufficient-n");
  expect(result.n).toBe(4);
});
