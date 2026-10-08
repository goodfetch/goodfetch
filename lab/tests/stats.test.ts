import { test } from 'node:test';
import assert from 'node:assert/strict';
import { wilcoxonSignedRank, hodgesLehmann } from '../src/stats/rank.ts';
import { normalQuantile, studentTQuantile, tost } from '../src/stats/tost.ts';
import { blockBootstrapCI } from '../src/stats/bootstrap.ts';

const close = (actual: number, expected: number, tolerance: number) => assert.ok(Math.abs(actual - expected) <= tolerance, `${actual} vs ${expected}`);

test('exact Wilcoxon signed-rank p-values match the enumerated null distribution', () => {
  // n = 5, all differences positive: only 1 of 32 sign patterns is as extreme, two-sided 2/32.
  close(wilcoxonSignedRank([1, 2, 3, 4, 5]).pValue, 2 / 32, 1e-12);
  // n = 10 with W- = 3 (ranks 1 and 2 negative): subsets of 1..10 summing to at most 3 are {}, {1}, {2}, {3}, {1,2}: 5 of 1024.
  const diffs = [-0.1, -0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];
  const result = wilcoxonSignedRank(diffs);
  assert.equal(result.method, 'exact');
  close(result.pValue, 2 * 5 / 1024, 1e-12);
});

test('the one-sample Hodges-Lehmann estimate is the median of Walsh averages', () => {
  // Walsh averages of [1,2,3,4,10] sorted: 1,1.5,2,2,2.5,2.5,3,3,3.5,4,5.5,6,6.5,7,10; the 8th of 15 is 3.
  assert.equal(hodgesLehmann([1, 2, 3, 4, 10]), 3);
  assert.equal(hodgesLehmann([4]), 4);
});

test('normal and Student t quantiles match the published tables', () => {
  close(normalQuantile(0.975), 1.959964, 1e-6);
  close(normalQuantile(0.8), 0.841621, 1e-6);
  close(studentTQuantile(0.95, 9), 1.833113, 1e-5);
  close(studentTQuantile(0.975, 4), 2.776445, 1e-5);
  close(studentTQuantile(0.95, 1), 6.313752, 1e-4);
});

test('TOST uses the 1 - 2 alpha t interval and declares equivalence only inside the margin', () => {
  // diffs [0, 0.04]: mean 0.02, sd 0.0282843, t(0.95, 1) = 6.313752, half width 0.126275.
  const result = tost([0, 0.04], 0.2, 0.05);
  close(result.ci[0], -0.106275, 1e-5);
  close(result.ci[1], 0.146275, 1e-5);
  assert.equal(result.equivalent, true);
  assert.equal(tost([0, 0.04], 0.1, 0.05).equivalent, false);
});

test('the block bootstrap interval is seeded, contains the estimate and widens with spread', () => {
  const tight = [0.10, 0.11, 0.09, 0.10, 0.12, 0.08];
  const wide = [0.30, -0.10, 0.25, -0.05, 0.40, -0.20];
  const first = blockBootstrapCI(tight, { resamples: 4000, seed: 7, alpha: 0.05 });
  assert.deepEqual(blockBootstrapCI(tight, { resamples: 4000, seed: 7, alpha: 0.05 }), first);
  assert.ok(first.ci[0] <= first.estimate && first.estimate <= first.ci[1]);
  const spread = blockBootstrapCI(wide, { resamples: 4000, seed: 7, alpha: 0.05 });
  assert.ok(spread.ci[1] - spread.ci[0] > first.ci[1] - first.ci[0]);
});
