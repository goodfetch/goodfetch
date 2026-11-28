// Integration test (spec #152, issue 175): runs a real `bench run` against
// the commit's Proving ground, then derives ADR 0025's three views (raw with
// verified/false-success/needs-review counts, per-verified-result, and dollars
// per 1,000 verified results) plus Plan amortization and separate human time.
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { loadPriceTable } from "../src/prices.ts";
import { buildThreeViews, formatRawFigure, PLAN_AMORTIZATION_RUN_COUNTS } from "../src/views.ts";
import { bench, written } from "./bench.ts";

describe(
  "three views, counts and cost model",
  {
    tags: [
      "story:2-1",
      "story:2-2",
      "story:2-6",
      "story:2-14",
      "story:2-15",
      "story:2-16",
      "story:2-72",
    ],
  },
  () => {
    test.override("provingGroundSeed", 152);

    test("computes all three views, counts, cost model and Plan amortization from a Proving ground run", async ({
      provingGround,
    }) => {
      const ran = await bench("decisions.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record, attempts } = await written(ran.out);
      const prices = loadPriceTable();

      const fastAttempts = attempts.filter((one) => one.adapter === "fast");
      const fastCells = record.cells.filter((cell) => cell.adapter === "fast");
      const report = buildThreeViews({
        adapter: "fast",
        task: "catalog/2",
        attempts: fastAttempts,
        cells: fastCells,
        hostMachineId: "reference-bare-metal",
        prices,
        planOneTimeCost: {
          oneTimeUsd: 0.5,
          declaredRunCounts: [...PLAN_AMORTIZATION_RUN_COUNTS],
          verifiedPerRun: 15,
        },
      });

      // Raw view always carries verified, false-success and needs-review counts.
      expect(report.raw.counts.verified).toBe(15);
      expect(report.raw.counts.false_success).toBe(0);
      expect(report.raw.counts.needs_review).toBe(0);
      expect(formatRawFigure(report.raw)).toContain(
        "verified=15/15, false_success=0, needs_review=0",
      );

      // Per-verified view carries verified throughput and latency.
      expect(report.per_verified.verified_tasks_per_second).toBeGreaterThan(0);
      expect(report.per_verified.task_p50_ms_verified).toBeGreaterThan(0);

      // Cost view sums host price × wall time with versioned price table provenance and Plan amortization.
      if (!report.cost.available) {
        throw new Error(`expected available cost view, got: ${report.cost.reason}`);
      }
      expect(report.cost.price_table.version).toBe(1);
      expect(report.cost.dollars_per_1k_verified).toBeGreaterThan(0);
      expect(report.cost.plan_amortization?.map((r) => r.runs)).toEqual([1, 10, 100, 1000]);

      // Human time is reported separately and never folded into dollars.
      expect(report.human_time).toEqual({ total_seconds: 0, seconds_per_verified: 0 });

      // Machine with no public hourly price emits no cost view.
      const unpriced = buildThreeViews({
        adapter: "fast",
        task: "catalog/2",
        attempts: fastAttempts,
        cells: fastCells,
        hostMachineId: null,
        prices,
      });
      expect(unpriced.cost.available).toBe(false);
    });
  },
);
