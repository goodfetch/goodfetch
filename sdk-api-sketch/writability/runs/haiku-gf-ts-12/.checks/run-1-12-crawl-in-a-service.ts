import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { Crawl, Foxhound, seed, type Logger } from "@goodfetch/foxhound";
import { z } from "zod";

const START_URL = "https://books.toscrape.com/";

const BookSchema = z.object({ title: z.string(), price: z.string() });
const CategoryPayload = z.object({ category: z.string() });

type Book = z.infer<typeof BookSchema>;

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// The service's own logger: one JSON line per entry, on standard error only.
function jsonLogger(): Logger {
  const write = (level: string, message: string, fields?: Readonly<Record<string, unknown>>): void => {
    process.stderr.write(`${JSON.stringify({ time: new Date().toISOString(), level, message, ...fields })}\n`);
  };
  return {
    debug: (message, fields) => write("debug", message, fields),
    info: (message, fields) => write("info", message, fields),
    warn: (message, fields) => write("warn", message, fields),
    error: (message, fields) => write("error", message, fields),
  };
}

// Each call declares its own crawl and starts its own Run, so concurrent calls share nothing but the connection.
async function crawlCategory(
  foxhound: Foxhound,
  logger: Logger,
  category: string,
  signal: AbortSignal,
): Promise<Book[]> {
  const crawl = new Crawl();
  const books = crawl.dataset("books", { schema: BookSchema });
  const home = crawl.label("home", { payload: CategoryPayload });
  const listing = crawl.label("listing", { payload: CategoryPayload });

  crawl.handle(home, async (ctx) => {
    const wanted = ctx.request.payload.category;
    await ctx.enqueueLinks(ctx.page.locator(".side_categories ul ul a"), listing, (link) =>
      link.text.trim() === wanted ? { category: wanted } : null,
    );
  });

  crawl.handle(listing, async (ctx) => {
    const pods = await ctx.page.locator("article.product_pod").all();
    for (const pod of pods) {
      const title = await pod.locator("h3 a").getAttribute("title");
      const price = await pod.locator(".price_color").textContent();
      if (title === null) continue;
      await ctx.push(books, { title, price: price.trim() });
    }
    await ctx.enqueueLinks(ctx.page.locator("li.next a"), listing, { category: ctx.request.payload.category });
  });

  await using run = await foxhound.startCrawl(crawl, {
    seeds: [seed(START_URL, home, { category })],
  });

  // A caller that disconnects stops the Run at its next safe point, which releases its pages and browsers.
  const stop = (): void => {
    run.stop().catch((error: unknown) => {
      logger.warn("stopping the crawl failed", { category, error: messageOf(error) });
    });
  };
  if (signal.aborted) stop();
  else signal.addEventListener("abort", stop, { once: true });
  try {
    await run.finish();
  } finally {
    signal.removeEventListener("abort", stop);
  }
  if (signal.aborted) return [];

  const result: Book[] = [];
  for await (const record of run.records(books)) result.push(record.value);
  return result;
}

function sendJSON(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

async function handleRequest(
  foxhound: Foxhound,
  logger: Logger,
  req: IncomingMessage,
  res: ServerResponse,
): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");
  if (req.method !== "GET" || url.pathname !== "/books") {
    sendJSON(res, 404, { error: "not_found" });
    return;
  }
  const category = url.searchParams.get("category")?.trim() ?? "";
  if (category === "") {
    sendJSON(res, 400, { error: "category_required" });
    return;
  }

  const abort = new AbortController();
  res.on("close", () => {
    if (!res.writableFinished) abort.abort();
  });
  try {
    const books = await crawlCategory(foxhound, logger, category, abort.signal);
    if (!abort.signal.aborted) sendJSON(res, 200, books);
  } catch (error) {
    if (abort.signal.aborted) return;
    logger.error("crawl failed", { category, error: messageOf(error) });
    sendJSON(res, 500, { error: "crawl_failed" });
  }
}

async function main(): Promise<void> {
  const logger = jsonLogger();
  const foxhound = await Foxhound.connect({ logger });

  const server = createServer((req, res) => {
    handleRequest(foxhound, logger, req, res).catch((error: unknown) => {
      logger.error("request failed", { error: messageOf(error) });
      if (res.headersSent) res.destroy();
      else sendJSON(res, 500, { error: "internal_error" });
    });
  });

  const port = Number(process.env["PORT"] ?? 8080);
  server.listen(port, () => logger.info("listening", { port }));

  for (const name of ["SIGINT", "SIGTERM"] as const) {
    process.once(name, () => {
      logger.info("shutting down", { signal: name });
      server.close();
      void foxhound.close().finally(() => process.exit(0));
    });
  }
}

await main();
