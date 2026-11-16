// The golden scenario "Crawl two page types into a Dataset" (TESTING.md),
// as a program writes it: a listing page's handler enqueues each product
// it links to, and a product page's handler reads the product and pushes
// it as one Record into the keyed Dataset `products`. The Crawl is declared
// at module scope, since it holds no connection and does no I/O.
import { Crawl, type Dataset, type Label, type PageRead } from "@goodfetch/foxhound";

/** One product, as the product page gives it. */
export interface Product {
  readonly id: string;
  readonly name: string;
  readonly price: string;
}

export const crawl: Crawl = new Crawl();
export const listing: Label = crawl.label("listing");
const product = crawl.label("product");
export const products: Dataset<Product> = crawl.dataset<Product>("products", { key: ["id"] });

crawl.handle(listing, async (context) => {
  const { values } = await context.page.read({
    products: { list: { locator: { kind: "css", selector: "li.product a" }, attribute: "href" } },
  });
  for (const href of texts(values, "products")) {
    await context.enqueue(href, product);
  }
});

crawl.handle(product, async (context) => {
  const { values } = await context.page.read({
    id: { locator: { kind: "css", selector: "span.id" } },
    name: { locator: { kind: "css", selector: "h1.name" } },
    price: { locator: { kind: "css", selector: "span.price" }, attribute: "data-price" },
  });
  await context.push(products, {
    id: text(values, "id"),
    name: text(values, "name"),
    price: text(values, "price"),
  });
});

/** The text a value field read, or an error naming the field the page did not give. */
function text(values: PageRead["values"], field: string): string {
  const value = values[field];
  if (typeof value !== "string") throw new Error(`the page gave no ${field}`);
  return value;
}

/** The texts a list field read, those the page did not give left out. */
function texts(values: PageRead["values"], field: string): string[] {
  const list = values[field];
  return Array.isArray(list) ? list.filter((value) => typeof value === "string") : [];
}
