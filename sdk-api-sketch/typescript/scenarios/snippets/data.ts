// Snippet for Typed data: schemas, Records and Payloads (#469), on golden scenarios 02 and 05.
import { z } from "zod";
import { Crawl, type CrawlContext, type JSONSchema, type JSONValue, type Page } from "@goodfetch/foxhound";

// `page.extract` is ADR 0091's; a crawl's Labels, handlers and `enqueue` are ADR 0093's (crawl.ts),
// its Datasets and `push` ADR 0103's (output.ts).

// Scenario 02: a typed object, the price an exact decimal kept as text by the schema, never a float.
const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
  description: z.string(),
});

export async function typedExtract(page: Page): Promise<string> {
  const { output: book } = await page.extract("the book on this page", Book); // checked by JSON Schema, then by Zod itself, inside the retry loop
  return `${book.title}: £${book.price}`;
}

// The library-free way down: a hand-written JSON Schema gives a JSONValue to narrow.
const priceOnly: JSONSchema = { type: "object", properties: { price: { type: "string" } }, required: ["price"] };
export async function rawSchema(page: Page): Promise<JSONValue> {
  return (await page.extract("the price", priceOnly)).output;
}

// Scenario 05: the category name travels as a typed Payload, apart from the engine's own fields.
const crawl = new Crawl();
const bookPage = crawl.label("book", { payload: z.object({ category: z.string().min(1) }) });
const books = crawl.dataset("books", { schema: Book.extend({ url: z.url(), category: z.string() }) });

export async function onCategoryPage({ enqueue }: CrawlContext, categoryName: string, bookUrls: readonly string[]): Promise<void> {
  for (const url of bookUrls) await enqueue(url, bookPage, { category: categoryName });
  // @ts-expect-error a wrong field name is a type error, not a silent undefined in the handler
  await enqueue("https://books.toscrape.com/x", bookPage, { name: categoryName });
}

crawl.handle(bookPage, async ({ page, request, push }) => {
  const { output: book } = await page.extract("the book on this page", Book);
  await push(books, { ...book, url: page.url(), category: request.payload.category });
});
