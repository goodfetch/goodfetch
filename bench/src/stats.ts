// The statistics a verdict rests on, carried over from the lab: the seeded
// percentile bootstrap over whole blocks (ADR 0025), the exact signed-rank
// test and the Hodges–Lehmann estimate as its robustness checks, and the
// normal and Student t quantiles with the two one-sided tests (TOST) of
// equivalence. Each takes the paired per-block figures as given.
import { random } from "./order.ts";

/** The `index`th of `values`, which the caller knows is inside it. */
function at(values: readonly number[], index: number): number {
  const value = values[index];
  if (value === undefined) {
    throw new Error(`invariant: index ${index} is inside the ${values.length} values`);
  }
  return value;
}

const mean = (values: readonly number[]): number =>
  values.reduce((sum, value) => sum + value, 0) / values.length;

/** A seeded bootstrap interval of a mean. */
export interface Bootstrap {
  readonly estimate: number;
  readonly ci: readonly [number, number];
  readonly resamples: number;
  readonly seed: number;
}

/**
 * The percentile bootstrap of the mean of `values`, one value a block: blocks
 * are resampled whole, never the attempts inside them, and the same seed
 * draws the same interval.
 */
export function blockBootstrap(
  values: readonly number[],
  options: { readonly resamples?: number; readonly seed: number; readonly alpha?: number },
): Bootstrap {
  const { resamples = 10_000, seed, alpha = 0.05 } = options;
  if (values.length === 0) {
    throw new Error("a bootstrap needs at least one block");
  }
  const draw = random(seed);
  const means: number[] = [];
  for (let resample = 0; resample < resamples; resample += 1) {
    let sum = 0;
    for (let index = 0; index < values.length; index += 1) {
      sum += at(values, Math.floor(draw() * values.length));
    }
    means.push(sum / values.length);
  }
  means.sort((left, right) => left - right);
  const percentile = (q: number): number =>
    at(means, Math.min(resamples - 1, Math.max(0, Math.floor(q * resamples))));
  return {
    estimate: mean(values),
    ci: [percentile(alpha / 2), percentile(1 - alpha / 2)],
    resamples,
    seed,
  };
}

/** The signed-rank test of paired differences: exact without ties up to 25 of them, normal otherwise. */
export interface SignedRank {
  readonly n: number;
  readonly wPlus: number;
  readonly pValue: number;
  readonly method: "exact" | "normal";
}

/** Wilcoxon's two-sided signed-rank test that `differences` centre on 0; zeros are dropped. */
export function wilcoxonSignedRank(differences: readonly number[]): SignedRank {
  const sorted = differences
    .filter((difference) => difference !== 0)
    .map((difference) => ({ size: Math.abs(difference), positive: difference > 0 }))
    .toSorted((left, right) => left.size - right.size);
  const n = sorted.length;
  if (n === 0) {
    return { n: 0, wPlus: 0, pValue: 1, method: "exact" };
  }
  // Tied sizes share their mean rank; each run of ties adds t^3 - t to the correction.
  const ranks: number[] = [];
  let tieCorrection = 0;
  for (let first = 0; first < n;) {
    let last = first;
    while (last + 1 < n && sorted[last + 1]?.size === sorted[first]?.size) {
      last += 1;
    }
    for (let index = first; index <= last; index += 1) {
      ranks.push((first + last) / 2 + 1);
    }
    const tied = last - first + 1;
    tieCorrection += tied ** 3 - tied;
    first = last + 1;
  }
  const wPlus = sorted.reduce(
    (sum, item, index) => sum + (item.positive ? at(ranks, index) : 0),
    0,
  );
  const total = (n * (n + 1)) / 2;
  if (tieCorrection === 0 && n <= 25) {
    // The exact null distribution of W+, by counting the sign assignments of each sum.
    const counts = Array.from({ length: total + 1 }, (_, sum): number => (sum === 0 ? 1 : 0));
    for (let rank = 1; rank <= n; rank += 1) {
      for (let sum = total; sum >= rank; sum -= 1) {
        counts[sum] = at(counts, sum) + at(counts, sum - rank);
      }
    }
    const all = 2 ** n;
    const lower = counts.slice(0, Math.floor(wPlus) + 1).reduce((a, b) => a + b, 0) / all;
    const upper = counts.slice(Math.ceil(wPlus)).reduce((a, b) => a + b, 0) / all;
    return { n, wPlus, pValue: Math.min(1, 2 * Math.min(lower, upper)), method: "exact" };
  }
  const variance = (n * (n + 1) * (2 * n + 1)) / 24 - tieCorrection / 48;
  const z = (Math.abs(wPlus - total / 2) - 0.5) / Math.sqrt(variance);
  return {
    n,
    wPlus,
    pValue: Math.min(1, 2 * (1 - normalCdf(Math.max(0, z)))),
    method: "normal",
  };
}

