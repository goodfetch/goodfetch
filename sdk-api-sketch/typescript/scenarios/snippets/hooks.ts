// Snippet for Hooks and injectable components in the SDK API (#481, ADR 0104), on golden scenario 05's crawl.
import { z } from "zod";
import { Crawl, Goodfetch, secret, seed, type Logger, type ProxyProviderHandler } from "goodfetch";

declare const log: Logger;
declare const proxies: { lease(domain: string): Promise<{ host: string; user: string }> }; // the program's own proxy pool

const Book = z.object({ title: z.string(), price: z.string(), url: z.url() });
const crawl = new Crawl();
const books = crawl.dataset("books", { schema: Book, key: ["url"] });
const home = crawl.label("home");
const book = crawl.label("book", { payload: z.object({ category: z.string() }) });

// Before request, scoped by Label handle: the Payload is typed, and the change is a returned value.
// The header's value is a Secret placeholder: the engine adds it after the last Hook, so the key
// never passes through the program (ADR 0092).
crawl.beforeRequest(
  ({ request, previousError, skip }) => {
    // Also the stage before a retry: stop retrying a request the site keeps timing out on.
    if (previousError?.code === "deadline_exceeded" && request.attempt > 2) skip(`gave up on ${request.payload.category}`);
    return { headers: { "x-catalog-key": secret("catalog_key"), "x-category": request.payload.category } };
  },
  { labels: [book], name: "catalog-key" },
);

// After response: a soft block served with a 200 is marked here, and feeds escalation by new Session.
crawl.afterResponse(async ({ response, block }) => {
  if (response.status() === 200 && (await response.text()).includes("Access denied")) block("ip_blocked", "soft block page");
});

// Before navigation (browser tier only): images are not needed, and a route is Collie's own way to drop them.
crawl.beforeNavigation(async ({ page }) => {
  page.route("**/*.{png,jpg,jpeg}", (route) => route.abort());
});

// Page ready, unscoped: runs just before every handler, on every tier — Crawlee's router.use.
crawl.pageReady(({ request, tier }) => {
  log.debug("handling", { url: request.url, label: request.label.name, tier });
});

// Before a Record's commit, scoped by Dataset: it sees and returns the Dataset's Record type.
crawl.beforeRecordCommit(
  ({ record, drop }) => {
    if (record.title.trim() === "") drop("no title");
    return { ...record, title: record.title.trim() };
  },
  { datasets: [books], name: "trim-title" },
);

crawl.handle(home, async ({ page, enqueueLinks }) => {
  await enqueueLinks(page.locator("article.product_pod h3 a"), book, { category: "all" });
});
crawl.handle(book, async ({ page, push }) => {
  await push(books, { title: await page.locator("h1").textContent(), price: await page.locator(".price_color").textContent(), url: page.url() });
});

// The program's own Proxy provider, an SDK handler registered on the connection under its name, as
// ADR 0101 registers `modelProviders`: the only one of its kind there, so it is the connection's
// choice for proxies, and `explainSettings()` shows it.
const officePool: ProxyProviderHandler = async ({ domain }) => {
  const { host, user } = await proxies.lease(domain);
  return { server: `http://${host}:3128`, username: user, password: "caller:office-proxy" };
};

export async function crawlWithHooks(): Promise<void> {
  await using gf = await Goodfetch.connect({
    logger: log,
    proxyProviders: { "office-pool": officePool },
    // `caller:office-proxy` resolves here, at the moment of use (ADR 0092).
    resolveSecret: async () => "from the program's own store",
  });
  await using run = await gf.foxhound.startCrawl(crawl, {
    seeds: [seed("https://books.toscrape.com/", home)],
    settings: { secrets: { catalog_key: { reference: "env:CATALOG_KEY", domains: ["books.toscrape.com"] } } },
  });
  void run;
}

// Typed mistakes, caught before anything runs.
export function mistakes(): void {
  crawl.beforeRequest(
    // @ts-expect-error a crawl request stays a GET: a Hook changes its URL and headers, never its method
    () => ({ method: "POST" }),
    { labels: [book] },
  );
  crawl.beforeRecordCommit(
    // @ts-expect-error a Hook scoped to `books` returns a Book
    ({ record }) => ({ ...record, title: 42 }),
    { datasets: [books] },
  );
  crawl.beforeRequest(({ request }) => {
    // @ts-expect-error an unscoped Hook runs for every Label, so its Payload is unknown
    void request.payload.category;
  });
}
