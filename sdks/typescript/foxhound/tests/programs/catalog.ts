// A crawl of the Proving ground's catalog family, as a program writes it:
// one Label for its listing pages, whose handler pushes each product the
// page lists as one Record into the keyed Dataset `products` and enqueues
// the page its Next link leads to, under its own Label.
import {
  Crawl,
  type CrawlContext,
  type Dataset,
  type Label,
  type PageRead,
} from "@goodfetch/foxhound";

/** One product, as the listing page gives it, the price without its currency sign. */
export interface Product {
  readonly id: string;
  readonly name: string;
  readonly price: string;
}

export const catalog: Crawl = new Crawl();
export const listing: Label = catalog.label("listing");
export const products: Dataset<Product> = catalog.dataset<Product>("products", { key: ["id"] });

catalog.handle(listing, (context) => crawlListing(context, listing, products));

/** Pushes what the listing page of `context` lists into `into`, and enqueues its next page under `label`. */
export async function crawlListing(
  context: CrawlContext,
  label: Label,
  into: Dataset<Product>,
): Promise<void> {
  const { values } = await context.page.read({
    ids: { list: { locator: { kind: "css", selector: "li.product" }, attribute: "data-id" } },
    names: { list: { locator: { kind: "css", selector: "li.product a" } } },
    prices: {
      list: {
        locator: { kind: "css", selector: "li.product span.price" },
        match: { pattern: "([0-9]+\\.[0-9]{2})" },
      },
    },
    next: { locator: { kind: "css", selector: "a[rel=next]" }, attribute: "href" },
  });
  const ids = texts(values, "ids");
  const names = texts(values, "names");
  const prices = texts(values, "prices");
  if (names.length !== ids.length || prices.length !== ids.length) {
    throw new Error(
      `the page lists ${ids.length} products but ${names.length} names and ${prices.length} prices`,
    );
  }
  for (const [index, id] of ids.entries()) {
    await context.push(into, { id, name: names[index] ?? "", price: prices[index] ?? "" });
  }
  const next = values["next"];
  if (typeof next === "string") await context.enqueue(next, label);
}

/** The texts a list field read, or an error naming a field the page did not give in full. */
function texts(values: PageRead["values"], field: string): string[] {
  const list = values[field];
  if (!Array.isArray(list) || !list.every((value) => typeof value === "string")) {
    throw new Error(`the page gave no ${field}`);
  }
  return list;
}
