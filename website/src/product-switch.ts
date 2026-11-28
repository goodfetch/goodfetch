// Where the header switcher between the Products leads (ADR 0097): the same
// page in the other Product's section when it has it, its home otherwise.
// Both sections sit in one docs instance, so a page's doc ID is its section's
// key, then its path in the section.

export const products = ["collie", "foxhound"] as const;

export type ProductKey = (typeof products)[number];

/** A Product's name, as its section's title reads it: `Goodfetch Collie`. */
export function productName(product: ProductKey): string {
  return `Goodfetch ${product.charAt(0).toUpperCase()}${product.slice(1)}`;
}

export interface DocRoute {
  id: string;
  path: string;
}

/** The section a doc ID belongs to, or none for a root page. */
export function sectionOf(id: string | undefined): ProductKey | undefined {
  return products.find((product) => id?.startsWith(`${product}/`) === true);
}

/** The path the switcher leads to in `product`'s section of the version whose docs are `docs`. */
export function switchTarget(
  docs: readonly DocRoute[],
  current: string | undefined,
  product: ProductKey,
): string {
  const section = sectionOf(current);
  const page = section === undefined ? undefined : current?.slice(section.length + 1);
  const wanted = [page === undefined ? undefined : `${product}/${page}`, `${product}/index`];
  for (const id of wanted) {
    const doc = docs.find((candidate) => candidate.id === id);
    if (doc !== undefined) {
      return doc.path;
    }
  }
  throw new Error(`the version has no home for ${product}`);
}
