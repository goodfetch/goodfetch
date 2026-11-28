// Integration test (spec #152, issue 182): runs the smoke campaign covering
// Playwright, Stagehand v4 and Vercel `agent-browser` against the Proving
// ground, verifying pinned configurations, `xtask/compare-collie-arms`
// wrapping, ADR 0025 defaults, and oracle-verified attempts.
import { readFileSync } from "node:fs";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { bench, written } from "./bench.ts";

const browserAdapters = ["playwright", "stagehand-v4", "agent-browser"] as const;

describe(
  "browser-automation competitor adapters",
  { tags: ["story:2-23", "story:2-27", "story:2-76"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("runs Playwright, Stagehand v4 and agent-browser smoke campaign against Proving ground", async ({
      provingGround,
    }) => {
      for (const name of browserAdapters) {
        const cfg: Record<string, unknown> = JSON.parse(
          readFileSync(new URL(`../adapters/${name}/configuration.json`, import.meta.url), "utf8"),
        );
        expect(cfg["pinned_version"]).toBeTruthy();
        expect(cfg["defaults"]).toEqual({
          filter_list_blocking: false,
          consent_prompt_handling: false,
        });
      }

      const ran = await bench("competitors-browser-smoke.json", provingGround.address);
      expect(ran.stderr).toBe("");
      expect(ran.code).toBe(0);

      const { record } = await written(ran.out);
      expect(record.cells).toHaveLength(browserAdapters.length);
      for (const cell of record.cells) {
        expect(cell.gate).toBe("passed");
        expect(cell.counts.verified).toBeGreaterThanOrEqual(2);
        expect(cell.counts.false_success).toBe(0);
      }
    });
  },
);