/** The one-sample Hodges–Lehmann estimate: the median of the Walsh averages (d_i + d_j) / 2, i <= j. */
export function hodgesLehmann(values: readonly number[]): number {
  const walsh: number[] = [];
  for (let i = 0; i < values.length; i += 1) {
    for (let j = i; j < values.length; j += 1) {
      walsh.push((at(values, i) + at(values, j)) / 2);
    }
  }
  walsh.sort((left, right) => left - right);
  const middle = Math.floor(walsh.length / 2);
  return walsh.length % 2 === 1
    ? at(walsh, middle)
    : (at(walsh, middle - 1) + at(walsh, middle)) / 2;
}

/** The standard normal CDF, by Abramowitz–Stegun 7.1.26 on erf (absolute error below 1.5e-7). */
export function normalCdf(x: number): number {
  const t = 1 / (1 + (0.327_591_1 * Math.abs(x)) / Math.SQRT2);
  const polynomial =
    ((((1.061_405_429 * t - 1.453_152_027) * t + 1.421_413_741) * t - 0.284_496_736) * t +
      0.254_829_592) *
    t;
  const erf = 1 - polynomial * Math.exp((-x * x) / 2);
  return x >= 0 ? (1 + erf) / 2 : (1 - erf) / 2;
}

/** Evaluates the polynomial with `coefficients`, highest degree first, at `x`. */
function horner(coefficients: readonly number[], x: number): number {
  return coefficients.reduce((sum, coefficient) => sum * x + coefficient, 0);
}

// Acklam's rational approximation of the inverse normal CDF.
const acklamA = [
  -3.969_683_028_665_376e1, 2.209_460_984_245_205e2, -2.759_285_104_469_687e2,
  1.383_577_518_672_69e2, -3.066_479_806_614_716e1, 2.506_628_277_459_239,
];
const acklamB = [
  -5.447_609_879_822_406e1, 1.615_858_368_580_409e2, -1.556_989_798_598_866e2,
  6.680_131_188_771_972e1, -1.328_068_155_288_572e1, 1,
];
const acklamC = [
  -7.784_894_002_430_293e-3, -3.223_964_580_411_365e-1, -2.400_758_277_161_838,
  -2.549_732_539_343_734, 4.374_664_141_464_968, 2.938_163_982_698_783,
];
const acklamD = [
  7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416,
  1,
];

/** The standard normal quantile of `p`, by Acklam's approximation (relative error below 1.2e-9). */
export function normalQuantile(p: number): number {
  if (!(p > 0 && p < 1)) {
    throw new Error("a normal quantile needs 0 < p < 1");
  }
  const tail = (q: number): number => horner(acklamC, q) / horner(acklamD, q);
  if (p < 0.024_25) {
    return tail(Math.sqrt(-2 * Math.log(p)));
  }
  if (p > 1 - 0.024_25) {
    return -tail(Math.sqrt(-2 * Math.log(1 - p)));
  }
  const q = p - 0.5;
  return (horner(acklamA, q * q) * q) / horner(acklamB, q * q);
}

