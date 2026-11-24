// A crawl's sitemap seed on an Attached engine of Foxhound's executable of
// the same commit (ADRs 0093 and 0120; issue 1471): a program starts a
// crawl from `sitemap(url, rules)` beside a request seed, and the engine
// reads the Proving ground's `sitemap-seed` family, each entry handled by
// the Label of the first rule whose pattern matches its URL. The Rust SDK's
// `tests/it/sdk/seeds.rs` and Python's `test_sitemap_seed_attached.py` of
// Foxhound's package are its twins.
import { Crawl, Foxhound, seed, sitemap, type FoxhoundSettings } from "@goodfetch/foxhound";
import { provingGroundUrl, test, type ProvingGroundDescription } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The `sitemap-seed` family's host. */
const host = "sitemap-seed.test";

/**
 * The Connection's settings: plain HTTP fetches, and each of the Proving
 * ground's hosts resolved to its address and reachable although private.
 */
function served(provingGround: ProvingGroundDescription): FoxhoundSettings {
  const hosts = Object.keys(provingGround.resolve);
  const resolve = Object.fromEntries(
    Object.entries(provingGround.resolve).map(([name, address]) => [name, [address]]),
  );
  return { network: { resolve }, egress: { privateRanges: hosts }, fetch: { tier: "http" } };
}

describe("a sitemap seed on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 903);

  test(
    "routes each entry to the Label of its first matching rule",
    { tags: ["story:11-1019", "operation:crawl.start"] },
    async ({ env, provingGround }) => {
      // A book matches `book`'s rule and the later `shelf`'s, and is a
      // `book`; `/about` matches no rule and the partner's book is out of
      // the crawl's scope, so neither is handled. The request seed, of the
      // first book, is a duplicate of its sitemap entry. Each page is an
      // article whose `data-id` names it.
      const url = (path: string): string => provingGroundUrl(provingGround, host, path);
      const crawl = new Crawl();
      const handled: string[] = [];
      const [book, shelf, author] = ["book", "shelf", "author"].map((name) => {
        const label = crawl.label(name);
        crawl.handle(label, async ({ page }) => {
          const { values } = await page.read({
            id: { locator: { kind: "css", selector: "article.entry" }, attribute: "data-id" },
          });
          handled.push(`${name} ${String(values["id"])}`);
        });
        return label;
      });
      if (book === undefined || shelf === undefined || author === undefined) {
        throw new Error("three Labels are declared");
      }
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env, settings: served(provingGround) });

      const run = await foxhound.startCrawl(crawl, {
        seeds: [
          seed(url("/books/1"), book),
          sitemap(url("/sitemap-index.xml"), [
            ["**/books/*", book],
            ["**/books/**", shelf],
            ["**/authors/*", author],
          ]),
        ],
      });
      const report = await run.finish();
      await foxhound.close();

      expect(report.lifecycle).toBe("finished");
      expect(handled.toSorted()).toEqual([
        "author SS-AUTHOR-1",
        "author SS-AUTHOR-2",
        "book SS-BOOK-1",
        "book SS-BOOK-2",
        "book SS-BOOK-3",
      ]);
    },
  );
});
