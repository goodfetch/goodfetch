// Snippet for Redirect limits, truncated bodies and transport failures on the fetch tiers (#549, ADR 0118).
import {
  BlockedError,
  Crawl,
  Foxhound,
  ResponseTruncatedError,
  ResponseUnavailableError,
  TooManyRedirectsError,
  seed,
  type Logger,
} from "@goodfetch/foxhound";

declare const log: Logger; // the program's own logger

// A link checker reads each redirect itself: a redirect status listed in `crawl.handle_statuses` is
// not followed, the one way not to follow, so the handler gets the 301 with its `Location`.
const crawl = new Crawl();
const link = crawl.label("link", {
  settings: { crawl: { handleStatuses: [301, 302, 303, 307, 308, 404] } },
});
crawl.handle(link, async ({ response }) => {
  // @ts-expect-error A cut body never reaches a handler, so there is no flag to forget to check (G1).
  void response.truncated;
  const status = response.status();
  if (status >= 300 && status < 400) log.info("redirects", { from: response.url(), to: response.headers()["location"] });
});

// Every other crawl follows Fetch's redirects on every tier, at most `fetch.redirect.max`, and a
// refresh stub's target as one more hop; a loop, a cut body or a long `Retry-After` is a typed outcome.
export async function fetchEdges(): Promise<void> {
  await using foxhound = await Foxhound.connect({ logger: log });
  await using run = await foxhound.startCrawl(crawl, {
    seeds: [seed("https://example.com/", link)],
    settings: {
      fetch: { redirect: { max: 10 }, refresh: { maxDelay: 0 } }, // instant refreshes only
      politeness: { retryAfter: { max: 3_600_000 } }, // wait out a site's maintenance hour
    },
  });
  for await (const outcome of run.outcomes({ kinds: ["failed"] })) {
    if (outcome.kind !== "failed") continue;
    const { error } = outcome;
    if (error instanceof TooManyRedirectsError) log.warn("redirect loop", { chain: error.details.chain });
    else if (error instanceof ResponseTruncatedError) log.warn("cut body", { received: error.details.receivedBytes });
    else if (error instanceof ResponseUnavailableError && error.details.retryAfter !== null)
      log.info("site asked to come back", { at: error.details.retryAfter, fix: error.fix });
  }
  // Requests a long `Retry-After` held are failures, so `retryFailed()` re-admits them once the site is back.
}

// A Collie page follows the browser's 20 redirects and never waits on a `Retry-After`: the program
// directs its own navigation, and reads the site's time from the error.
export async function colliePage(): Promise<void> {
  await using foxhound = await Foxhound.connect({ logger: log });
  await using run = await foxhound.start();
  const page = await run.newPage();
  try {
    await page.goto("https://example.com/");
    const api = await page.request.get("https://example.com/api/items", { maxRedirects: 0 }); // the 3xx itself
    log.info("api", { status: api.status() });
  } catch (e) {
    if (e instanceof TooManyRedirectsError) log.warn("loop", { url: e.details.url });
    else if (e instanceof ResponseTruncatedError && e.retryable) await page.reload(); // the program's own retry
    else if (e instanceof BlockedError && e.details.blockCause === "rate_limit")
      log.warn("rate limited", { until: e.details.retryAfter });
    else throw e;
  }
}
