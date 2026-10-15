/// <reference types="node" />
// Golden scenario 12, "Run a crawl inside a web service" (Foxhound, TypeScript).
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { z } from "zod";
import { Crawl, Foxhound, seed, type CallOptions, type Logger } from "@goodfetch/foxhound";

const Book = z.object({ title: z.string().min(1), price: z.string() });
type Book = z.output<typeof Book>;

// The declaration holds no Run's state, so every call, at once or later, shares it.
const crawl = new Crawl();
const books = crawl.dataset("books", { schema: Book });
const home = crawl.label("home", { payload: z.object({ name: z.string() }) });
const categoryPage = crawl.label("category");

crawl.handle(home, async ({ page, request, enqueueLinks, skip }) => {
  const link = page
    .locator(".side_categories ul ul")
    .getByRole("link", { name: request.payload.name, exact: true });
  const { added } = await enqueueLinks(link, categoryPage);
  if (added.length === 0) skip(`no category named "${request.payload.name}"`);
});

crawl.handle(categoryPage, async ({ page, request, push, enqueueLinks }) => {
  for (const book of await page.locator("article.product_pod").all()) {
    // A missing title attribute fails the schema's `min(1)`: the request fails with RecordInvalidError.
    const title = (await book.locator("h3 a").getAttribute("title")) ?? "";
    await push(books, { title, price: await book.locator(".price_color").textContent() });
  }
  await enqueueLinks(page.locator("li.next a"), request.label);
});

/**
 * The category's books, or `null` when the site has no such category. Each call is a Run of its
 * own, so calls running at once share no state. An abort of `options.signal` ends `finish()`'s wait, and
 * leaving the scope then closes the Run: closing an unfinished Run stops it with a Checkpoint and
 * closes its Sessions, releasing its browser and connections (ADRs 0078, 0102).
 */
export async function booksIn(
  foxhound: Foxhound,
  name: string,
  options?: CallOptions,
): Promise<Book[] | null> {
  await using run = await foxhound.startCrawl(crawl, {
    seeds: [seed("https://books.toscrape.com/", home, { name })],
  });
  const report = await run.finish(options);
  if (report.requests.bySkipReason.handler !== undefined) return null;
  const found: Book[] = [];
  for await (const record of run.records(books)) found.push(record.value);
  return found;
}

// The service's own logger: JSON lines on standard error, so standard output stays empty. With no
// logger, Foxhound logs nothing; `console` would not do, since its `info` writes to standard output.
const write = (level: string) => (message: string, fields?: Readonly<Record<string, unknown>>) => {
  process.stderr.write(`${JSON.stringify({ level, message, ...fields })}\n`);
};
const log: Logger = {
  debug: write("debug"),
  info: write("info"),
  warn: write("warn"),
  error: write("error"),
};

async function answer(
  foxhound: Foxhound,
  req: IncomingMessage,
  res: ServerResponse,
): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");
  const name = url.searchParams.get("category");
  if (url.pathname !== "/books" || name === null) {
    res.writeHead(404).end();
    return;
  }
  const disconnected = new AbortController();
  res.on("close", () => {
    if (!res.writableFinished) disconnected.abort();
  });
  try {
    const found = await booksIn(foxhound, name, { signal: disconnected.signal });
    res.writeHead(found === null ? 404 : 200, { "content-type": "application/json" });
    res.end(JSON.stringify(found ?? { error: `no category named "${name}"` }));
  } catch (error) {
    if (disconnected.signal.aborted) return; // the caller left: the Run is already stopped
    log.error("crawl failed", { category: name, error: String(error) });
    res.writeHead(502).end();
  }
}

async function serve(): Promise<void> {
  // One connection for the service's life: Foxhound installs no signal handler, so the service's own stops it.
  await using foxhound = await Foxhound.connect({ logger: log });
  const server = createServer((req, res) => void answer(foxhound, req, res)).listen(8000);
  await new Promise((stopped) => process.once("SIGTERM", stopped));
  server.close();
}

// A plain script: `node 12-crawl-in-a-service.js Travel`; with no argument, the service.
async function script(name: string): Promise<void> {
  await using foxhound = await Foxhound.connect({ logger: log });
  console.log(JSON.stringify(await booksIn(foxhound, name))); // the script's own output
}

const name = process.argv[2];
await (name === undefined ? serve() : script(name));
