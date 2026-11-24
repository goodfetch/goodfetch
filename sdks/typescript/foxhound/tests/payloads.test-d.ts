// What a program can and cannot pass as a Payload (ADRs 0082 and 0093, G3):
// a Label's handle carries its Payload's type, from its schema's input side
// or a JSON object for a hand-written JSON Schema, so `tsc` fails on a
// misspelled handle, a Payload of the wrong shape and a Payload given to a
// Label that has none, as the package's typecheck runs it.
import {
  Crawl,
  seed,
  sitemap,
  type CrawlContext,
  type Label,
  type RequestSeed,
  type Seed,
} from "@goodfetch/foxhound";
import { z } from "zod";

const crawl = new Crawl();
const page = crawl.label("page");
const shelf = crawl.label("shelf", { payload: z.object({ number: z.number().int().min(1) }) });
const book = crawl.label("book", {
  payload: { type: "object", properties: { isbn: { type: "string" } }, required: ["isbn"] },
});

// A Label's handle types its Payload: none, the schema's input, a JSON object.
export const pageLabel: Label = page;
export const shelfLabel: Label<{ number: number }> = shelf;
export const bookLabel: Label<{ readonly [key: string]: unknown }> = book;

// A seed carries a Payload only for a Label with one, then its options.
export function seeds(): RequestSeed[] {
  // @ts-expect-error a misspelled handle names nothing
  seed("https://shop.test/", shelff, { number: 1 });
  // @ts-expect-error a Payload of another shape than its Label's
  seed("https://shop.test/shelf/1", shelf, { number: "one" });
  // @ts-expect-error a Label with a Payload needs one
  seed("https://shop.test/shelf/1", shelf);
  // @ts-expect-error a Label without a Payload takes none, only its options
  seed("https://shop.test/", page, { from: "home" }, { priority: 1 });
  // @ts-expect-error a Payload is a JSON object, never a bare JSON value
  crawl.label("isbn", { payload: z.string() });
  return [
    seed("https://shop.test/", page),
    seed("https://shop.test/", page, { priority: 1 }),
    seed("https://shop.test/shelf/1", shelf, { number: 1 }),
    seed("https://shop.test/shelf/1", shelf, { number: 1 }, { priority: 1 }),
    seed("https://shop.test/book/1", book, { isbn: "9780000000001" }),
  ];
}

// A sitemap entry carries no Payload, so a sitemap seed's rule takes only a
// Label without one, and the seed goes beside request seeds in one start.
export function sitemapSeeds(): Seed[] {
  // @ts-expect-error a Label with a Payload names no sitemap entry
  sitemap("https://shop.test/sitemap.xml", [["**/shelf/*", shelf]]);
  return [
    seed("https://shop.test/", page),
    sitemap("https://shop.test/sitemap.xml", [["**/page/*", page]]),
  ];
}

// A handler's enqueue and enqueueLinks follow the same rules.
export async function enqueued(context: CrawlContext): Promise<void> {
  await context.enqueue("/shelf/2", shelf, { number: 2 });
  await context.enqueue("/next", page, { priority: 1 });
  await context.enqueueLinks(context.page.locator("a.shelf"), shelf, { number: 3 }, { limit: 2 });
  await context.enqueueLinks(context.page.locator("a"), page, { limit: 2 });
  // @ts-expect-error a Payload of another shape than its Label's
  await context.enqueue("/shelf/2", shelf, { number: "two" });
  // @ts-expect-error a Label without a Payload takes none, only its options
  await context.enqueue("/next", page, { from: "home" }, { priority: 1 });
  // @ts-expect-error a Label with a Payload needs one
  await context.enqueueLinks(context.page.locator("a.shelf"), shelf);
}
