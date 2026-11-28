// Three views, result counts and the cost model (ADR 0025):
//   1. Raw view (throughput / latency), never emitted without verified,
//      false-success and `needs-review` counts beside it.
//   2. Per-verified-result view (time, CPU and memory per verified result).
//   3. Cost view (dollars per 1,000 verified results), summing host hourly
//      price × wall time (or `available: false` when the machine has no public
//      hourly price), plus model, proxy, CAPTCHA and hosted-browser spend
//      including every failed attempt.
//
// A Plan's one-time cost (compilation and Repairs) is spread over 1, 10, 100
// and 1,000 runs declared before measuring. Human time is a separate column,
// never folded into dollars.
import type { Attempt, Cell } from "./cell.ts";
import { BenchError } from "./errors.ts";
import { quantile } from "./order.ts";
import type { PriceTable } from "./prices.ts";

/** Standard run counts over which a Plan's one-time cost is amortized (ADR 0025). */
export const PLAN_AMORTIZATION_RUN_COUNTS = [1, 10, 100, 1000] as const;

/** Result counts carried beside every raw figure and every view. */
export interface ResultCounts {
  /** Attempts confirmed correct and complete by the oracle (including confirmed `needs-review`). */
  readonly verified: number;
  /** Attempts the tool reported as `success` that the oracle refuted. */
  readonly false_success: number;
  /** Attempts the tool reported as `needs-review` (never counted as false success). */
  readonly needs_review: number;
  /** Subset of `needs_review` confirmed by the oracle as verified. */
  readonly needs_review_verified: number;
  /** Failed, incomplete, timed-out or crashed attempts. */
  readonly failed: number;
  /** Total measured attempts in the window. */
  readonly total_attempts: number;
}

/** View 1: Raw throughput and latency, constructively paired with `counts`. */
export interface RawView {
  readonly raw_tasks_per_second: number | null;
  readonly raw_p50_ms: number | null;
  readonly raw_p95_ms: number | null;
  readonly counts: ResultCounts;
}

/** View 2: Time, CPU and memory per verified result, with `counts`. */
export interface PerVerifiedView {
  readonly verified_tasks_per_second: number | null;
  readonly task_p50_ms_verified: number | null;
  readonly task_p95_ms_verified: number | null;
  readonly cpu_ms_per_verified_task: number | null;
  readonly pss_peak_bytes: number | null;
  readonly rss_peak_bytes: number | null;
  readonly counts: ResultCounts;
}

/** Itemized dollar spend across all attempts (failed attempts included). */
export interface CostBreakdown {
  readonly host_usd: number;
  readonly model_usd: number;
  readonly proxy_usd: number;
  readonly captcha_usd: number;
  readonly hosted_browser_usd: number;
  readonly total_usd: number;
  /** Portion of `total_usd` incurred by non-verified attempts (failed, false_success, incomplete, timeout, crashed). */
  readonly failed_attempt_usd: number;
}

/** Plan one-time cost (compilation and Repairs) amortized over a declared run count. */
export interface PlanAmortizationRow {
  readonly runs: number;
  readonly one_time_usd: number;
  readonly amortized_per_run_usd: number;
  readonly total_dollars_per_1k_verified: number;
}

/** View 3: Dollars per 1,000 verified results, or explicit unavailability when the machine has no public price or zero verified results. */
export type CostView =
  | {
      readonly available: true;
      readonly dollars_per_1k_verified: number;
      readonly breakdown: CostBreakdown;
      readonly plan_amortization: readonly PlanAmortizationRow[] | null;
      readonly price_table: {
        readonly version: number;
        readonly effective_date: string;
        readonly source: string;
      };
      readonly host_pricing: {
        readonly machine_id: string;
        readonly hourly_usd: number;
        readonly source: string;
        readonly date: string;
      };
      readonly counts: ResultCounts;
    }
  | {
      readonly available: false;
      readonly reason: string;
      readonly counts: ResultCounts;
    };

/** Human intervention time: always a separate column, never folded into dollars. */
export interface HumanTimeColumn {
  readonly total_seconds: number;
  readonly seconds_per_verified: number | null;
}

/** Complete three-view report for one adapter + task pair. */
export interface ThreeViewReport {
  readonly adapter: string;
  readonly task: string;
  readonly counts: ResultCounts;
  readonly raw: RawView;
  readonly per_verified: PerVerifiedView;
  readonly cost: CostView;
  readonly human_time: HumanTimeColumn;
}

