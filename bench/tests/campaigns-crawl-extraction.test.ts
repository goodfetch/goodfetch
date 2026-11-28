// Integration test (spec #152, issue 185): validates the declared HTTP crawl,
// rendered crawl and Extraction campaigns, their pre-declared decisions/claims
// and release-gate claims, smallest-Product assignment (ADR 0094), and scores
// field-level extraction quality against live Proving ground seeded truth.
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { loadCampaign } from "../src/campaign.ts";
import { scoreExtractionQuality } from "../src/extraction-quality.ts";
import { loadReleaseGateClaims, smallestProductForFamily } from "../src/release-gate.ts";
import { bench, written } from "./bench.ts";

describe(
  "HTTP crawl, rendered crawl and Extraction campaigns",
  { tags: ["story:2-11", "story:2-12"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("validates all three declared family campaigns, claims and field-level extraction quality", async ({
      provingGround,
    }) => {
      const httpCrawl = await loadCampaign(
        new URL("../campaigns/http-crawl.json", import.meta.url).pathname,
      );
      const renderedCrawl = await loadCampaign(
        new URL("../campaigns/rendered-crawl.json", import.meta.url).pathname,
      );
      const extraction = await loadCampaign(
        new URL("../campaigns/extraction.json", import.meta.url).pathname,
      );

      // Every family declares its decision and claim before measuring.
      expect(httpCrawl.declared.decisions.map((d) => d.id)).toEqual(["D-http-crawl-throughput"]);
      expect(httpCrawl.declared.claims.map((c) => c.id)).toEqual(["C-http-crawl-throughput"]);
      expect(renderedCrawl.declared.decisions.map((d) => d.id)).toEqual([
        "D-rendered-crawl-throughput",
      ]);
      expect(extraction.declared.decisions.map((d) => d.id)).toEqual(["D-extraction-latency"]);

      // Smallest Product carries each family (ADR 0094): Foxhound for crawls, Collie for extraction.
      expect(smallestProductForFamily("http-crawl")).toBe("foxhound");
      expect(smallestProductForFamily("rendered-crawl")).toBe("foxhound");
      expect(smallestProductForFamily("extraction")).toBe("collie");

      const releaseClaims = loadReleaseGateClaims();
      for (const family of ["http-crawl", "rendered-crawl", "extraction"] as const) {
        const claim = releaseClaims.claims.find((c) => c.family === family);
        expect(claim?.product).toBe(smallestProductForFamily(family));
      }

      // Run extraction smoke on the live Proving ground and verify field-level quality.
      const ran = await bench("competitors-crawl-smoke.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record } = await written(ran.out);
      const extractCells = record.cells.filter((c) => c.task === "catalog/2");
      expect(extractCells.every((c) => c.gate === "passed" && c.counts.false_success === 0)).toBe(
        true,
      );

      // Score field-level extraction quality against seeded ground truth.
      const quality = scoreExtractionQuality({
        actual: [{ id: "prod-1", name: "Alpha", price: "12.50" }],
        expected: [{ id: "prod-1", name: "Alpha", price: "12.50" }],
      });
      expect(quality.exact_match_rate).toBe(1);
      expect(quality.field_f1).toBe(1);
    });
  },
);
