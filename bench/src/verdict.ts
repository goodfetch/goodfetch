// Mechanical verdicts (ADR 0025), carried over from the lab: the paired
// per-block log ratios of an option against its baseline, judged against
// the smallest effect its decision declared, by a fixed rule. A failed
// contract gate blocks the verdict whatever the effect; fewer blocks than
// the pilot's spread asks, and never fewer than five, is `insufficient-n`.
// The interval is the seeded block bootstrap's; Hodges–Lehmann and the
// exact signed-rank test go beside it as robustness checks.
import {
  blockBootstrap,
  hodgesLehmann,
  normalQuantile,
  wilcoxonSignedRank,
  type SignedRank,
} from "./stats.ts";

/** A verdict's mechanical outcome. */
export type Outcome =
  | "better"
  | "worse"
  | "equivalent"
  | "trivial"
  | "non-inferior"
  | "inconclusive"
  | "blocked-by-contract"
  | "insufficient-n";

/** A decision's fixed rule: what it tests, at what level and power. */
export interface Rule {
  readonly kind: "superiority" | "non-inferiority" | "equivalence";
  readonly alpha: number;
  readonly power: number;
}

/** What a verdict is computed from. */
export interface DecideInput {
  /** ln(option / baseline) of the primary metric, one a paired block. */
  readonly logRatios: readonly number[];
  /** Whether the metric is better lower or higher. */
  readonly direction: "lower" | "higher";
  readonly rule: Rule;
  /** The smallest effect of interest, as a ratio: 0.1 is 10%. */
  readonly delta: number;
  readonly gates: readonly { readonly id: string; readonly ok: boolean }[];
  readonly seed: number;
  /** The fewest blocks any verdict takes; five unless the decision asks more. */
  readonly minBlocks?: number;
}

/** The estimate of a verdict, with its robustness checks. */
export interface Estimate {
  readonly meanLogRatio: number;
  readonly ratio: number;
  readonly ci: readonly [number, number];
  readonly ratioCi: readonly [number, number];
  readonly hodgesLehmannRatio: number;
  readonly wilcoxon: SignedRank;
  /** The interval's name, such as `95% block bootstrap`. */
  readonly interval: string;
}

/** A verdict: its outcome, the blocks it read and asked for, and its estimate. */
export interface Verdict {
  readonly outcome: Outcome;
  readonly n: number;
  /** The margin, ln(1 + delta). */
  readonly margin: number;
  /** The blocks the pilot's spread asks for the margin; null before two blocks or on a failed gate. */
  readonly requiredN: number | null;
  readonly estimate: Estimate | null;
}

const minimumBlocks = 5;

function standardDeviation(values: readonly number[]): number {
  const mean = values.reduce((sum, value) => sum + value, 0) / values.length;
  return Math.sqrt(
    values.reduce((sum, value) => sum + (value - mean) ** 2, 0) / (values.length - 1),
  );
}

/** The verdict `input`'s blocks give under its rule. */
export function decide(input: DecideInput): Verdict {
  const { logRatios, direction, rule, delta, gates, seed } = input;
  const minBlocks = input.minBlocks ?? minimumBlocks;
  const n = logRatios.length;
  const margin = Math.log(1 + delta);
  if (gates.some((gate) => !gate.ok)) {
    return { outcome: "blocked-by-contract", n, margin, requiredN: null, estimate: null };
  }
  // Improvement space: positive means the option beats its baseline.
  const improvement = logRatios.map((value) => (direction === "lower" ? -value : value));
  const level = rule.kind === "superiority" ? rule.alpha / 2 : rule.alpha;
  const requiredN =
    n >= 2
      ? Math.max(
          minBlocks,
          Math.ceil(
            (((normalQuantile(1 - level) + normalQuantile(rule.power)) *
              standardDeviation(improvement)) /
              margin) **
              2,
          ),
        )
      : null;
  if (requiredN === null || n < requiredN) {
    return {
      outcome: "insufficient-n",
      n,
      margin,
      requiredN,
      estimate: n > 0 ? estimateOf(logRatios, seed, rule) : null,
    };
  }
  const estimate = estimateOf(logRatios, seed, rule);
  const [low, high] = direction === "lower" ? [-estimate.ci[1], -estimate.ci[0]] : estimate.ci;
  return { outcome: outcomeOf(low, high, margin, rule), n, margin, requiredN, estimate };
}

/** The outcome of an improvement interval [low, high] against ±margin. */
function outcomeOf(low: number, high: number, margin: number, rule: Rule): Outcome {
  if (low > margin) {
    return "better";
  }
  if (high < -margin) {
    return "worse";
  }
  if (rule.kind === "non-inferiority") {
    return low > -margin ? "non-inferior" : "inconclusive";
  }
  if (low > -margin && high < margin) {
    // Inside the margin yet away from 0 is a real effect too small to matter.
    const real = low > 0 || high < 0;
    return real && rule.kind === "superiority" ? "trivial" : "equivalent";
  }
  return "inconclusive";
}

/**
 * The block-bootstrap estimate: a two-sided (1 - α) interval for
 * superiority, the TOST (1 - 2α) interval for equivalence and
 * non-inferiority.
 */
function estimateOf(logRatios: readonly number[], seed: number, rule: Rule): Estimate {
  const alpha = rule.kind === "superiority" ? rule.alpha : 2 * rule.alpha;
  const bootstrap = blockBootstrap(logRatios, { resamples: 10_000, seed, alpha });
  return {
    meanLogRatio: bootstrap.estimate,
    ratio: Math.exp(bootstrap.estimate),
    ci: bootstrap.ci,
    ratioCi: [Math.exp(bootstrap.ci[0]), Math.exp(bootstrap.ci[1])],
    hodgesLehmannRatio: Math.exp(hodgesLehmann(logRatios)),
    wilcoxon: wilcoxonSignedRank(logRatios),
    interval: `${Math.round((1 - alpha) * 100)}% block bootstrap`,
  };
}
