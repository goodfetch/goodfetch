// Crawl request headers on seeds and `enqueue`, deduplication, and forbidden
// header rejection (spec #611's #3660; ADRs 0033, 0093 and 0104).
import {
  Crawl,
  Foxhound,
  HeaderForbiddenError,
  seed,
  type CrawlSettings,
  type Enqueued,
} from "@goodfetch/foxhound";
import { provingGroundObservations, provingGroundUrl, test } from "@goodfetch/test-support";
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

describe("Request headers on seeds and enqueue (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 3660);
  test.override("withBrowser", false);

  test(
    "carries seed and enqueue headers to Proving ground, exposes them on ctx.request, deduplicates ignoring headers, and refuses forbidden headers",
    {
      tags: [
        "attached",
        "story:11-1019",
        "operation:crawl_request.enqueue",
        "error:header_forbidden",
      ],
    },
    async ({ env, provingGround }) => {
      const host = "access.test";
      const rootUrl = provingGroundUrl(provingGround, host, "/corpus/article/1");
      const childUrl = provingGroundUrl(provingGround, host, "/corpus/article/2");
      const blockedUrl = provingGroundUrl(provingGround, host, "/corpus/article/3");
      const forbidden: readonly (readonly [string, string])[] = [
        ["Host", "other.test"],
        ["Cookie", "session=disallowed"],
        ["Authorization", "disallowed"],
        ["Proxy-Authorization", "disallowed"],
        ["X-HTTP-Method-Override", "DELETE"],
      ];

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      const foxhound = await Foxhound.connect({
        env,
        settings: fastSiteSettings(provingGround),
      });

      try {
        const seenHeaders = new Map<string, Readonly<Record<string, string>> | undefined>();
        let duplicateAnswer: Enqueued | undefined;
        const enqueueForbiddenErrors: string[] = [];

        const crawl = new Crawl();
        const root = crawl.label("root");
        const child = crawl.label("child");

        crawl.handle(root, async (ctx) => {
          seenHeaders.set("root", ctx.request.headers);
          const first = await ctx.enqueue(childUrl, child, {
            headers: { "X-TS-Enqueue": "enqueue-val" },
          });
          expect(first.kind).toBe("added");
          duplicateAnswer = await ctx.enqueue(childUrl, child, {
            headers: { "X-TS-Enqueue": "other-val" },
          });
          for (const [name, value] of forbidden) {
            try {
              await ctx.enqueue(blockedUrl, child, { headers: { [name]: value } });
              enqueueForbiddenErrors.push(`${name}:unexpected_success`);
            } catch (error) {
              if (error instanceof HeaderForbiddenError) {
                enqueueForbiddenErrors.push(`${name}:${error.code}`);
              } else {
                enqueueForbiddenErrors.push(`${name}:unexpected_error:${String(error)}`);
              }
            }
          }
        });
        crawl.handle(child, (ctx) => {
          seenHeaders.set("child", ctx.request.headers);
        });

        for (const [name, value] of forbidden) {
          await expect(
            foxhound.startCrawl(crawl, {
              seeds: [seed(blockedUrl, child, { headers: { [name]: value } })],
            }),
          ).rejects.toBeInstanceOf(HeaderForbiddenError);
        }

        const run = await foxhound.startCrawl(crawl, {
          seeds: [seed(rootUrl, root, { headers: { "X-TS-Seed": "seed-val" } })],
        });
        const report = await run.finish();
        expect(report.lifecycle).toBe("finished");

        const observations = await provingGroundObservations(provingGround);
        const headerOf = (target: string, name: string): string | undefined => {
          const lower = name.toLowerCase();
          return observations.requests
            .find((req) => req.host === host && req.target === target)
            ?.headers.find(([k]) => k.toLowerCase() === lower)?.[1];
        };

        expect(headerOf("/corpus/article/1", "x-ts-seed")).toBe("seed-val");
        expect(headerOf("/corpus/article/2", "x-ts-enqueue")).toBe("enqueue-val");
        expect(observations.requests.some((req) => req.target === "/corpus/article/3")).toBe(false);
        expect(duplicateAnswer).toMatchObject({ kind: "not_added", reason: "duplicate" });
        expect(seenHeaders.get("root")).toEqual({ "X-TS-Seed": "seed-val" });
        expect(seenHeaders.get("child")).toEqual({ "X-TS-Enqueue": "enqueue-val" });
        expect(enqueueForbiddenErrors).toEqual(
          forbidden.map(([name]) => `${name}:header_forbidden`),
        );
      } finally {
        await foxhound.close();
      }
    },
  );
});
