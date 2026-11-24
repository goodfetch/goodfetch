// A metric a run cannot measure is `null` with its reason, never 0: no probe
// on the platform, a probe the host lacks, or no reading to give.
import { describe, expect, test } from "vitest";
import { metricBook } from "./metrics.ts";

const everyProbe = new Set(["harness", "adapter", "ps", "proc_stat", "smaps_rollup"] as const);

describe("metricBook", () => {
  test("gives a metric's reading with its unit and source when it is available", () => {
    const book = metricBook({ platform: "linux", probes: everyProbe });
    expect(book.measure("pss_peak_bytes", { value: 4096, source: "adapter process tree" })).toEqual(
      {
        value: 4096,
        unit: "B",
        source: "adapter process tree",
      },
    );
  });

  test("makes a metric with no probe on the platform null, with that reason", () => {
    const book = metricBook({ platform: "darwin", probes: everyProbe });
    expect(book.availability.pss_peak_bytes).toEqual({
      available: false,
      reason: "no probe on darwin",
    });
    expect(book.measure("pss_peak_bytes", { value: 4096, source: "adapter process tree" })).toEqual(
      {
        value: null,
        unit: "B",
        reason: "no probe on darwin",
      },
    );
  });

  test("makes a metric whose probe the host lacks null, naming the probe", () => {
    const book = metricBook({ platform: "linux", probes: new Set(["harness", "adapter", "ps"]) });
    expect(book.measure("cpu_ms_per_verified_task", { value: 12, source: "process tree" })).toEqual(
      {
        value: null,
        unit: "ms/task",
        reason: "probe proc_stat is not available on this host",
      },
    );
  });

  test("keeps the reason of an available metric with no reading, never 0", () => {
    const book = metricBook({ platform: "linux", probes: everyProbe });
    expect(
      book.measure("task_p50_ms_verified", { value: null, reason: "no verified task" }),
    ).toEqual({ value: null, unit: "ms", reason: "no verified task" });
    expect(book.measure("task_p50_ms_verified", { value: Number.NaN, source: "run" })).toEqual({
      value: null,
      unit: "ms",
      reason: "not a finite number",
    });
  });
});
