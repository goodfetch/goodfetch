// What a program can and cannot write in a Connection's, a crawl's and a
// Label's `settings` (ADRs 0083 and 0093): `tsc` fails on a wrong one, as
// the package's typecheck runs it.
import {
  Crawl,
  Foxhound,
  type CrawlSettings,
  type FoxhoundSettings,
  type LabelSettings,
} from "@goodfetch/foxhound";

declare const foxhound: Foxhound;

// A crawl takes Collie's Run settings and Foxhound's, each segment in camelCase.
export const crawlSettings: CrawlSettings = {
  timeout: { navigation: 30_000 },
  crawl: { handler: { concurrency: 4 }, retry: { handlerFailed: true } },
  politeness: { domain: { concurrency: 2 } },
};
// The Connection's take Foxhound's keys too, and a Run's own timeout, as
// its Runs' default.
export const connectionSettings: FoxhoundSettings = {
  timeout: { run: 60_000 },
  crawl: { maxDepth: 3 },
};
// A Label takes its own tier, page check and main-content step.
export const labelSettings: LabelSettings = {
  fetch: { tier: "http" },
  content: { extractor: "trafilatura" },
};

// @ts-expect-error a Label never presses its Run's Politeness
export const labelPoliteness: LabelSettings = { politeness: { domain: { concurrency: 2 } } };
// @ts-expect-error the tier race is the Run's alone
export const labelRace: LabelSettings = { fetch: { tierRace: true } };
// @ts-expect-error a Label's Sessions take their Run's proxy
export const labelProxy: LabelSettings = { proxy: { server: null } };
// @ts-expect-error a key the catalog does not have
export const unknownKey: CrawlSettings = { crawl: { handler: { parallelism: 4 } } };
// @ts-expect-error the wire's snake_case is not the TypeScript spelling
export const snake: CrawlSettings = { crawl: { retry: { handler_failed: true } } };
// @ts-expect-error a crawl's own timeout is its start's, never its settings
export const crawlTimeout: CrawlSettings = { timeout: { run: 60_000 } };
// @ts-expect-error a value outside the setting's own
export const wrongTier: LabelSettings = { fetch: { tier: "fast" } };

// `crawl.label` and `startCrawl` take them.
export async function declared(): Promise<void> {
  const crawl = new Crawl();
  const listing = crawl.label("listing", { settings: { fetch: { tier: "browser" } } });
  // @ts-expect-error a Label takes no Run-wide key
  crawl.label("other", { settings: { crawl: { maxDepth: 2 } } });
  crawl.handle(listing, () => undefined);
  await foxhound.startCrawl(crawl, {
    seeds: [{ url: "https://shop.test/", label: listing.name }],
    settings: { crawl: { handler: { concurrency: 1 } } },
  });
}

// `connect()` takes the Connection's settings, Foxhound's keys with them.
export async function connected(): Promise<void> {
  // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Foxhound.connect({ settings: { crawl: { handler: { concurrency: 2 } } } });
  // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Foxhound.connect({
    // @ts-expect-error configuration alone sets the proxy providers
    settings: { proxy: { providers: {} } },
  });
}
