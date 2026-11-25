// The figures a report gives beside its verdicts: a set of attempts'
// counts and latencies, carried over from the lab, and what Evaluations
// read from the benchmark's one statistics engine (ADR 0065): the verified
// rate with its 95% Wilson interval, pass^k, the bound a zero count can
// claim at its trial count, and the regression statistic, in percentage
// points of verified rate against a baseline, never a log ratio.
import { countStatuses, type Counts } from "./cell.ts";
import type { AttemptStatus } from "./errors.ts";
import { quantile } from "./order.ts";
import { blockBootstrap, normalQuantile } from "./stats.ts";

/** One attempt's status and duration; an attempt that never started has none. */
export interface Sample {
  readonly status: AttemptStatus;
  readonly elapsedMs: number | null;
}

/** Nearest-rank quantiles of some durations. */
export interface Quantiles {
  readonly n: number;
  readonly p50: number | null;
  readonly p95: number | null;
  readonly p99: number | null;
}

/** A set of attempts' counts, verified rate and latencies. */
export interface AttemptSummary {
  readonly attempts: number;
  readonly counts: Counts;
  /** Verified attempts over every attempt, failed, rejected and unsupported ones included. */
  readonly verifiedRate: number | null;
  /** The verified attempts' durations: a failure is never a fast success. */
  readonly verifiedLatencyMs: Quantiles;
  /** Every started attempt's duration. */
  readonly startedLatencyMs: Quantiles;
  /** When every attempt was verified, the highest failure rate that is still 95% plausible. */
  readonly zeroFailureUpper95: number | null;
}

function quantiles(values: readonly number[]): Quantiles {
  return {
    n: values.length,
    p50: quantile(values, 0.5),
    p95: quantile(values, 0.95),
    p99: quantile(values, 0.99),
  };
}

/** The summary of `samples`: no attempt leaves a denominator. */
export function attemptSummary(samples: readonly Sample[]): AttemptSummary {
  const started: number[] = [];
  const verified: number[] = [];
  for (const sample of samples) {
    if (sample.elapsedMs === null) {
      continue;
    }
    if (!Number.isFinite(sample.elapsedMs) || sample.elapsedMs < 0) {
      throw new Error(
        `an attempt's duration must be finite and non-negative, not ${sample.elapsedMs}`,
      );
    }
    started.push(sample.elapsedMs);
    if (sample.status === "verified") {
      verified.push(sample.elapsedMs);
    }
  }
  const counts = countStatuses(samples);
  const allVerified = samples.length > 0 && counts.verified === samples.length;
  return {
    attempts: samples.length,
    counts,
    verifiedRate: samples.length > 0 ? counts.verified / samples.length : null,
    verifiedLatencyMs: quantiles(verified),
    startedLatencyMs: quantiles(started),
    zeroFailureUpper95: allVerified ? zeroBound(samples.length) : null,
  };
}

/** A rate and its interval. */
export interface Interval {
  readonly rate: number;
  readonly ci: readonly [number, number];
}

/** The verified rate of `verified` of `trials`, with its Wilson score interval at `confidence`. */
export function wilson(verified: number, trials: number, confidence = 0.95): Interval {
  if (!(trials > 0) || verified < 0 || verified > trials) {
    throw new Error(
      `a Wilson interval needs 0 <= verified <= trials and trials > 0, not ${verified} of ${trials}`,
    );
  }
  const z = normalQuantile(1 - (1 - confidence) / 2);
  const rate = verified / trials;
  const shrink = 1 + (z * z) / trials;
  const centre = (rate + (z * z) / (2 * trials)) / shrink;
  const half =
    (z / shrink) * Math.sqrt((rate * (1 - rate)) / trials + (z * z) / (4 * trials * trials));
  return { rate, ci: [Math.max(0, centre - half), Math.min(1, centre + half)] };
}

/** One task's trials and how many the oracle verified. */
export interface TaskTrials {
  readonly trials: number;
  readonly verified: number;
}

/** The number of ways to choose `k` of `n`. */
function choose(n: number, k: number): number {
  let ways = 1;
  for (let index = 0; index < k; index += 1) {
    ways = (ways * (n - index)) / (index + 1);
  }
  return ways;
}

/**
 * pass^k: the mean over tasks of the chance that `k` of a task's trials,
 * drawn without replacement, all pass, C(verified, k) / C(trials, k).
 */
export function passK(tasks: readonly TaskTrials[], k: number): number {
  if (tasks.length === 0 || !(k >= 1)) {
    throw new Error("pass^k needs at least one task and k >= 1");
  }
  let sum = 0;
  for (const task of tasks) {
    if (task.trials < k) {
      throw new Error(`a task with ${task.trials} trials has fewer than ${k} trials`);
    }
    sum += choose(task.verified, k) / choose(task.trials, k);
  }
  return sum / tasks.length;
}

/**
 * The bound a zero count can claim at `trials`: the highest rate at which
 * seeing none in that many trials still has a 1 - `confidence` chance,
 * 1 - (1 - confidence)^(1 / trials), about 3 / trials at 95%.
 */
export function zeroBound(trials: number, confidence = 0.95): number {
  if (!(trials > 0)) {
    throw new Error("a zero count's bound needs at least one trial");
  }
  return 1 - (1 - confidence) ** (1 / trials);
}

/** One task's verified trials in the run and in its baseline, on the same pinned models. */
export interface PairedTask {
  readonly task: string;
  readonly run: TaskTrials;
  readonly baseline: TaskTrials;
}

/** The regression rule's outcome: it fails only on an interval wholly below -Δ. */
export type RegressionOutcome = "fails" | "passes" | "inconclusive" | "insufficient-n";

/** The regression statistic and its verdict. */
export interface Regression {
  readonly outcome: RegressionOutcome;
  /** Paired tasks. */
  readonly n: number;
  /** The mean over tasks of the run's verified rate minus its baseline's, in percentage points. */
  readonly estimatePp: number;
  readonly ciPp: readonly [number, number];
  readonly deltaPp: number;
}

/** The fewest paired tasks the regression rule judges, as ADR 0025's fewest blocks. */
const minimumTasks = 5;

/**
 * Evaluations' regression statistic (ADR 0065): the mean over tasks of the
 * paired difference in verified rate, in percentage points, with the
 * seeded block-bootstrap interval, each task's trials resampled as one
 * block. It fails when the interval's upper bound is below -`delta`,
 * passes when its lower bound is above it, and is inconclusive otherwise.
 */
export function regression(
  tasks: readonly PairedTask[],
  options: { readonly delta: number; readonly seed: number; readonly alpha?: number },
): Regression {
  const { delta, seed, alpha = 0.05 } = options;
  const differences = tasks.map(
    (task) =>
      100 * (task.run.verified / task.run.trials - task.baseline.verified / task.baseline.trials),
  );
  if (differences.length === 0) {
    throw new Error("the regression statistic needs at least one paired task");
  }
  const { estimate, ci } = blockBootstrap(differences, { seed, alpha });
  const outcome: RegressionOutcome =
    differences.length < minimumTasks
      ? "insufficient-n"
      : ci[1] < -delta
        ? "fails"
        : ci[0] > -delta
          ? "passes"
          : "inconclusive";
  return { outcome, n: differences.length, estimatePp: estimate, ciPp: ci, deltaPp: delta };
}
