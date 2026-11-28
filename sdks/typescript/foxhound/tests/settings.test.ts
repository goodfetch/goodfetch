// Crawl-space settings, URL-shape caps, and `explainSettings()` for Foxhound's
// keys (spec #611's #1500; ADRs 0083, 0093 and 0120).
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { Crawl, Foxhound, seed, type CrawlSettings, type Enqueued } from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

function fastSiteSettings(provingGround: {
  readonly resolve: Readonly<Record<string, string>>;
}): CrawlSettings {
  const resolve: Record<string, readonly string[]> = {};
  for (const [host, address] of Object.entries(provingGround.resolve)) {
    resolve[host] = [address];
  }
  return {
    fetch: { tier: "http" },
    network: { resolve },
    egress: { privateRanges: Object.keys(resolve) },
    politeness: { domain: { concurrency: 8, delay: 0 } },
  };
}

describe("Crawl-space settings and explainSettings() (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1500);
  test.override("withBrowser", false);

  test(
    "enforces crawl bounds and URL caps with not-added reasons and clears caps with null",
    {
      tags: [
        "attached",
        "story:6-227",
        "story:6-228",
        "setting:crawl.max_depth",
        "setting:crawl.max_requests",
        "setting:crawl.max_url_length",
        "setting:crawl.max_path_segments",
        "setting:crawl.max_segment_repeats",
      ],
    },
    async ({ env, provingGround }) => {
      const url = (p: string): string => provingGroundUrl(provingGround, "traps.test", p);
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: fastSiteSettings(provingGround),
      });

      const answers: Enqueued[] = [];
      const crawl = new Crawl();
      const root = crawl.label("root");
      const child = crawl.label("child");
      const seedUrl = url("/archive/");
      crawl.handle(root, async (ctx) => {
        answers.push(
          await ctx.enqueue(
            url("/archive/very-long-segment-that-exceeds-the-configured-length-cap"),
            child,
          ),
        );
        answers.push(await ctx.enqueue(url("/a/b/c/d"), child));
        answers.push(await ctx.enqueue(url("/x/x/y"), child));
      });
      crawl.handle(child, () => undefined);

      const capped = await foxhound.startCrawl(crawl, {
        seeds: [seed(seedUrl, root)],
        settings: {
          crawl: {
            maxURLLength: seedUrl.length + 10,
            maxPathSegments: 3,
            maxSegmentRepeats: 1,
          },
        },
      });
      const cappedReport = await capped.finish();
      expect({
        completeness: cappedReport.completeness,
        reasons: cappedReport.completenessReasons,
        answers: answers.map((ans) => (ans.kind === "not_added" ? ans.reason : ans.kind)),
      }).toEqual({
        completeness: "unverified",
        reasons: expect.any(Array),
        answers: ["max_url_length", "max_path_segments", "max_segment_repeats"],
      });

      answers.length = 0;
      const uncapped = await foxhound.startCrawl(crawl, {
        seeds: [seed(seedUrl, root)],
        settings: {
          crawl: {
            maxURLLength: null,
            maxPathSegments: null,
            maxSegmentRepeats: null,
          },
        },
      });
      const uncappedReport = await uncapped.finish();
      expect(uncappedReport.completeness).toBe("complete");
      expect(answers.map((ans) => ans.kind)).toEqual(["added", "added", "added"]);

      await foxhound.close();
    },
  );

  test(
    "explainSettings() reports value, layer and lock for Foxhound keys and equals report.settings",
    {
      tags: [
        "attached",
        "story:6-314",
        "setting:crawl.max_depth",
        "setting:crawl.max_requests",
        "setting:crawl.max_url_length",
        "operation:config.explain",
      ],
    },
    async ({ env, provingGround }) => {
      const home = env["GOODFETCH_HOME"];
      expect(home).toBeDefined();
      const perMajor = path.join(home ?? "", "0.0");
      mkdirSync(perMajor, { recursive: true });
      writeFileSync(
        path.join(perMajor, "operator-policy.toml"),
        'version = "0.1"\n\n[lock.crawl]\nmax_depth = 2\n\n[cap.crawl]\nmax_requests = 10\n',
      );

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: fastSiteSettings(provingGround),
      });
      const crawl = new Crawl();
      const page = crawl.label("page");
      crawl.handle(page, () => undefined);

      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed(provingGroundUrl(provingGround, "traps.test", "/archive/"), page)],
        settings: { crawl: { maxURLLength: 4_096 } },
      });
      const explained = await run.explainSettings();
      const byKey = new Map((explained.settings ?? []).map((entry) => [entry.key, entry]));

      expect(byKey.get("crawl.max_depth")).toMatchObject({
        value: 2,
        layer: "operator_policy",
        lockedBy: "operator_policy",
      });
      expect(byKey.get("crawl.max_requests")).toMatchObject({
        value: 10,
        layer: "operator_policy",
        cap: 10,
      });
      expect(byKey.get("crawl.max_url_length")).toMatchObject({
        value: 4_096,
        layer: "run",
      });
      expect(byKey.get("crawl.max_path_segments")).toMatchObject({
        value: 20,
        layer: "default",
      });

      const report = await run.finish();
      expect(report.lifecycle).toBe("finished");
      expect(report.settings).toEqual(explained.settings);

      await foxhound.close();
    },
  );
});
