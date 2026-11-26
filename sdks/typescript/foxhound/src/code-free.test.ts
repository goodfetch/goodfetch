// A code-free crawl (ADR 0093) on the Scripted engine (ADR 0069):
// `startCrawl` given a reviewed `CrawlDeclaration` sends it as `crawl.start`'s
// `declaration`, with seeds naming their Label by name and `detached`, and
// the program reads a declared Dataset by its name through the raw `call`
// (ADR 0099). Its twin is the Python package's
// `tests/unit/test_code_free_crawl.py`.
import { playScript, test } from "@goodfetch/test-support";
import { describe, expect, expectTypeOf } from "vitest";
import type { Crawl, CrawlDeclaration, Foxhound, Seed } from "./index.ts";
import { connectOver } from "./testing.ts";

const declaration: CrawlDeclaration = {
  labels: [
    {
      name: "listing",
      follow: [{ links: "a.next", label: "listing", limit: 10 }],
      extract: [
        {
          dataset: "products",
          fields: { id: { locator: { kind: "css", selector: "article.product .id" } } },
        },
      ],
    },
  ],
  datasets: [{ name: "products", key: ["id"] }],
};

describe("a code-free crawl", () => {
  test(
    "starts detached from a declaration, and its Dataset reads by name through call",
    { tags: ["story:11-1069"] },
    async () => {
      const played = playScript("run/foxhound-code-free-crawl.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.startCrawl(declaration, {
        seeds: [{ url: "https://shop.test/catalog/1", label: "listing" }],
        runId: "catalog",
        detached: true,
      });
      const page = await foxhound.call("dataset.page", { run_id: "catalog", dataset: "products" });
      await run.close();
      await foxhound.close();

      expect(run.id()).toBe("catalog");
      expect(page).toMatchObject({ records: [{ number: 1, value: { id: "p-1" } }], through: 1 });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("detaches only a code-free crawl, and names a declaration or an extraction, not both", () => {
    const start = (foxhound: Foxhound, crawl: Crawl): void => {
      const seeds: [Seed] = [{ url: "https://shop.test/", label: "page" }];
      // @ts-expect-error -- a program's crawl waits on its handlers, so it never detaches.
      void foxhound.startCrawl(crawl, { seeds, detached: true });
      // @ts-expect-error -- a start names a declaration or an extraction.
      void foxhound.startCrawl({ ...declaration, extraction: { formats: {} } }, { seeds });
      void foxhound.startCrawl(
        { extraction: { formats: { markdown: true } } },
        { seeds, detached: true },
      );
    };
    expectTypeOf(start).toBeFunction();
  });
});