/** Declared Plan one-time cost to amortize over run counts declared before measuring. */
export interface PlanOneTimeCostInput {
  /** One-time compilation and Repair cost in USD. */
  readonly oneTimeUsd: number;
  /** Run counts declared before measuring (must include 1, 10, 100 and 1,000). */
  readonly declaredRunCounts?: readonly number[] | undefined;
  /** Number of verified results produced per run (defaults to this report's verified count). */
  readonly verifiedPerRun?: number | undefined;
}

/** Inputs to compute ADR 0025's three views for a cell or set of attempts. */
export interface BuildThreeViewsInput {
  readonly adapter: string;
  readonly task: string;
  readonly attempts: readonly Attempt[];
  /** Optional summarized cells for CPU / PSS / RSS readings. */
  readonly cells?: readonly Cell[] | undefined;
  /** Wall time of the measured window in ms; defaults to sum of attempt task_ms when omitted. */
  readonly windowMs?: number | null | undefined;
  /** Machine ID in `prices.hosts`, or `null` for a machine with no public hourly price (e.g. owner's laptop). */
  readonly hostMachineId: string | null;
  readonly prices: PriceTable;
  readonly planOneTimeCost?: PlanOneTimeCostInput | undefined;
}

const MS_PER_HOUR = 3_600_000;
const BYTES_PER_GB = 1_000_000_000;

function isFailedAttemptStatus(status: Attempt["status"]): boolean {
  return (
    status === "failed" ||
    status === "false_success" ||
    status === "incomplete" ||
    status === "timeout" ||
    status === "crashed"
  );
}

/** Summarizes verified, false-success and `needs-review` counts from attempts. */
export function countResults(attempts: readonly Attempt[]): ResultCounts {
  let verified = 0;
  let falseSuccess = 0;
  let needsReview = 0;
  let needsReviewVerified = 0;
  let failed = 0;

  for (const one of attempts) {
    const isNeedsReview = one.tool_status === "needs-review";
    if (isNeedsReview) {
      needsReview += 1;
      if (one.status === "verified") {
        needsReviewVerified += 1;
      }
    }
    if (one.status === "verified") {
      verified += 1;
    } else if (one.status === "false_success" && !isNeedsReview) {
      // ADR 0025: a `needs-review` result is never a false success.
      falseSuccess += 1;
    } else if (isFailedAttemptStatus(one.status)) {
      failed += 1;
    }
  }

  return {
    verified,
    false_success: falseSuccess,
    needs_review: needsReview,
    needs_review_verified: needsReviewVerified,
    failed,
    total_attempts: attempts.length,
  };
}

interface DirectSpendBreakdown {
  readonly model_usd: number;
  readonly proxy_usd: number;
  readonly captcha_usd: number;
  readonly hosted_browser_usd: number;
  readonly total_usd: number;
}

function attemptDirectSpendBreakdown(attempt: Attempt, prices: PriceTable): DirectSpendBreakdown {
  const usage = attempt.adapter_metrics?.cost_usage;
  if (usage === undefined) {
    return { model_usd: 0, proxy_usd: 0, captcha_usd: 0, hosted_browser_usd: 0, total_usd: 0 };
  }
  let modelUsd = 0;
  let proxyUsd = 0;
  let captchaUsd = 0;
  let hostedBrowserUsd = 0;
  if (usage.model !== undefined) {
    const rate = prices.models[usage.model.id];
    if (rate === undefined) {
      throw new BenchError(
        "invalid_document",
        `price table v${prices.version} has no model rate for ${usage.model.id}`,
      );
    }
    modelUsd =
      (usage.model.input_tokens / 1_000_000) * rate.input_per_1m_usd +
      (usage.model.output_tokens / 1_000_000) * rate.output_per_1m_usd;
  }
  if (usage.proxy !== undefined) {
    const rate = prices.proxies[usage.proxy.tier];
    if (rate === undefined) {
      throw new BenchError(
        "invalid_document",
        `price table v${prices.version} has no proxy rate for ${usage.proxy.tier}`,
      );
    }
    proxyUsd =
      ((usage.proxy.bytes ?? 0) / BYTES_PER_GB) * rate.per_gb_usd +
      (usage.proxy.requests ?? 0) * (rate.per_request_usd ?? 0);
  }
  if (usage.captcha !== undefined) {
    const rate = prices.captchas[usage.captcha.kind];
    if (rate === undefined) {
      throw new BenchError(
        "invalid_document",
        `price table v${prices.version} has no CAPTCHA rate for ${usage.captcha.kind}`,
      );
    }
    captchaUsd = (usage.captcha.solves / 1_000) * rate.per_1k_solves_usd;
  }
  if (usage.hosted_browser !== undefined) {
    const rate = prices.hosted_browsers[usage.hosted_browser.vendor];
    if (rate === undefined) {
      throw new BenchError(
        "invalid_document",
        `price table v${prices.version} has no hosted browser rate for ${usage.hosted_browser.vendor}`,
      );
    }
    hostedBrowserUsd = usage.hosted_browser.minutes * rate.per_browser_minute_usd;
  }
  return {
    model_usd: modelUsd,
    proxy_usd: proxyUsd,
    captcha_usd: captchaUsd,
    hosted_browser_usd: hostedBrowserUsd,
    total_usd: modelUsd + proxyUsd + captchaUsd + hostedBrowserUsd,
  };
}

