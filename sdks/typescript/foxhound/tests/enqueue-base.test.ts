// A handler's relative `enqueue` on an Attached engine of Foxhound's
// executable of the same commit (ADR 0120; issue 1471): the URL resolves
// against the base URL of the document the handler was given, as a
// browser's link would. The Rust SDK's `tests/it/sdk/enqueue.rs` and
// Python's `test_link_payload_attached.py` of Foxhound's package are its
// twins.
import { Crawl, Foxhound, seed, type Enqueued } from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

import { served } from "./served.ts";

describe("a relative enqueue on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 863);

  test(
    "resolves against its document's base URL: its <base href>, or the URL it moved to",
    { tags: ["story:11-1022", "operation:crawl_request.enqueue"] },
    async ({ env, provingGround }) => {
      // `links/1`'s shelf names `<base href="/library/shelves/">`, and
      // `links/2`'s moved from `/old/shelf` to `/new/shelf/`; a book's
      // article names it by its `data-id`, and a wrong base reaches a `404`.
      const url = (ordinal: number, path: string): string =>
        provingGroundUrl(provingGround, `${String(ordinal)}.links.test`, path);
      const crawl = new Crawl();
      const shelf = crawl.label("shelf");
      const book = crawl.label("book");
      const answered: Enqueued[] = [];
      const handled: string[] = [];
      crawl.handle(shelf, async (context) => {
        answered.push(await context.enqueue("book/1", book));
      });
      crawl.handle(book, async ({ page }) => {
        const { values } = await page.read({
          id: { locator: { kind: "css", selector: "article.entry" }, attribute: "data-id" },
        });
        handled.push(String(values["id"]));
      });
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env, settings: served(provingGround) });

      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed(url(1, "/guide/"), shelf), seed(url(2, "/old/shelf"), shelf)],
      });
      const report = await run.finish();
      await foxhound.close();

      expect(report.lifecycle).toBe("finished");
      expect(answered.map((answer) => answer.url).toSorted()).toEqual([
        url(1, "/library/shelves/book/1"),
        url(2, "/new/shelf/book/1"),
      ]);
      expect(handled.toSorted()).toEqual(["B-1-01", "B-2-01"]);
    },
  );
});
