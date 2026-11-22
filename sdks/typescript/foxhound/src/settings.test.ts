// A crawl's and a Label's settings in code (ADRs 0083 and 0093): written in
// camelCase, sent with each segment in its wire spelling.
import { setImmediate } from "node:timers/promises";
import { describe, expect, test } from "vitest";
import { Crawl, crawlDeclaration } from "./crawl.ts";
import { FoxhoundOperations } from "./generated/operations.ts";
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

describe("FoxhoundOperations", () => {
  test(
    "an operation taking a raw run layer warns once per process of a deprecated key it sets",
    { tags: ["story:11-182"] },
    async () => {
      const operations = new FoxhoundOperations({
        call: (operation) => Promise.reject(new Error(operation)),
      });
      const warnings: Error[] = [];
      const listen = (warning: Error): void => {
        warnings.push(warning);
      };
      const refused: string[] = [];
      process.on("warning", listen);
      try {
        for (const repeats of [3, 2]) {
          await operations
            .runRetryFailed({ runId: "run-1", settings: { sample: { echo_repeats: repeats } } })
            .catch((refusal: unknown) => {
              refused.push(String(refusal));
            });
        }
        await setImmediate();
      } finally {
        process.off("warning", listen);
      }

      expect({
        shown: warnings
          .filter((warning) => "code" in warning && warning.code === "GOODFETCH_DEPRECATED")
          .map((warning) => [warning.name, warning.message]),
        refused,
      }).toEqual({
        shown: [
          [
            "DeprecationWarning",
            "The setting `sample.echo_repeats` is deprecated. Since 0.1, removed no earlier than 1.0: A sample setting, which the real catalog replaces.",
          ],
        ],
        refused: ["Error: run.retry_failed", "Error: run.retry_failed"],
      });
    },
  );
});
