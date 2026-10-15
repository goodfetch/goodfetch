// Snippet for Typed data: schemas, Records and Payloads (#469), on golden scenarios 02 and 05.
import { z } from "zod";
import type { Dataset, JsonSchema, JsonValue, Label, Page, PayloadInput, PayloadOutput, Schema, SchemaInput, SchemaOutput } from "goodfetch";

// Provisional signatures, each its owner's to decide: a crawl's Labels, handlers and `enqueue` are
// #477's, Datasets are #479's. `page.extract` is ADR 0091's.
declare function enqueue<L extends Label<unknown, unknown>>(url: string, label: L, payload: PayloadInput<L>): Promise<void>;
declare function label<S extends Schema>(name: string, payload: S): Label<SchemaInput<S>, SchemaOutput<S>>;
declare function dataset<S extends Schema>(name: string, schema: S): Dataset<SchemaInput<S>, SchemaOutput<S>>;

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

// The library-free way down: a hand-written JSON Schema gives a JsonValue to narrow.
const priceOnly: JsonSchema = { type: "object", properties: { price: { type: "string" } }, required: ["price"] };
export async function rawSchema(page: Page): Promise<JsonValue> {
  return (await page.extract("the price", priceOnly)).output;
}

// Scenario 05: the category name travels as a typed Payload, apart from the engine's own fields.
const bookPage = label("book", z.object({ category: z.string().min(1) }));
const books = dataset("books", Book.extend({ url: z.url(), category: z.string() }));

export async function onCategoryPage(categoryName: string, bookUrls: readonly string[]): Promise<void> {
  for (const url of bookUrls) await enqueue(url, bookPage, { category: categoryName });
  // @ts-expect-error a wrong field name is a type error, not a silent undefined in the handler
  await enqueue("https://books.toscrape.com/x", bookPage, { name: categoryName });
}

export async function onBookPage(page: Page, url: string, payload: PayloadOutput<typeof bookPage>): Promise<void> {
  const { output: book } = await page.extract("the book on this page", Book);
  await books.push({ ...book, url, category: payload.category });
}
