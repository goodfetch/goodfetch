// One crawl with Crawlee, for `just compare` (ADR 0138's matrix): from
// `--url`, at most `--concurrency` requests at once, every page of the site,
// with `--host`, when given, sent as each request's `Host` so the Proving
// ground serves its family on the loopback address. `--scenario` names the
// matrix's scenario:
//
// - `raw`: `CheerioCrawler`, nothing kept on disk (`persistStorage: false`),
//   links followed on the same hostname (`enqueueLinks`' default);
// - `polite`: the same with robots.txt obeyed and one request start a second
//   on each domain (`sameDomainDelaySecs: 1`), every link followed, since
//   the multi-domain site spans many hosts and links nothing else;
// - `extraction`: `raw` with each page's title and main text given to
//   `pushData`, its default Dataset kept on disk (`persistStorage: true`,
//   Crawlee's default);
// - `browser`: `PlaywrightCrawler` in the headless Chrome `--browser` names,
//   nothing kept on disk;
// - `auto` (#2411): `AdaptivePlaywrightCrawler` at its defaults, which
//   fetches each page over HTTP or renders it in the Chrome `--browser`
//   names as it predicts, nothing kept on disk; each page's Records, every
//   `article.item`'s `p.id` and `h1.name`, read with `parseWithCheerio()`,
//   and written, with whether a browser page handled it, as one JSON line
//   `{url, rendered, records}` a page to `pages.jsonl` in the folder it runs
//   in once the crawl ends. The crawler may run a page's handler twice, over
//   HTTP and in the browser, to learn which the page needs, and keeps the
//   data of one run alone, so a page's line is the run it kept;
// - `hostile`: the hostile pages' pass at each tool's defaults (#2439):
//   `CheerioCrawler` with no option but the `Host` it sends, at Crawlee's
//   defaults (`maxConcurrency` 200, robots.txt not obeyed, its storage on
//   disk), `--concurrency` unread.
//
// The crawl-like scenario (#2284) runs `raw` on an `https` site, which
// `got-scraping` fetches over HTTP/2 when ALPN offers it, its default.
//
// Scenario 2's cell says why Crawlee has none. It prints one line,
// `pages=<n>`, the pages Crawlee says it handled; the comparison's same-work
// check reads the Proving ground's own observations instead.
//
//   node crawl.mjs --url http://127.0.0.1:<port>/sites/1000/1 \
//     --host scale.test:<port> --concurrency 16 --scenario raw

import { writeFile } from "node:fs/promises";
import process from "node:process";
import { parseArgs } from "node:util";

import { CheerioCrawler, Configuration } from "@crawlee/cheerio";

const SCENARIOS = ["raw", "polite", "extraction", "browser", "auto", "hostile"];

const { values } = parseArgs({
  options: {
    url: { type: "string" },
    host: { type: "string" },
    concurrency: { type: "string" },
    scenario: { type: "string" },
    browser: { type: "string" },
  },
  strict: true,
});
const concurrency = Number(values.concurrency);
if (
  !values.url ||
  !Number.isInteger(concurrency) ||
  concurrency < 1 ||
  !SCENARIOS.includes(values.scenario) ||
  (["browser", "auto"].includes(values.scenario) && !values.browser)
) {
  process.stderr.write(
    "usage: node crawl.mjs --url <url> [--host <host:port>] --concurrency <n> " +
      "--scenario <raw|polite|extraction|browser|auto|hostile> [--browser <chrome>]\n",
  );
  process.exit(2);
}
const { scenario } = values;

let pages = 0;
/** `text` with its runs of white space made one space, and trimmed. */
const collapsed = (text) => text.replace(/\s+/g, " ").trim();
// At most 16 in flight, as the other tools. Its autoscaled pool starts at
// the crawler's desired concurrency and grows toward the limit as the system
// allows: Crawlee's own behaviour, kept. At its defaults, no limit is set.
// Outside the hostile pass, an `https` site's certificate is verified, which
// Crawlee skips by default, against Node.js's roots and the Proving ground's
// authority `NODE_EXTRA_CA_CERTS` names.
const options = {
  ...(scenario === "hostile" ? {} : { maxConcurrency: concurrency, ignoreSslErrors: false }),
  preNavigationHooks: values.host
    ? [
        (_context, gotOptions) => {
          gotOptions.headers = { ...gotOptions.headers, host: values.host };
        },
      ]
    : [],
};
// Every link of the multi-domain site is on another host; it links nothing
// but its own pages.
const enqueued = scenario === "polite" ? { strategy: "all" } : {};

let crawler;
if (scenario === "browser") {
  const { PlaywrightCrawler } = await import("@crawlee/playwright");
  crawler = new PlaywrightCrawler(
    {
      maxConcurrency: concurrency,
      launchContext: {
        launchOptions: {
          executablePath: values.browser,
          headless: true,
          args: ["--use-mock-keychain", "--password-store=basic", "--no-first-run"],
        },
      },
      async requestHandler({ enqueueLinks }) {
        pages += 1;
        await enqueueLinks();
      },
    },
    new Configuration({ persistStorage: false }),
  );
} else if (scenario === "auto") {
  const { AdaptivePlaywrightCrawler } = await import("@crawlee/playwright");
  crawler = new AdaptivePlaywrightCrawler(
    {
      maxConcurrency: concurrency,
      launchContext: {
        launchOptions: {
          executablePath: values.browser,
          headless: true,
          // Its HTTP fetches skip certificate checks, Crawlee's default
          // there; its Chrome does likewise.
          args: [
            "--use-mock-keychain",
            "--password-store=basic",
            "--no-first-run",
            "--ignore-certificate-errors",
          ],
        },
      },
      async requestHandler(context) {
        const $ = await context.parseWithCheerio();
        const records = $("article.item")
          .toArray()
          .map((item) => ({
            id: collapsed($(item).find("p.id").text()),
            name: collapsed($(item).find("h1.name").text()),
          }))
          .filter(({ id, name }) => id !== "" || name !== "");
        // A browser run hands the handler its page; an HTTP run a getter
        // that throws.
        const rendered = "value" in (Object.getOwnPropertyDescriptor(context, "page") ?? {});
        await context.pushData({
          url: context.request.loadedUrl ?? context.request.url,
          rendered,
          records,
        });
        // In a browser run, `enqueueLinks()` waits up to 5 seconds for a
        // link and fails the request when none comes: a page without one
        // enqueues nothing.
        if ($("a").length > 0) {
          await context.enqueueLinks();
        }
      },
    },
    new Configuration({ persistStorage: false }),
  );
} else {
  crawler = new CheerioCrawler(
    {
      ...options,
      ...(scenario === "polite" ? { sameDomainDelaySecs: 1, respectRobotsTxtFile: true } : {}),
      async requestHandler({ $, request, enqueueLinks, pushData }) {
        pages += 1;
        if (scenario === "extraction") {
          await pushData({
            url: request.url,
            title: $("title").text(),
            text: $("main").text().trim(),
          });
        }
        await enqueueLinks(enqueued);
      },
    },
    scenario === "hostile"
      ? new Configuration()
      : new Configuration({ persistStorage: scenario === "extraction" }),
  );
}

await crawler.run([values.url]);
if (scenario === "auto") {
  const { items } = await crawler.getData();
  // A page whose handler ran twice counts once.
  pages = items.length;
  await writeFile("pages.jsonl", items.map((item) => `${JSON.stringify(item)}\n`).join(""));
}
process.stdout.write(`pages=${pages}\n`);
