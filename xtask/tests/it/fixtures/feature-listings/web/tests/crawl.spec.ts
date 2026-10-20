import { expect, test } from "@playwright/test";

test.describe(
  "Crawl",
  { annotation: { type: "covers", description: "event:sample.echoed" } },
  () => {
    test(
      "crawls",
      { annotation: { type: "issue", description: "https://example.invalid/1" } },
      () => {
        expect(1).toBe(1);
      },
    );
  },
);
