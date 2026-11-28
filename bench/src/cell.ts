// A cell is one adapter on one Proving ground task in one block: its
// attempts, as `attempts.jsonl` writes them, and the metrics the run record
// gives it. Every metric is a value with its source, or `null` with the reason
// it has none: a cell that failed its gate is not measured, and a probe the
// host lacks reads nothing.
import type { AttemptStatus, BenchErrorCode } from "./errors.ts";
import type { Measure, MetricBook, MetricId, Reading } from "./metrics.ts";
import { quantile } from "./order.ts";
import type { Judgement, ToolStatus } from "./proving-ground.ts";
import type { TreeUsage } from "./resources.ts";

/** A cell's phase: its gate, its warmup and its measured attempts. */
export type Phase = "conformance" | "warmup" | "measure";

/** Per-attempt billable usage reported by an adapter for the cost model (ADR 0025). */
export interface AttemptCostUsage {
  readonly model?:
    | {
        readonly id: string;
        readonly input_tokens: number;
        readonly output_tokens: number;
      }
    | undefined;
  readonly proxy?:
    | {
        readonly tier: string;
        readonly bytes?: number | undefined;
        readonly requests?: number | undefined;
      }
    | undefined;
  readonly captcha?:
    | {
        readonly kind: string;
        readonly solves: number;
      }
    | undefined;
  readonly hosted_browser?:
    | {
        readonly vendor: string;
        readonly minutes: number;
      }
    | undefined;
  /** Human intervention time in seconds; reported in its own column, never folded into dollars. */
  readonly human_seconds?: number | undefined;
}

/** What an adapter reports of its own work on one task. */
export interface AdapterMetrics {
  readonly cdp_commands?: number | undefined;
  readonly cost_usage?: AttemptCostUsage | undefined;
}

/** One attempt, as `attempts.jsonl` writes it. */
export interface Attempt {
  readonly attempt_id: string;
  readonly cell_id: string;
  readonly adapter: string;
  readonly task: string;
  readonly block: number;
  readonly phase: Phase;
  /** Its place in the cell: the conformance attempt is 0, the warmup follows, then the measured attempts. */
  readonly index: number;
  readonly status: AttemptStatus;
  /** What the tool itself reported (`success`, `needs-review`, `failure`), or `null` if it did not answer. */
  readonly tool_status?: ToolStatus | null | undefined;
  readonly error_code: BenchErrorCode | null;
  readonly reason: string | null;
  readonly timing: {
    /** Waiting for the adapter to be ready, a restart included. */
    readonly queue_ms: number | null;
    /** The `run` call only. */
    readonly task_ms: number | null;
    /** The oracle's own time, never part of the task's. */
    readonly oracle_ms: number | null;
  };
  /** The adapter process's generation, from 0. */
  readonly generation: number;
  readonly judgement: Judgement | null;
  readonly adapter_metrics: AdapterMetrics | null;
}

/** How many attempts ended with each status, plus `needs_review` count (ADR 0025). */
export type Counts = Readonly<Record<AttemptStatus, number>> & {
  readonly needs_review: number;
};

/** One cell as the run record writes it, with its metrics. */
export interface Cell {
  readonly cell_id: string;
  readonly adapter: string;
  readonly task: string;
  readonly block: number;
  readonly gate: "passed" | "failed";
  readonly counts: Counts;
  readonly metrics: Readonly<Record<MetricId, Measure>>;
}

/** A cell's attempts, before its metrics. */
export interface CellRun {
  readonly cellId: string;
  readonly adapter: string;
  readonly task: string;
  readonly block: number;
  /** Whether its conformance attempt and warmup were all verified. */
  readonly passed: boolean;
  readonly attempts: readonly Attempt[];
  /** The measured window's length, or `null` when the cell was not measured. */
  readonly windowMs: number | null;
}

/** The statuses a failure rate counts against the verified attempts. */
const failures: ReadonlySet<AttemptStatus> = new Set([
  "failed",
  "false_success",
  "incomplete",
  "timeout",
  "crashed",
]);

function zero(): Record<AttemptStatus, number> & { needs_review: number } {
  return {
    verified: 0,
    incomplete: 0,
    false_success: 0,
    failed: 0,
    timeout: 0,
    crashed: 0,
    aborted: 0,
    unsupported: 0,
    rejected: 0,
    skipped_gate: 0,
    needs_review: 0,
  };
}

/** How many of `attempts` ended with each status (and how many reported `needs-review`). */
export function countStatuses(
  attempts: readonly {
    readonly status: AttemptStatus;
    readonly tool_status?: ToolStatus | null | undefined;
  }[],
): Counts {
  const counts = zero();
  for (const one of attempts) {
    counts[one.status] += 1;
    if (one.tool_status === "needs-review") {
      counts.needs_review += 1;
    }
  }
  return counts;
}