// Lanczos's coefficients for g = 7, n = 9.
const lanczos = [
  0.999_999_999_999_809_9, 676.520_368_121_885_1, -1_259.139_216_722_402_8, 771.323_428_777_653_1,
  -176.615_029_162_140_6, 12.507_343_278_686_905, -0.138_571_095_265_720_12,
  9.984_369_578_019_571_6e-6, 1.505_632_735_149_311_6e-7,
];

function logGamma(x: number): number {
  if (x < 0.5) {
    return Math.log(Math.PI / Math.sin(Math.PI * x)) - logGamma(1 - x);
  }
  const shifted = x - 1;
  let sum = at(lanczos, 0);
  for (let index = 1; index < lanczos.length; index += 1) {
    sum += at(lanczos, index) / (shifted + index);
  }
  const t = shifted + 7.5;
  return 0.5 * Math.log(2 * Math.PI) + (shifted + 0.5) * Math.log(t) - t + Math.log(sum);
}

/** Keeps a continued fraction's term away from 0. */
const nonZero = (value: number): number => (Math.abs(value) < 1e-300 ? 1e-300 : value);

/** The regularized incomplete beta I_x(a, b), by Lentz's continued fraction. */
function incompleteBeta(x: number, a: number, b: number): number {
  if (x <= 0) {
    return 0;
  }
  if (x >= 1) {
    return 1;
  }
  if (x > (a + 1) / (a + b + 2)) {
    return 1 - incompleteBeta(1 - x, b, a);
  }
  const front = Math.exp(
    logGamma(a + b) - logGamma(a) - logGamma(b) + a * Math.log(x) + b * Math.log(1 - x),
  );
  let c = 1;
  let d = 1 / nonZero(1 - ((a + b) * x) / (a + 1));
  let f = d;
  for (let m = 1; m <= 300; m += 1) {
    const even = (m * (b - m) * x) / ((a + 2 * m - 1) * (a + 2 * m));
    const odd = (-(a + m) * (a + b + m) * x) / ((a + 2 * m) * (a + 2 * m + 1));
    for (const numerator of [even, odd]) {
      d = 1 / nonZero(1 + numerator * d);
      c = nonZero(1 + numerator / c);
      f *= c * d;
    }
    if (Math.abs(c * d - 1) < 1e-14) {
      break;
    }
  }
  return (front * f) / a;
}

/** Student's t CDF at `t` with `df` degrees of freedom. */
export function studentTCdf(t: number, df: number): number {
  const tail = 0.5 * incompleteBeta(df / (df + t * t), df / 2, 0.5);
  return t >= 0 ? 1 - tail : tail;
}

/** Student's t quantile of `p` with `df` degrees of freedom, by bisection of the CDF. */
export function studentTQuantile(p: number, df: number): number {
  if (!(p > 0 && p < 1) || !(df > 0)) {
    throw new Error("a Student t quantile needs 0 < p < 1 and df > 0");
  }
  let low = -1e4;
  let high = 1e4;
  for (let step = 0; step < 200; step += 1) {
    const middle = (low + high) / 2;
    if (studentTCdf(middle, df) < p) {
      low = middle;
    } else {
      high = middle;
    }
  }
  return (low + high) / 2;
}

/** The two one-sided tests of a paired mean difference against ±`margin`. */
export interface Tost {
  readonly n: number;
  readonly mean: number;
  readonly sd: number;
  readonly ci: readonly [number, number];
  readonly margin: number;
  readonly equivalent: boolean;
}

/** TOST at level `alpha`: equivalent when the (1 - 2α) t interval of the mean lies inside (-margin, margin). */
export function tost(differences: readonly number[], margin: number, alpha: number): Tost {
  const n = differences.length;
  if (n < 2) {
    throw new Error("TOST needs at least two paired differences");
  }
  const centre = mean(differences);
  const sd = Math.sqrt(
    differences.reduce((sum, difference) => sum + (difference - centre) ** 2, 0) / (n - 1),
  );
  const half = (studentTQuantile(1 - alpha, n - 1) * sd) / Math.sqrt(n);
  const ci: [number, number] = [centre - half, centre + half];
  return { n, mean: centre, sd, ci, margin, equivalent: ci[0] > -margin && ci[1] < margin };
}
