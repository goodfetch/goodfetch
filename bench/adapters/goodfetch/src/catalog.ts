// The Goodfetch adapter's programs for the Proving ground's catalog family,
// as a user writes them on each Product's public surface: Foxhound crawls
// the listing pages (`crawl`), and Collie reads one product page (`extract`).
import type { Page } from "@goodfetch/collie";
import { Crawl, type Dataset, type Label, type PageRead } from "@goodfetch/foxhound";

/** One product, as the listing page gives it, the price without its currency sign. */
export interface Product {
  readonly id: string;
  readonly name: string;
  readonly price: string;
}

/** The crawl of the listing pages: each page's products into the keyed Dataset `products`, then its Next link. */
export const catalog: Crawl = new Crawl();
export const listing: Label = catalog.label("listing");
export const products: Dataset<Product> = catalog.dataset<Product>("products", { key: ["id"] });

catalog.handle(listing, async (context) => {
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
    await context.push(products, { id, name: names[index] ?? "", price: prices[index] ?? "" });
  }
  const next = values["next"];
  if (typeof next === "string") await context.enqueue(next, listing);
});

/** The product a product page shows, read in one call. */
export async function productOf(page: Page): Promise<Readonly<Record<string, unknown>>> {
  const read = await page.read({
    id: { locator: { kind: "css", selector: "span.id" } },
    name: { locator: { kind: "css", selector: "h1.name" } },
    price: {
      locator: { kind: "css", selector: "span.price" },
      attribute: "data-price",
      transform: { kind: "number", locale: "en" },
    },
  });
  return read.values;
}

/** The texts a list field read, or an error naming a field the page did not give in full. */
function texts(values: PageRead["values"], field: string): string[] {
  const list = values[field];
  if (!Array.isArray(list) || !list.every((value) => typeof value === "string")) {
    throw new Error(`the page gave no ${field}`);
  }
  return list;
}
