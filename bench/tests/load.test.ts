// Integration test (spec #152, issue 178): runs a real Proving ground campaign
// and drives the concurrency search over [1, 8, 64, 256] against a scripted
// curve anchored on the real run's verified throughput, then verifies the
// published GovernorCeilings artifact read by the Engine runtime (ADR 0040).
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import {
  parseGovernorCeilings,
  runConcurrencySearch,
  STANDARD_CONCURRENCY_LEVELS,
  toGovernorCeilings,
} from "../src/load.ts";
import { bench, written } from "./bench.ts";

describe("load runs and concurrency search", { tags: ["story:2-17", "story:2-70"] }, () => {
  test.override("provingGroundSeed", 152);

  test("runs concurrency search over declared levels and publishes GovernorCeilings", async ({
    provingGround,
  }) => {
    const ran = await bench("stubs.json", provingGround.address);
    expect(ran.stderr).toBe("");
    const { record } = await written(ran.out);

    const baselineCell = record.cells.find((c) => c.gate === "passed");
    const baseTps = baselineCell?.metrics.verified_tasks_per_second.value ?? 10;

    // Scripted curve: scales up cleanly at 1 -> 8 -> 64, then saturates at 256.
    const multipliers: Readonly<Record<number, number>> = {
      1: 1,
      8: 6,
      64: 24,
      256: 22,
    };

    const search = await runConcurrencySearch({
      levels: STANDARD_CONCURRENCY_LEVELS,
      measure: async (concurrency) => ({
        concurrency,
        verified_tasks_per_second: baseTps * (multipliers[concurrency] ?? 1),
        verified: baselineCell?.counts.verified ?? 5,
        false_success: 0,
        failed: 0,
        total_attempts: baselineCell?.counts.verified ?? 5,
      }),
    });

    expect(search.highest_safe_concurrency).toBe(64);
    expect(search.stop_reason).toBe("throughput_plateau");
    expect(search.stopped_at_concurrency).toBe(256);

    const ceilings = toGovernorCeilings({
      product: "foxhound",
      family: "http-crawl",
      search,
      hostClass: record.host.class_granted,
      measuredAt: record.finished_at,
      provenanceCommit: record.provenance.git?.sha ?? "unknown",
    });

    const parsed = parseGovernorCeilings(JSON.parse(JSON.stringify(ceilings)));
    expect(parsed.schema_version).toBe(1);
    expect(parsed.default_ceiling).toBe(64);
    expect(parsed.levels.map((l) => l.concurrency)).toEqual([1, 8, 64, 256]);
  });
});