/** Validates and sorts Plan amortization run counts declared before measuring (must include 1, 10, 100 and 1,000). */
export function resolveAmortizationCounts(declared?: readonly number[]): readonly number[] {
  if (declared === undefined) {
    return PLAN_AMORTIZATION_RUN_COUNTS;
  }
  for (const required of PLAN_AMORTIZATION_RUN_COUNTS) {
    if (!declared.includes(required)) {
      throw new BenchError(
        "invalid_document",
        `declared Plan amortization run counts must include ${required}`,
      );
    }
  }
  for (const count of declared) {
    if (!Number.isInteger(count) || count <= 0) {
      throw new BenchError(
        "invalid_document",
        `Plan amortization run count must be a positive integer, got ${count}`,
      );
    }
  }
  return declared.toSorted((a, b) => a - b);
}

function medianOrNull(values: readonly number[]): number | null {
  return values.length === 0 ? null : quantile(values, 0.5);
}

function maxOrNull(values: readonly number[]): number | null {
  return values.length === 0 ? null : Math.max(...values);
}

/**
 * Formats a raw figure with its verified, false-success and `needs-review`
 * counts beside it, enforcing ADR 0025's rule that a raw figure is never
 * emitted alone.
 */
export function formatRawFigure(view: RawView): string {
  const rate =
    view.raw_tasks_per_second === null ? "n/a" : `${view.raw_tasks_per_second.toFixed(2)} task/s`;
  const p50 = view.raw_p50_ms === null ? "n/a" : `${view.raw_p50_ms.toFixed(1)} ms`;
  return (
    `raw=${rate} (p50=${p50}) | ` +
    `verified=${view.counts.verified}/${view.counts.total_attempts}, ` +
    `false_success=${view.counts.false_success}, ` +
    `needs_review=${view.counts.needs_review}`
  );
}