function reading(value: number | null, source: string, reason: string): Reading {
  return value === null ? { value: null, reason } : { value, source };
}

function known(values: readonly (number | null)[]): number[] {
  return values.filter((value): value is number => value !== null);
}

/** The usage of every generation of one adapter: CPU summed, peaks maxed, `null` when none was read. */
export function mergeUsage(usages: readonly TreeUsage[]): TreeUsage {
  const cpu = known(usages.map((usage) => usage.cpuMs));
  const pss = known(usages.map((usage) => usage.peakPssBytes));
  const rss = known(usages.map((usage) => usage.peakRssBytes));
  return {
    cpuMs: cpu.length === 0 ? null : cpu.reduce((sum, value) => sum + value, 0),
    peakPssBytes: pss.length === 0 ? null : Math.max(...pss),
    peakRssBytes: rss.length === 0 ? null : Math.max(...rss),
  };
}

/** A cell's metrics, each a value with its source or `null` with its reason. */
export function summarize(input: {
  readonly book: MetricBook;
  readonly run: CellRun;
  /** The adapter's usage over its block, every cell and phase. */
  readonly usage: TreeUsage;
  /** The adapter's verified attempts over its block, every cell and phase. */
  readonly verifiedInBlock: number;
  readonly startupMs: readonly number[];
}): Cell {
  const { book, run, usage, verifiedInBlock } = input;
  const measured = run.attempts.filter((one) => one.phase === "measure");
  const verified = measured.filter((one) => one.status === "verified");
  const durations = known(verified.map((one) => one.timing.task_ms));
  const cdp = verified
    .map((one) => one.adapter_metrics?.cdp_commands)
    .filter((value): value is number => value !== undefined);
  const counted = measured.filter((one) => failures.has(one.status)).length + verified.length;
  const gated = (read: () => Reading): Reading =>
    run.passed ? read() : { value: null, reason: "the cell failed its conformance gate" };
  const timed = "harness; the run call only, the oracle's time excluded";
  const tree = "the adapter's process tree over every phase of its block";
  const noSample = "no sample of the adapter's process tree";

  const metrics: Record<MetricId, Measure> = {
    verified_tasks_per_second: book.measure(
      "verified_tasks_per_second",
      gated(() =>
        reading(
          run.windowMs === null || run.windowMs === 0
            ? null
            : (verified.length * 1000) / run.windowMs,
          "harness; verified measured attempts over the measured window",
          "no measured window",
        ),
      ),
    ),
    task_p50_ms_verified: book.measure(
      "task_p50_ms_verified",
      gated(() => reading(quantile(durations, 0.5), timed, "no verified measured attempt")),
    ),
    task_p95_ms_verified: book.measure(
      "task_p95_ms_verified",
      gated(() =>
        reading(
          quantile(durations, 0.95),
          `${timed}, nearest rank`,
          "no verified measured attempt",
        ),
      ),
    ),
    startup_ms: book.measure(
      "startup_ms",
      reading(
        quantile(input.startupMs, 0.5),
        "harness; the adapter's start to its answer to prepare",
        "the adapter never started",
      ),
    ),
    cpu_ms_per_verified_task: book.measure(
      "cpu_ms_per_verified_task",
      usage.cpuMs === null
        ? { value: null, reason: noSample }
        : reading(
            verifiedInBlock === 0 ? null : usage.cpuMs / verifiedInBlock,
            `proc_stat; ${tree}`,
            "no verified attempt in the adapter's block",
          ),
    ),
    pss_peak_bytes: book.measure(
      "pss_peak_bytes",
      reading(usage.peakPssBytes, `smaps_rollup; ${tree}, Pss_Anon plus SwapPss`, noSample),
    ),
    rss_peak_bytes: book.measure(
      "rss_peak_bytes",
      reading(usage.peakRssBytes, `ps; ${tree}, shared pages counted per process`, noSample),
    ),
    cdp_commands_per_task: book.measure(
      "cdp_commands_per_task",
      gated(() =>
        reading(
          quantile(cdp, 0.5),
          "adapter; median over the verified measured attempts",
          "the adapter reports no CDP commands",
        ),
      ),
    ),
    failure_rate: book.measure(
      "failure_rate",
      gated(() =>
        reading(
          counted === 0 ? null : (counted - verified.length) / counted,
          "harness; measured attempts, aborted siblings, rejected and unsupported ones left out",
          "no measured attempt counts toward it",
        ),
      ),
    ),
  };
  return {
    cell_id: run.cellId,
    adapter: run.adapter,
    task: run.task,
    block: run.block,
    gate: run.passed ? "passed" : "failed",
    counts: countStatuses(run.attempts),
    metrics,
  };
}
