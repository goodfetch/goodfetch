// Integration test (spec #152, issue 175): runs a real `bench run` against
// the commit's Proving ground and verifies that `run.json` emits ADR 0025's
// three views (raw paired with verified/false-success/needs-review counts,
// per-verified-result, dollars per 1,000 verified results), Plan amortization
// over pre-declared run counts, and the separate human-time column.
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { loadPriceTable } from "../src/prices.ts";
import { runCampaign } from "../src/runner.ts";
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

    test("run.json emits three views, needs-review counts, Plan amortization and human time from Proving ground", async ({
      provingGround,
    }) => {
      // Standard bench CLI run emits views, raw summaries, and needs_review counts directly in run.json.
      const ran = await bench("decisions.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record, attempts } = await written(ran.out);
      expect(record.totals.needs_review).toBe(0);
      expect(record.views.length).toBeGreaterThan(0);
      expect(record.raw_summaries.length).toBe(record.views.length);

      const fastView = record.views.find((v) => v.adapter === "fast");
      if (fastView === undefined) {
        throw new Error("expected fast view in run.json");
      }
      expect(fastView.raw.counts.verified).toBe(15);
      expect(fastView.raw.counts.false_success).toBe(0);
      expect(fastView.raw.counts.needs_review).toBe(0);
      expect(fastView.per_verified.verified_tasks_per_second).toBeGreaterThan(0);
      expect(fastView.human_time).toEqual({ total_seconds: 0, seconds_per_verified: 0 });
      // Default local run has no public hourly host price -> cost view unavailable.
      expect(fastView.cost.available).toBe(false);

      const fastSummary = record.raw_summaries.find((s) => s.adapter === "fast")?.summary;
      expect(fastSummary).toContain("verified=15/15, false_success=0, needs_review=0");

      // Campaign with pre-declared Plan amortization and public host machine emits priced cost view.
      const dir = await mkdtemp(join(tmpdir(), "goodfetch-views-"));
      try {
        const stubPath = fileURLToPath(new URL("./stub-adapter.ts", import.meta.url));
        const campaignPath = join(dir, "views-campaign.json");
        await writeFile(
          campaignPath,
          JSON.stringify({
            id: "views-priced",
            host_class: { requested: "local-dev", allow_downgrade: false },
            order_seed: 175,
            blocks: 1,
            warmup: 0,
            iterations: 3,
            concurrency: 1,
            host_machine_id: "reference-bare-metal",
            plan_amortization_runs: [...PLAN_AMORTIZATION_RUN_COUNTS],
            plan_one_time_usd: 0.5,
            timeout_ms: 10000,
            max_duration_ms: 60000,
            max_restarts: 1,
            adapters: [{ id: "stub", command: ["node", stubPath, "--delay-ms", "10"] }],
            tasks: ["catalog/2"],
          }),
        );
        const priced = await runCampaign({
          campaignPath,
          provingGround: provingGround.address,
          out: dir,
        });
        const view = priced.record.views[0];
        if (view === undefined || !view.cost.available) {
          throw new Error("expected available cost view in priced run.json");
        }
        expect(view.cost.price_table.version).toBe(1);
        expect(view.cost.dollars_per_1k_verified).toBeGreaterThan(0);
        expect(view.cost.plan_amortization?.map((r) => r.runs)).toEqual([1, 10, 100, 1000]);
        expect(formatRawFigure(view.raw)).toContain(
          "verified=3/3, false_success=0, needs_review=0",
        );
      } finally {
        await rm(dir, { recursive: true, force: true });
      }

      // Direct buildThreeViews cross-check against loaded price table.
      const prices = loadPriceTable();
      const fastAttempts = attempts.filter((one) => one.adapter === "fast");
      const unpriced = buildThreeViews({
        adapter: "fast",
        task: "catalog/2",
        attempts: fastAttempts,
        hostMachineId: null,
        prices,
      });
      expect(unpriced.cost.available).toBe(false);
    });
  },
);
