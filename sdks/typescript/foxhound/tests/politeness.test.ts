// Politeness settings in code: safe defaults, pressing-harder visibility
// (weakened, one logged warning per Run, explainSettings), env refusal,
// politeness.degraded_max refusal, and hedges/tierRace refusal at default
// concurrency 1 (spec #611's #1499; ADR 0086).
import {
  Crawl,
  Foxhound,
  SettingNotApplicableError,
  SettingUnknownError,
  seed,
  type CrawlSettings,
  type Logger,
} from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

function siteSettings(provingGround: {
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
  };
}

describe("Politeness settings in code (Attached engine)", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1499);
  test.override("withBrowser", false);

  test(
    "records pressed-harder values in weakened, one warning log per Run and explainSettings; refuses env, degraded_max, tierRace and hedge at concurrency 1",
    {
      tags: [
        "attached",
        "story:4-117",
        "story:3-63",
        "story:6-263",
        "setting:politeness.domain.concurrency",
        "setting:politeness.domain.delay",
        "setting:politeness.backoff.enabled",
        "setting:politeness.backoff.initial",
        "setting:politeness.backoff.max",
        "setting:politeness.degraded_max",
        "setting:site_preferences.respect_robots_txt",
        "setting:site_preferences.robots_txt_user_agent",
        "setting:fetch.tier_race",
        "setting:fetch.hedge.enabled",
        "error:setting_not_applicable",
        "error:setting_unknown",
      ],
    },
    async ({ env, provingGround }) => {
      const url = provingGroundUrl(provingGround, "sitemap-seed.test", "/books/1");
      const warnRecords: {
        message: string;
        fields: Readonly<Record<string, unknown>> | undefined;
      }[] = [];
      const logger: Logger = {
        level: "warn",
        debug: () => undefined,
        info: () => undefined,
        warn: (message, fields) => {
          warnRecords.push({ message, fields });
        },
        error: () => undefined,
      };

      const crawl = new Crawl();
      const page = crawl.label("page");
      crawl.handle(page, () => undefined);

      // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
      const foxhound = await Foxhound.connect({
        env,
        logger,
        settings: siteSettings(provingGround),
      });
      try {
        const runId = "ts-politeness-pressed";
        const run = await foxhound.startCrawl(crawl, {
          runId,
          seeds: [seed(url, page)],
          settings: {
            politeness: {
              domain: { concurrency: 2, delay: 250 },
              backoff: { enabled: false, initial: 1_000, max: 30_000 },
            },
            sitePreferences: {
              respectRobotsTxt: false,
              robotsTxtUserAgent: "operator-bot",
            },
          },
        });
        const explained = await run.explainSettings();
        const report = await run.finish();
        expect(report.lifecycle).toBe("finished");

        const expectedKeys = [
          "politeness.domain.concurrency",
          "politeness.domain.delay",
          "politeness.backoff.enabled",
          "politeness.backoff.initial",
          "politeness.backoff.max",
          "site_preferences.respect_robots_txt",
          "site_preferences.robots_txt_user_agent",
        ];
        for (const key of expectedKeys) {
          expect(report.weakened?.some((w) => w.key === key && w.layer === "run")).toBe(true);
          const entry = explained.settings.find((s) => s.key === key);
          expect(entry?.layer).toBe("run");
          expect(entry?.weakens).toBe(true);
          const warns = warnRecords.filter(
            (record) =>
              record.fields?.["source"] === "engine" &&
              record.message.includes(runId) &&
              record.message.includes("politeness_weakened") &&
              record.message.includes(key),
          );
          expect(warns).toHaveLength(1);
        }

        // `politeness.degraded_max` is listed in explainSettings() yet refused in settings.
        const connExplain = await foxhound.explainSettings();
        const degraded = connExplain.settings.find((s) => s.key === "politeness.degraded_max");
        expect(degraded?.value).toBe(600_000);
        expect(degraded?.layer).toBe("default");

        await expect(
          foxhound.call("crawl.start", {
            labels: { page: {} },
            seeds: [{ url, label: "page" }],
            settings: { politeness: { degraded_max: 60_000 } },
          }),
        ).rejects.toBeInstanceOf(SettingNotApplicableError);

        // With default concurrency 1, `fetch.tierRace` and `fetch.hedge.enabled` are refused naming the key.
        await expect(
          foxhound.startCrawl(crawl, {
            seeds: [seed(url, page)],
            settings: { fetch: { tierRace: true } },
          }),
        ).rejects.toSatisfy(
          (err: unknown) =>
            err instanceof SettingNotApplicableError &&
            err.message.includes("politeness.domain.concurrency"),
        );

        await expect(
          foxhound.startCrawl(crawl, {
            seeds: [seed(url, page)],
            settings: { fetch: { hedge: { enabled: true } } },
          }),
        ).rejects.toSatisfy(
          (err: unknown) =>
            err instanceof SettingNotApplicableError &&
            err.message.includes("politeness.domain.concurrency"),
        );
      } finally {
        await foxhound.close();
      }

      // Environment variables cannot set pressed-harder politeness settings.
      await expect(
        // oxlint-disable-next-line no-restricted-properties -- Foxhound.connect is the SDK entry point.
        Foxhound.connect({
          env: { ...env, GOODFETCH_POLITENESS_DOMAIN_CONCURRENCY: "4" },
          settings: siteSettings(provingGround),
        }),
      ).rejects.toBeInstanceOf(SettingUnknownError);
    },
  );
});