/** Computes ADR 0025's three views, counts, cost breakdown and separate human-time column. */
export function buildThreeViews(input: BuildThreeViewsInput): ThreeViewReport {
  const measured = input.attempts.filter((one) => one.phase === "measure");
  const counts = countResults(measured);

  const allDurations = measured
    .map((one) => one.timing.task_ms)
    .filter((value): value is number => value !== null);
  const verifiedDurations = measured
    .filter((one) => one.status === "verified")
    .map((one) => one.timing.task_ms)
    .filter((value): value is number => value !== null);

  const summedWallMs = allDurations.reduce((sum, ms) => sum + ms, 0);
  const windowMs =
    input.windowMs !== undefined && input.windowMs !== null
      ? input.windowMs
      : summedWallMs > 0
        ? summedWallMs
        : null;

  const raw: RawView = {
    raw_tasks_per_second:
      windowMs === null || windowMs <= 0 ? null : (measured.length * 1000) / windowMs,
    raw_p50_ms: quantile(allDurations, 0.5),
    raw_p95_ms: quantile(allDurations, 0.95),
    counts,
  };

  const matchingCells = (input.cells ?? []).filter(
    (cell) => cell.adapter === input.adapter && cell.task === input.task,
  );
  const cpuValues = matchingCells
    .map((cell) => cell.metrics.cpu_ms_per_verified_task.value)
    .filter((value): value is number => value !== null);
  const pssValues = matchingCells
    .map((cell) => cell.metrics.pss_peak_bytes.value)
    .filter((value): value is number => value !== null);
  const rssValues = matchingCells
    .map((cell) => cell.metrics.rss_peak_bytes.value)
    .filter((value): value is number => value !== null);

  const per_verified: PerVerifiedView = {
    verified_tasks_per_second:
      windowMs === null || windowMs <= 0 ? null : (counts.verified * 1000) / windowMs,
    task_p50_ms_verified: quantile(verifiedDurations, 0.5),
    task_p95_ms_verified: quantile(verifiedDurations, 0.95),
    cpu_ms_per_verified_task: medianOrNull(cpuValues),
    pss_peak_bytes: maxOrNull(pssValues),
    rss_peak_bytes: maxOrNull(rssValues),
    counts,
  };

  // Human time is tracked separately and NEVER folded into dollars.
  const totalHumanSeconds = measured.reduce(
    (sum, one) => sum + (one.adapter_metrics?.cost_usage?.human_seconds ?? 0),
    0,
  );
  const human_time: HumanTimeColumn = {
    total_seconds: totalHumanSeconds,
    seconds_per_verified: counts.verified > 0 ? totalHumanSeconds / counts.verified : null,
  };

  // Cost view: requires a machine with a public hourly price and >= 1 verified result.
  const hostPrice =
    input.hostMachineId === null ? undefined : input.prices.hosts[input.hostMachineId];
  let cost: CostView;
  if (input.hostMachineId === null || hostPrice === undefined) {
    cost = {
      available: false,
      reason:
        input.hostMachineId === null
          ? "no public hourly price for this machine"
          : `machine ${input.hostMachineId} has no public hourly price in price table v${input.prices.version}`,
      counts,
    };
  } else if (counts.verified === 0) {
    cost = {
      available: false,
      reason: "zero verified results; dollars per 1,000 verified results is undefined",
      counts,
    };
  } else {
    const wallMsForCost = windowMs ?? 0;
    const hostUsd = (wallMsForCost / MS_PER_HOUR) * hostPrice.hourly_usd;
    let modelUsd = 0;
    let proxyUsd = 0;
    let captchaUsd = 0;
    let hostedBrowserUsd = 0;
    let failedAttemptUsd = 0;

    for (const one of measured) {
      const spend = attemptDirectSpendBreakdown(one, input.prices);
      const attemptHost =
        one.timing.task_ms !== null && summedWallMs > 0
          ? (one.timing.task_ms / summedWallMs) * hostUsd
          : 0;
      if (one.status !== "verified") {
        failedAttemptUsd += spend.total_usd + attemptHost;
      }
      modelUsd += spend.model_usd;
      proxyUsd += spend.proxy_usd;
      captchaUsd += spend.captcha_usd;
      hostedBrowserUsd += spend.hosted_browser_usd;
    }

    const totalUsd = hostUsd + modelUsd + proxyUsd + captchaUsd + hostedBrowserUsd;
    const dollarsPer1k = (totalUsd / counts.verified) * 1_000;

    let planAmortization: readonly PlanAmortizationRow[] | null = null;
    const plan = input.planOneTimeCost;
    if (plan !== undefined) {
      const runCounts = resolveAmortizationCounts(plan.declaredRunCounts);
      const verifiedPerRun = plan.verifiedPerRun ?? counts.verified;
      if (verifiedPerRun <= 0) {
        throw new BenchError("invalid_document", "verifiedPerRun must be positive");
      }
      planAmortization = runCounts.map((runs) => {
        const amortizedPerRunUsd = plan.oneTimeUsd / runs;
        const amortizedPer1k = (amortizedPerRunUsd / verifiedPerRun) * 1_000;
        return {
          runs,
          one_time_usd: plan.oneTimeUsd,
          amortized_per_run_usd: amortizedPerRunUsd,
          total_dollars_per_1k_verified: dollarsPer1k + amortizedPer1k,
        };
      });
    }

    cost = {
      available: true,
      dollars_per_1k_verified: dollarsPer1k,
      breakdown: {
        host_usd: hostUsd,
        model_usd: modelUsd,
        proxy_usd: proxyUsd,
        captcha_usd: captchaUsd,
        hosted_browser_usd: hostedBrowserUsd,
        total_usd: totalUsd,
        failed_attempt_usd: failedAttemptUsd,
      },
      plan_amortization: planAmortization,
      price_table: {
        version: input.prices.version,
        effective_date: input.prices.effective_date,
        source: input.prices.source,
      },
      host_pricing: {
        machine_id: input.hostMachineId,
        hourly_usd: hostPrice.hourly_usd,
        source: hostPrice.source,
        date: hostPrice.date,
      },
      counts,
    };
  }

  return {
    adapter: input.adapter,
    task: input.task,
    counts,
    raw,
    per_verified,
    cost,
    human_time,
  };
}
