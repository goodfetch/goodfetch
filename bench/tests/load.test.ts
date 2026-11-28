// Integration test (spec #152, issue 178): runs a real multi-concurrency
// campaign (`concurrency_levels: [1, 2, 4]`) against the Proving ground using
// `stub-adapter.ts`, verifying that `runner.ts` executes real multi-concurrency
// sweeps, stops at the scripted false-success cutoff, and writes a validated
// `governor-ceilings.json` artifact aligned with Engine runtime's 3 WorkKinds.
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { parseGovernorCeilings } from "../src/load.ts";
import { runCampaign } from "../src/runner.ts";

describe("load runs and concurrency search", { tags: ["story:2-17", "story:2-70"] }, () => {
  test.override("provingGroundSeed", 152);

  test("executes real multi-concurrency stub-adapter sweep and writes governor-ceilings.json", async ({
    provingGround,
  }) => {
    const dir = await mkdtemp(join(tmpdir(), "goodfetch-load-"));
    try {
      const stubPath = fileURLToPath(new URL("./stub-adapter.ts", import.meta.url));
      const campaignPath = join(dir, "load-sweep.json");
      await writeFile(
        campaignPath,
        JSON.stringify({
          id: "http-crawl",
          host_class: { requested: "local-dev", allow_downgrade: false },
          order_seed: 178,
          blocks: 1,
          warmup: 0,
          iterations: 6,
          concurrency: 1,
          concurrency_levels: [1, 2, 4],
          timeout_ms: 10000,
          max_duration_ms: 60000,
          max_restarts: 1,
          adapters: [
            {
              id: "scripted-load",
              command: [
                "node",
                stubPath,
                "--delay-ms",
                "40",
                "--false-success-at-concurrency",
                "4",
              ],
            },
          ],
          tasks: ["catalog/2"],
        }),
      );

      const { directory, record } = await runCampaign({
        campaignPath,
        provingGround: provingGround.address,
        out: dir,
      });

      const search = record.concurrency_search;
      if (search === null) {
        throw new Error("expected concurrency_search in run.json");
      }
      expect(search.levels.map((l) => l.concurrency)).toEqual([1, 2, 4]);
      expect(search.levels[0]?.false_success).toBe(0);
      expect(search.levels[1]?.false_success).toBe(0);
      expect(search.levels[1]?.verified_tasks_per_second).toBeGreaterThan(
        search.levels[0]?.verified_tasks_per_second ?? 0,
      );
      expect(search.levels[2]?.false_success).toBeGreaterThan(0);
      expect(search.stop_reason).toBe("false_success");
      expect(search.stopped_at_concurrency).toBe(4);
      expect(search.highest_safe_concurrency).toBe(2);

      const ceilingsFile: unknown = JSON.parse(
        await readFile(join(directory, "governor-ceilings.json"), "utf8"),
      );
      const parsed = parseGovernorCeilings(ceilingsFile);
      expect(parsed.schema_version).toBe(1);
      expect(parsed.work_kind).toBe("http");
      expect(parsed.setting).toBe("governor.http.ceiling");
      expect(parsed.default_ceiling).toBe(2);
      expect(parsed.ceilings).toEqual({ http: 2, browser_page: null, model_call: null });
      expect(record.governor_ceilings).toEqual(parsed);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
