// Snippet for Crawl output in code (#479, ADR 0103), on golden scenarios 05, 07 and 12.
import { z } from "zod";
import { Crawl, Goodfetch, RecordInvalidError, seed, type CrawlRun, type Logger } from "goodfetch";

declare const log: Logger;

const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  url: z.url(),
  category: z.string(),
});
const Cursor = z.object({ lastPage: z.number().int() });

// Datasets are declared on the crawl, as its Labels are: no connection, no I/O, no default Dataset.
// The key identifies a book collected twice (two categories, or tomorrow's run): Sinks upsert on it.
const crawl = new Crawl();
const books = crawl.dataset("books", { schema: Book, key: ["url"] });
const skipped = crawl.dataset("skipped"); // no schema: any JSON object
const category = crawl.label("category", { payload: z.object({ name: z.string() }) });

crawl.handle(category, async ({ page, request, push, run, enqueueLinks }) => {
  for (const pod of await page.locator("article.product_pod").all()) {
    const price = /£(\d+\.\d{2})/.exec(await pod.locator(".price_color").textContent())?.[1];
    const url = new URL((await pod.locator("h3 a").getAttribute("href")) ?? "", page.url()).href;
    if (price === undefined) {
      await push(skipped, { url, why: "no price" });
      continue;
    }
    // Staged with this attempt: it commits with it, numbered, or is discarded if the attempt fails,
    // retries or escalates, so a retried page never doubles the Dataset (P18).
    await push(books, { title: await pod.locator("h3 a").getAttribute("title") ?? "", price, url, category: request.payload.name });
  }
  // Staged too, through the attempt's `run`: the cursor advances only if this page's Records commit.
  await run.keyValueStore.set(`cursor/${request.payload.name}`, { lastPage: request.depth });
  await enqueueLinks(page.locator("li.next a"), request.label, { name: request.payload.name });
});

// Scenario 12: a crawl in a service reads its Records as they commit, and resumes after a restart
// from the last number it handled. The reader is a cursor over run state: slow or paused, it never
// slows the crawl.
export async function follow(run: CrawlRun, lastHandled: number, save: (n: number) => Promise<void>): Promise<void> {
  try {
    for await (const record of run.records(books, { after: lastHandled })) {
      log.info("book", { n: record.number, title: record.value.title, from: record.url });
      await save(record.number);
    }
  } catch (error) {
    // A Record the schema's own parse now rejects (a changed `.refine()`) ends the loop with its number.
    if (error instanceof RecordInvalidError) log.error("unreadable record", { n: error.details.number });
    throw error;
  }
  for await (const rejected of run.rejectedRecords(books)) log.warn("rejected", { reason: rejected.reason, hook: rejected.hook });
}

export async function scenario05(): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.foxhound.start(crawl, {
    seeds: [seed("https://books.toscrape.com/catalogue/category/books/travel_2/index.html", category, { name: "Travel" })],
    // A Sink is a setting: from code, the project file, the CLI or a Recipe alike.
    settings: { sinks: { warehouse: { type: "postgres", datasets: ["books"], mode: "upsert" } } },
  });
  const cursor = await run.keyValueStore.get("cursor/Travel", Cursor); // typed by its schema, or null
  log.info("cursor", { lastPage: cursor?.lastPage ?? null });
  // An Export of what has committed so far, saved on this machine wherever the engine runs.
  const csv = await run.export(books, { format: "csv" });
  await csv.saveAs("books.csv");
  log.info("exported", { records: csv.records, upTo: csv.lastNumber, sha256: csv.sha256 });
}

// Typed mistakes, caught before anything runs.
export function mistakes(run: CrawlRun): void {
  crawl.handle(crawl.label("other"), async ({ push }) => {
    // @ts-expect-error a Record missing a field of its Dataset's schema
    await push(books, { title: "x", price: "1.00", category: "y" });
    // @ts-expect-error a price that is a number where the schema keeps exact text
    await push(books, { title: "x", price: 1, url: "https://x", category: "y" });
  });
  // @ts-expect-error a key naming a field the schema does not have
  crawl.dataset("misspelled", { schema: Book, key: ["isbn"] });
  // @ts-expect-error the format is named, never guessed from a file name
  void run.export(books, { format: "cvs" });
}
