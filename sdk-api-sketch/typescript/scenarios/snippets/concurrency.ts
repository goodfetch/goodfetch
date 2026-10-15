// Snippet for Concurrency model per language (#468).
import { Foxhound, ListenerFailedError, type CrawlRun, type Dataset, type Logger } from "@goodfetch/foxhound";

declare const log: Logger; // the program's own logger

export async function firstBooks(limit: number, stop: AbortSignal): Promise<string[]> {
  await using foxhound = await Foxhound.connect({ logger: log });
  await using run = await foxhound.start();
  const page = await run.newPage();

  // Side activity is observed with a listener, subscribed when called; `using` closes it (Q5, Q11).
  using _console = page.on("console", (message) => log.debug(message.text()));
  // A request that needs an answer runs this callback on the program's event loop (Q8).
  using _images = page.route("**/*.{png,jpg}", (route) => route.abort());

  // A glob or a regular expression runs in the engine; a lookahead such as `(?=` would fail here with a
  // ProgramError naming it. A predicate runs in the program and may use its variables (P22).
  const seen = new Set<string>();
  const nextPage = page.waitForResponse(/\/catalogue\/page-\d+\.html$/);
  const fresh = page.waitForEvent("response", { predicate: (response) => !seen.has(response.url()), signal: stop });

  try {
    await page.goto("https://books.toscrape.com/");
    await page.locator("li.next a").click();
    await Promise.all([nextPage, fresh]);
  } catch (error) {
    // A listener that threw was removed; its exception surfaces here, on the page it was registered on.
    if (error instanceof ListenerFailedError) log.error(error.message, { event: error.details.event });
    throw error;
  }
  return (await page.locator("article.product_pod h3 a").allTextContents()).slice(0, limit);
}

// A sequence read to its end; leaving the loop unsubscribes without waiting (Q10). A crawl's Records
// are typed by their Dataset and read from its Run (ADR 0103).
export async function firstTitles(run: CrawlRun, books: Dataset<{ title: string }>, limit: number): Promise<string[]> {
  const titles: string[] = [];
  for await (const record of run.records(books)) {
    titles.push(record.value.title);
    if (titles.length >= limit) break;
  }
  return titles;
}
