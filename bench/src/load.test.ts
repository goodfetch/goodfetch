import { describe, expect, test } from "vitest";
import {
  parseGovernorCeilings,
  runConcurrencySearch,
  STANDARD_CONCURRENCY_LEVELS,
  toGovernorCeilings,
  type LoadLevelMeasurement,
} from "./load.ts";

function scripted(
  curve: Readonly<Record<number, { tps: number; falseSuccess?: number }>>,
): (concurrency: number) => Promise<LoadLevelMeasurement> {
  return async (concurrency) => {
    const point = curve[concurrency];
    if (point === undefined) {
      throw new Error(`unexpected concurrency level ${concurrency}`);
    }
    return {
      concurrency,
      verified_tasks_per_second: point.tps,
      verified: 100,
      false_success: point.falseSuccess ?? 0,
      failed: 0,
      total_attempts: 100 + (point.falseSuccess ?? 0),
    };
  };
}

describe("concurrency search and governor ceilings", () => {
  test("stops at the first level where verified throughput plateaus and records previous level", async () => {
    const visited: number[] = [];
    const result = await runConcurrencySearch({
      levels: STANDARD_CONCURRENCY_LEVELS,
      measure: async (c) => {
        visited.push(c);
        return scripted({
          1: { tps: 20 },
          8: { tps: 140 },
          64: { tps: 135 },
          256: { tps: 90 },
        })(c);
      },
    });

    expect(visited).toEqual([1, 8, 64]);
    expect(result.highest_safe_concurrency).toBe(8);
    expect(result.peak_verified_tasks_per_second).toBe(140);
    expect(result.stop_reason).toBe("throughput_plateau");
    expect(result.stopped_at_concurrency).toBe(64);
  });

  test("stops immediately when a false success appears, even if throughput rose", async () => {
    const visited: number[] = [];
    const result = await runConcurrencySearch({
      levels: STANDARD_CONCURRENCY_LEVELS,
      measure: async (c) => {
        visited.push(c);
        return scripted({
          1: { tps: 25 },
          8: { tps: 180 },
          64: { tps: 500, falseSuccess: 1 },
          256: { tps: 900 },
        })(c);
      },
    });

    expect(visited).toEqual([1, 8, 64]);
    expect(result.highest_safe_concurrency).toBe(8);
    expect(result.stop_reason).toBe("false_success");
    expect(result.stopped_at_concurrency).toBe(64);
  });

  test("publishes a validated GovernorCeilings document for the Engine runtime", async () => {
    const search = await runConcurrencySearch({
      levels: STANDARD_CONCURRENCY_LEVELS,
      measure: scripted({
        1: { tps: 10 },
        8: { tps: 75 },
        64: { tps: 420 },
        256: { tps: 810 },
      }),
    });

    expect(search.highest_safe_concurrency).toBe(256);
    expect(search.stop_reason).toBe("exhausted_levels");

    const ceilings = toGovernorCeilings({
      product: "foxhound",
      family: "http-crawl",
      search,
      hostClass: "local-controlled",
      measuredAt: "2026-11-28T00:00:00.000Z",
      provenanceCommit: "133ec258a",
    });

    const reparsed = parseGovernorCeilings(JSON.parse(JSON.stringify(ceilings)));
    expect(reparsed.default_ceiling).toBe(256);
    expect(reparsed.product).toBe("foxhound");
    expect(reparsed.levels).toHaveLength(4);
  });
});
