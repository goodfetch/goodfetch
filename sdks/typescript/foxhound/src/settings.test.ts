// A crawl's and a Label's settings in code (ADRs 0083 and 0093): written in
// camelCase, sent with each segment in its wire spelling.
import { describe, expect, test } from "vitest";
import { Crawl, crawlDeclaration } from "./crawl.ts";
import { settingsOnWire } from "./generated/settings.ts";

describe("a crawl's settings", () => {
  test("a Label's own settings go on the wire by their snake_case keys, a Label without any with none", () => {
    const crawl = new Crawl();
    crawl.label("detail", {
      settings: { fetch: { tier: "http", httpFrom: "browser" }, crawl: { keepURLFragment: true } },
    });
    crawl.label("listing");

    expect(crawlDeclaration(crawl).labels).toEqual({
      detail: {
        settings: {
          fetch: { tier: "http", http_from: "browser" },
          crawl: { keep_url_fragment: true },
        },
      },
      listing: {},
    });
  });

  test("a crawl's Run settings, Collie's and Foxhound's, go on the wire by their snake_case keys", () => {
    expect(
      settingsOnWire({
        crawl: { handler: { concurrency: 2 }, maxDepth: null },
        filterLists: { firstPartyScripts: true },
      }),
    ).toEqual({
      crawl: { handler: { concurrency: 2 }, max_depth: null },
      filter_lists: { first_party_scripts: true },
    });
  });
});
