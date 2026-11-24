import { describe, expect, test } from "vitest";
import { summarize, type Attempt, type CellRun, type Phase } from "./cell.ts";
import type { AttemptStatus } from "./errors.ts";
import { metricBook, type Probe } from "./metrics.ts";

const book = metricBook({
  platform: "linux",
  probes: new Set<Probe>(["harness", "adapter", "ps", "proc_stat"]),
});

function attemptOf(phase: Phase, index: number, status: AttemptStatus, taskMs: number): Attempt {
  return {
    attempt_id: `a:t:b0:${phase}:${index}`,
    cell_id: "a:t:b0",
    adapter: "a",
    task: "t",
    block: 0,
    phase,
    index,
    status,
    error_code: null,
    reason: null,
    timing: { queue_ms: 0, task_ms: taskMs, oracle_ms: null },
    generation: 0,
    judgement: null,
    adapter_metrics: status === "verified" ? { cdp_commands: 10 * index } : null,
  };
}

function cellRun(passed: boolean, statuses: readonly AttemptStatus[]): CellRun {
  return {
    cellId: "a:t:b0",
    adapter: "a",
    task: "t",
    block: 0,
    passed,
    attempts: [
      attemptOf("conformance", 0, passed ? "verified" : "false_success", 5),
      ...statuses.map((status, offset) =>
        attemptOf("measure", offset + 1, status, 10 * (offset + 1)),
      ),
    ],
    windowMs: passed ? 2000 : null,
  };
}

const usage = { cpuMs: 300, peakPssBytes: null, peakRssBytes: 4096 };

describe("summarize", () => {
  test("counts failures against the verified attempts, leaving aborted siblings out", () => {
    const run = cellRun(true, [
      "timeout",
      "aborted",
      "aborted",
      "aborted",
      "verified",
      "verified",
      "verified",
      "verified",
    ]);
    const cell = summarize({ book, run, usage, verifiedInBlock: 5, startupMs: [30, 10, 20] });

    expect({
      failureRate: cell.metrics.failure_rate.value,
      throughput: cell.metrics.verified_tasks_per_second.value,
      p50: cell.metrics.task_p50_ms_verified.value,
      p95: cell.metrics.task_p95_ms_verified.value,
      cdp: cell.metrics.cdp_commands_per_task.value,
      startup: cell.metrics.startup_ms.value,
      cpu: cell.metrics.cpu_ms_per_verified_task.value,
      counts: cell.counts,
    }).toEqual({
      failureRate: 0.2,
      throughput: 2,
      p50: 60,
      p95: 80,
      cdp: 60,
      startup: 20,
      cpu: 60,
      counts: expect.objectContaining({ verified: 5, timeout: 1, aborted: 3, false_success: 0 }),
    });
  });

  test("gives a cell that failed its gate no measured figure, and an unprobed metric its reason", () => {
    const cell = summarize({
      book,
      run: cellRun(false, ["skipped_gate", "skipped_gate"]),
      usage,
      verifiedInBlock: 0,
      startupMs: [12],
    });
    const gate = "the cell failed its conformance gate";

    expect(cell.metrics).toEqual({
      verified_tasks_per_second: { value: null, unit: "task/s", reason: gate },
      task_p50_ms_verified: { value: null, unit: "ms", reason: gate },
      task_p95_ms_verified: { value: null, unit: "ms", reason: gate },
      startup_ms: { value: 12, unit: "ms", source: expect.any(String) },
      cpu_ms_per_verified_task: {
        value: null,
        unit: "ms/task",
        reason: "no verified attempt in the adapter's block",
      },
      pss_peak_bytes: {
        value: null,
        unit: "B",
        reason: "probe smaps_rollup is not available on this host",
      },
      rss_peak_bytes: { value: 4096, unit: "B", source: expect.any(String) },
      cdp_commands_per_task: { value: null, unit: "command/task", reason: gate },
      failure_rate: { value: null, unit: "ratio", reason: gate },
    });
  });
});
