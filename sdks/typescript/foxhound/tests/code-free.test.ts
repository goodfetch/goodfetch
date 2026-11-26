// A code-free crawl (ADR 0093) on an Attached engine of Foxhound's
// executable of the same commit (ADR 0069): `startCrawl` given a reviewed
// `CrawlDeclaration` crawls the Proving ground's `catalog` family with no
// handler in the program, its Labels following links and extracting fields
// into the declared Dataset, which the program reads by its name through
// the raw `call` (ADR 0099) for the oracle to judge. Rust's twin is
// `goodfetch-foxhound`'s `tests/it/sdk/code_free.rs`.
import { Foxhound, type CrawlDeclaration } from "@goodfetch/foxhound";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The catalog crawled as data: listings follow their Next link and their products, and each product page extracts into `products`. */
const catalog: CrawlDeclaration = {
  labels: [
    {
      name: "listing",
      follow: [
        { links: "a[rel=next]", label: "listing" },
        { links: "li.product a", label: "product" },
      ],
    },
    {
      name: "product",
      extract: [
        {
          dataset: "products",
          fields: {
            id: { locator: { kind: "css", selector: ".id" } },
            name: { locator: { kind: "css", selector: "h1.name" } },
            price: { locator: { kind: "css", selector: ".price" }, attribute: "data-price" },
          },
        },
      ],
    },
  ],
  datasets: [{ name: "products", key: ["id"] }],
  settings: { fetch: { tier: "http" }, politeness: { domain: { delay: 0 } } },
};

/** The values of a `dataset.page` answer's Records. */
function values(page: unknown): unknown[] {
  if (typeof page !== "object" || page === null || !("records" in page)) return [];
  const { records } = page;
  if (!Array.isArray(records)) return [];
  return records.map((record: unknown) =>
    typeof record === "object" && record !== null && "value" in record ? record.value : undefined,
  );
}

describe("a code-free crawl on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1467);
  test.override("withBrowser", true);

  test(
    "crawls the catalog family from a declaration, and its Dataset read by name through call meets the oracle",
    { tags: ["attached", "story:11-1069", "operation:crawl.start", "operation:dataset.page"] },
    async ({ env, provingGround }) => {
      const start = await taskStartUrl(provingGround, "catalog/1");
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });

      const run = await foxhound.startCrawl(catalog, { seeds: [{ url: start, label: "listing" }] });
      const report = await run.finish();
      const page = await foxhound.call("dataset.page", {
        run_id: run.id(),
        dataset: "products",
        limit: 1000,
      });
      await foxhound.close();
      const answer = await judgeTask(provingGround, { task: "catalog/1", records: values(page) });

      expect([report.lifecycle, answer.outcome]).toEqual(["finished", "verified"]);
    },
  );
});
