// Integration test (spec #152, issues 180 and 181): runs the smoke campaign
// covering Crawlee JS/Py, Scrapy, Scrapling, Firecrawl self-hosted, Spider
// and Colly against the live Proving ground (`catalog/1` crawl and `catalog/2`
// extract), verifies oracle confirmation and published configurations.
import { readFileSync } from "node:fs";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { bench, written } from "./bench.ts";

const adapterConfigs = [
  "crawlee-js",
  "crawlee-py",
  "scrapy",
  "scrapling",
  "firecrawl-self-hosted",
  "spider",
  "colly",
] as const;

describe(
  "competitor crawl and extraction adapters",
  { tags: ["story:2-23", "story:2-27", "story:2-76"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("runs all 7 crawl/extraction competitor adapters against the Proving ground and verifies their published configurations", async ({
      provingGround,
    }) => {
      // Verify every published configuration declares pinned version + ADR 0025 defaults.
      const configs = Object.fromEntries(
        adapterConfigs.map((name) => {
          const url = new URL(`../adapters/${name}/configuration.json`, import.meta.url);
          const parsed: Record<string, unknown> = JSON.parse(readFileSync(url, "utf8"));
          return [name, parsed];
        }),
      );

      for (const name of adapterConfigs) {
        const cfg = configs[name];
        expect(cfg?.["pinned_version"]).toBeTruthy();
        expect(cfg?.["defaults"]).toEqual({
          filter_list_blocking: false,
          consent_prompt_handling: false,
        });
      }
      expect(String(configs["firecrawl-self-hosted"]?.["notes"])).toContain(
        "cloud-only anti-bot engine is absent",
      );
      expect(configs["crawlee-js"]?.["wraps_compare_side"]).toBe("xtask/compare-crawlee");
      expect(configs["scrapy"]?.["wraps_compare_side"]).toBe("xtask/compare-scrapy");
      expect(configs["spider"]?.["wraps_compare_side"]).toBe("xtask/compare-spider");

      const ran = await bench("competitors-crawl-smoke.json", provingGround.address);
      expect(ran.stderr).toBe("");
      expect(ran.code).toBe(0);

      const { record } = await written(ran.out);
      expect(record.cells).toHaveLength(adapterConfigs.length * 2);
      for (const cell of record.cells) {
        expect({ adapter: cell.adapter, task: cell.task, gate: cell.gate }).toEqual({
          adapter: cell.adapter,
          task: cell.task,
          gate: "passed",
        });
        expect(cell.counts.verified).toBeGreaterThanOrEqual(2);
        expect(cell.counts.false_success).toBe(0);
      }
    });
  },
);
