// The docs site's search (ADR 0097): `@easyops-cn/docusaurus-search-local`
// builds Lunr indexes with the site, served from it, so search needs no
// hosted service and works offline. Each kept version gets one index per
// Product section, which a page of that section searches, and one of the
// root pages, which a root page searches.
//
// A pnpm patch (`patches/`) makes two changes to the plugin's build step:
// - a section's index also holds the root pages, which ADR 0097 has every
//   section's search return;
// - a section is matched on a page's path in its version, as the search bar
//   matches it, not in the site, so `next` and the previous majors find
//   their indexes.
import type { PluginOptions } from "@easyops-cn/docusaurus-search-local";
import { productName, products } from "./product-switch.ts";

export const searchOptions: PluginOptions = {
  indexBlog: false,
  indexPages: false,
  docsRouteBasePath: "/",
  language: ["en"],
  searchContextByPaths: products.map((product) => ({
    label: productName(product),
    path: product,
  })),
  hideSearchBarWithNoSearchContext: false,
  useAllContextsWithNoSearchContext: false,
};
