// Snippet for Capping a crawl's model use without stopping its Run (#513, ADR 0107), on golden scenario 08.
import { z } from "zod";
import { Crawl, Goodfetch, ModelAllowanceReachedError, seed, type Logger, type Page, type StopReason } from "goodfetch";

declare const log: Logger;

const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
});

// Scenario 08: the model is asked once per page type (the Plan), and stops being asked once the crawl
// has spent 0.50 USD. The crawl goes on: books whose layout the Plan learned are still extracted with
// no model call, and a book that needs the model is skipped with the reason `model_allowance`.
const crawl = new Crawl();
const books = crawl.dataset("books", { schema: Book.extend({ url: z.url() }), key: ["url"] });
const category = crawl.label("category");
const book = crawl.label("book");

crawl.handle(category, async ({ page, enqueueLinks }) => {
  await enqueueLinks(page.locator("article.product_pod h3 a"), book);
  await enqueueLinks(page.locator("li.next a"), category);
});

crawl.handle(book, async ({ page, push }) => {
  // Past the allowance, a step with no Plan entry or a Repair raises ModelAllowanceReachedError: left to
  // escape, it ends this request `skipped` with the reason `model_allowance`, not `failed`.
  const { output } = await page.extract("the book on this page", Book);
  await push(books, { ...output, url: page.url() });
});

export async function scenario08(categories: readonly string[]): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: log, settings: { ai: { model: "vercel/openai/gpt-5-mini" } } });
  await using run = await gf.foxhound.startCrawl(crawl, {
    seeds: categories.map((url) => seed(url, category)),
    plan: "books.plan.json", // learned once per page type, replayed with no model call (ADR 0091)
    settings: { ai: { allowance: { price: { amount: "0.50", currency: "USD" } } } }, // the Model allowance: not a Limit
  });
  for await (const outcome of run.outcomes({ kinds: ["skipped"] })) {
    if (outcome.kind === "skipped" && outcome.reason === "model_allowance") log.info("left unextracted", { url: outcome.request.url, why: outcome.message });
  }
  const report = await run.finish(); // `finished`, and `incomplete` when any book was left
  const [allowance] = report.usage.modelAllowance;
  log.info("model use", {
    tokens: report.usage.model.tokens,
    price: report.usage.model.price, // `null` only if a model had no price
    allowanceReachedAt: allowance?.reachedAt ?? null,
    completeness: report.completeness,
    reasons: report.completenessReasons,
  });
}

// A handler that keeps a Record for every book catches the refusal: a skipped attempt keeps nothing.
const keepAll = new Crawl();
const all = keepAll.dataset("books", { schema: z.object({ url: z.url(), book: Book.nullable(), why: z.string().nullable() }), key: ["url"] });
const anyBook = keepAll.label("book");
keepAll.handle(anyBook, async ({ page, push }) => {
  try {
    const { output } = await page.extract("the book on this page", Book);
    await push(all, { url: page.url(), book: output, why: null });
  } catch (error) {
    if (!(error instanceof ModelAllowanceReachedError)) throw error;
    await push(all, { url: page.url(), book: null, why: "the Model allowance was reached" });
  }
});

// An Agent's task ends as a value when the allowance is reached, and the Run goes on.
export async function agentStops(page: Page): Promise<StopReason | null> {
  const result = await page.agent("add the cheapest poetry book to the basket");
  if (result.ended.kind === "stopped" && result.ended.reason.kind === "model_allowance") {
    log.warn("the Model allowance stopped the task", { setting: result.ended.reason.source.setting });
    return result.ended.reason;
  }
  return null;
}

// Counted per Run: a Session or a page takes none.
export async function notPerSession(): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.collie.start({ settings: { ai: { allowance: { calls: 50 } } } });
  // @ts-expect-error a Session takes no Model allowance
  await run.newSession({ settings: { ai: { allowance: { calls: 5 } } } });
}

export const definitions = { crawl, keepAll };
