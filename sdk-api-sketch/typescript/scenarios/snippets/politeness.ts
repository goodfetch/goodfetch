// Snippet for Crawl politeness defaults (#480), on golden scenario 05.
import { Goodfetch, type ExplainedSetting, type Logger, type RequestOutcome } from "goodfetch";

declare const log: Logger; // the program's own logger
declare const outcomes: AsyncIterable<RequestOutcome>; // how a program reads them is #478's

// With nothing set, a crawl obeys robots.txt and keeps one request in flight per domain, one
// second between starts: scenario 05 sets nothing and is polite to books.toscrape.com (G5).
export async function politeByDefault(): Promise<readonly ExplainedSetting[]> {
  await using gf = await Goodfetch.connect();
  // Each comes from the `default` layer unless configuration or an Operator policy set it: what a
  // crawl opened on this connection runs with.
  return gf.explainSettings([
    "politeness.domain.concurrency",
    "politeness.domain.delay",
    "site_preferences.respect_robots_txt",
    "site_preferences.max_crawl_delay",
  ]);
}

// The program's own staging site, which it may press harder: an explicit value, recorded in the
// Audit log and listed in the run's result, and logged once as a warning through `logger`.
export async function ownStagingSite(): Promise<void> {
  await using gf = await Goodfetch.connect({
    logger: log,
    settings: {
      politeness: { domain: { concurrency: 8, delay: 0 } },
      sitePreferences: { respectRobotsTxt: false },
    },
  });
  // @ts-expect-error A misspelled key fails tsc; built at runtime, `setting_unknown` names `respectRobotsTxt`.
  await Goodfetch.connect({ settings: { sitePreferences: { respectRobots: false } } });
  // @ts-expect-error A Collie Run's program directs its own navigation: no per-domain limit (`setting_not_applicable` at runtime).
  await gf.collie.start({ settings: { politeness: { domain: { concurrency: 2 } } } });
  // @ts-expect-error Nor does robots.txt gate it.
  await gf.collie.start({ settings: { sitePreferences: { respectRobotsTxt: true } } });
  // Per-proxy limits apply to every run, a Collie Run included.
  await using run = await gf.collie.start({ settings: { politeness: { proxy: { rate: 5 } } } });
  log.info("started", { run: run.id });
}

// A page the site asked crawlers to leave is skipped, a value with its reason, never a failure (G1).
export async function whySkipped(): Promise<Map<string, number>> {
  const skipped = new Map<string, number>();
  for await (const outcome of outcomes) {
    if (outcome.kind !== "skipped") continue;
    skipped.set(outcome.reason, (skipped.get(outcome.reason) ?? 0) + 1);
    switch (outcome.reason) {
      case "robots_txt":
        log.info("disallowed by the host's robots.txt");
        break;
      case "crawl_delay":
        log.warn("the host's Crawl-delay is above site_preferences.max_crawl_delay");
        break;
      case "use_signals":
        log.info("an AI-use signal refuses this run's Purpose");
        break;
      default:
        log.info("skipped", { reason: outcome.reason });
    }
  }
  return skipped;
}
