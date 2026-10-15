// A small web service: GET /books?category=<name> crawls that category of
// https://books.toscrape.com/ (following its "next" pages) and answers with its books as JSON.
//
// - Each call builds its own Crawl, its own Foxhound connection and its own Run, so two calls
//   running at once share nothing, and calling again later works the same.
// - If the caller disconnects, an AbortSignal ends the crawl; `await using` then closes the Run
//   and the connection (and so the engine and its browsers) the call opened.
// - The SDK's logs go to the service's own logger, which writes to standard error. Nothing is
//   printed to standard output.

import { createServer } from "node:http";
import type { IncomingMessage, ServerResponse } from "node:http";
import { z } from "zod";
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";
import type { Logger } from "@goodfetch/foxhound";

const SITE = "https://books.toscrape.com/";

const Book = z.object({
  title: z.string(),
  price: z.string(),
});
type Book = z.output<typeof Book>;

// The service's own logger: JSON lines on standard error, never on standard output.
function makeLogger(bindings: Readonly<Record<string, unknown>> = {}): Logger {
  const write = (level: string, message: string, fields?: Readonly<Record<string, unknown>>): void => {
    process.stderr.write(
      `${JSON.stringify({ time: new Date().toISOString(), level, message, ...bindings, ...fields })}\n`,
    );
  };
  return {
    debug: (message, fields) => write("debug", message, fields),
    info: (message, fields) => write("info", message, fields),
    warn: (message, fields) => write("warn", message, fields),
    error: (message, fields) => write("error", message, fields),
  };
}

const serviceLog = makeLogger({ service: "books" });

class CategoryNotFoundError extends Error {
  constructor(readonly category: string) {
    super(`no category named ${JSON.stringify(category)}`);
  }
}

function normalize(name: string): string {
  return name.trim().toLowerCase().replace(/[\s_-]+/g, " ");
}

/** Crawls one category, page by page, and returns its books. `signal` aborting ends the crawl and releases everything. */
async function crawlCategory(category: string, signal: AbortSignal, log: Logger): Promise<Book[]> {
  // One crawl declaration per call: its Labels, Dataset and handlers belong to this call alone.
  const crawl = new Crawl();
  const home = crawl.label("home");
  const listing = crawl.label("listing");
  const books = crawl.dataset("books", { schema: Book });

  const wanted = normalize(category);
  let found = false;

  // The home page: find the category's link in the sidebar and follow it.
  crawl.handle(home, async (ctx) => {
    const links = await ctx.page.locator(".side_categories ul ul li a").all();
    for (const link of links) {
      const text = await link.textContent();
      if (normalize(text) === wanted) {
        found = true;
        await ctx.enqueueLinks(link, listing);
        return;
      }
    }
  });

  // A listing page: push its books, then follow its "next" link.
  crawl.handle(listing, async (ctx) => {
    const cards = await ctx.page.locator("article.product_pod").all();
    for (const card of cards) {
      const title = await card.locator("h3 a").getAttribute("title");
      const price = await card.locator(".price_color").textContent();
      if (title === null) continue;
      await ctx.push(books, { title: title.trim(), price: price.trim() });
    }
    await ctx.enqueueLinks(ctx.page.locator("li.next a"), listing);
  });

  signal.throwIfAborted();
  // Closed last-in first-out when this function ends, however it ends.
  await using foxhound = await Foxhound.connect({ logger: log });
  signal.throwIfAborted();
  await using run = await foxhound.startCrawl(crawl, {
    seeds: [seed(SITE, home)],
    signal,
    timeout: 5 * 60_000,
  });

  const collected: Book[] = [];
  const collect = async (): Promise<void> => {
    using records = run.records(books);
    const stop = (): void => records.close();
    signal.addEventListener("abort", stop, { once: true });
    try {
      for await (const record of records) collected.push(record.value);
    } finally {
      signal.removeEventListener("abort", stop);
    }
  };

  const [report] = await Promise.all([run.finish({ signal }), collect()]);
  signal.throwIfAborted();
  log.info("crawl finished", {
    category,
    books: collected.length,
    verdict: report.verdict,
    completeness: report.completeness,
  });

  if (!found) throw new CategoryNotFoundError(category);
  return collected;
}

function sendJSON(res: ServerResponse, status: number, body: unknown): void {
  if (res.writableEnded || res.destroyed) return;
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

let nextRequestId = 0;

async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");
  if (req.method !== "GET" || url.pathname !== "/books") {
    sendJSON(res, 404, { error: "not found" });
    return;
  }
  const category = url.searchParams.get("category");
  if (category === null || category.trim() === "") {
    sendJSON(res, 400, { error: "the category query parameter is required" });
    return;
  }

  const log = makeLogger({ service: "books", request: ++nextRequestId, category });

  // The caller going away before the answer was sent aborts this call's crawl.
  const controller = new AbortController();
  res.on("close", () => {
    if (!res.writableFinished) controller.abort(new Error("caller disconnected"));
  });

  try {
    const found = await crawlCategory(category, controller.signal, log);
    sendJSON(res, 200, { category, count: found.length, books: found });
  } catch (error) {
    if (controller.signal.aborted) {
      log.info("caller disconnected, crawl stopped");
      return;
    }
    if (error instanceof CategoryNotFoundError) {
      sendJSON(res, 404, { error: error.message });
      return;
    }
    log.error("crawl failed", { error: error instanceof Error ? `${error.name}: ${error.message}` : String(error) });
    sendJSON(res, 502, { error: "the crawl failed" });
  }
}

const server = createServer((req, res) => {
  handle(req, res).catch((error: unknown) => {
    serviceLog.error("unhandled request error", { error: String(error) });
    sendJSON(res, 500, { error: "internal error" });
  });
});

const port = Number(process.env["PORT"] ?? 3000);
server.listen(port, () => {
  serviceLog.info("listening", { port });
});

for (const signalName of ["SIGINT", "SIGTERM"] as const) {
  process.once(signalName, () => {
    serviceLog.info("shutting down", { signal: signalName });
    server.close();
    server.closeAllConnections();
  });
}
