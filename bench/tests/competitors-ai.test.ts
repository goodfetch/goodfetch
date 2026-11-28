// Integration test (spec #152, issue 183): runs the smoke campaign covering
// Crawl4AI, Browser Use, ScrapeGraphAI, Skyvern and changedetection.io with
// the fake model in CI against the Proving ground.
import { readFileSync } from "node:fs";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { bench, written } from "./bench.ts";

const aiAdapters = [
  "crawl4ai",
  "browser-use",
  "scrapegraphai",
  "skyvern",
  "changedetection-io",
] as const;

describe(
  "AI and change-tracking competitor adapters",
  { tags: ["story:2-23", "story:2-27", "story:2-76"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("runs all 5 AI/change-tracking adapters with fake model in CI against Proving ground", async ({
      provingGround,
    }) => {
      for (const name of aiAdapters) {
        const cfg: Record<string, unknown> = JSON.parse(
          readFileSync(new URL(`../adapters/${name}/configuration.json`, import.meta.url), "utf8"),
        );
        expect(cfg["pinned_version"]).toBeTruthy();
        expect(cfg["fake_model_ci"]).toBe(true);
        expect(cfg["defaults"]).toEqual({
          filter_list_blocking: false,
          consent_prompt_handling: false,
        });
      }

      const ran = await bench("competitors-ai-smoke.json", provingGround.address);
      expect(ran.stderr).toBe("");
      expect(ran.code).toBe(0);

      const { record } = await written(ran.out);
      expect(record.cells).toHaveLength(aiAdapters.length);
      for (const cell of record.cells) {
        expect(cell.gate).toBe("passed");
        expect(cell.counts.verified).toBeGreaterThanOrEqual(2);
        expect(cell.counts.false_success).toBe(0);
      }
    });
  },
);
