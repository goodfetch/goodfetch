// Partial results (ADR 0079), on the Scripted engine as a Collie engine: a
// value of a declared type is complete. A navigation into a redirect loop
// raises `TooManyRedirectsError`, whose details hold what the navigation
// gathered before it failed, and a read of several fields answers each
// field's own outcome, a value or null with why, never one error for the
// call. Python's `test_partial_results.py` and Rust's `partial_results.rs`
// are its twins.
import { Collie, PageError, TooManyRedirectsError } from "@goodfetch/collie";
import { goodfetchFailure, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The hops the looping navigation followed before the browser's count stopped it. */
const HOPS = Array.from({ length: 19 }, (_, hop) => `https://shop.test/loop/${hop + 1}`);

describe("A Collie Connection on the Scripted engine", () => {
  test(
    "puts what a failed call gathered on its error's details and reads each field's own outcome",
    { tags: ["client_case/partial-results", "attached", "story:11-35"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/partial-results.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });

      const run = await collie.start();
      const page = await run.newPage();
      const looping = await goodfetchFailure(page.goto("https://shop.test/loop"));
      const read = await page.read({
        title: { locator: { kind: "css", selector: "h1" }, transform: { kind: "text" } },
        price: {
          locator: { kind: "css", selector: ".price" },
          transform: { kind: "number", locale: "en" },
        },
        rating: {
          locator: { kind: "css", selector: ".rating" },
          transform: { kind: "number", locale: "en" },
        },
      });
      await run.finish();
      await collie.close();

      await clientCase.check(
        "a failed navigation's error holds the hops it followed before failing",
        () => {
          expect(looping).toBeInstanceOf(TooManyRedirectsError);
          expect(looping).toBeInstanceOf(PageError);
          expect(looping).toMatchObject({
            code: "too_many_redirects",
            operation: "page.goto",
            url: "https://shop.test/loop",
          });
          expect(looping.details).toEqual({
            hops: HOPS,
            limit: { kind: "browser", count: 20 },
          });
        },
      );
      await clientCase.check("a read answers each field's own outcome, null with why", () => {
        expect(read.values).toEqual({ title: "Blue kettle", price: null, rating: null });
        expect(read.unread).toEqual([
          { field: "/price", reason: "no_element" },
          { field: "/rating", reason: "not_parsed", text: "five stars" },
        ]);
      });
    },
  );
});
