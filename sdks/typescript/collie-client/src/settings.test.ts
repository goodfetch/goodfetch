// Settings in code (ADR 0083): a scope's `settings`, written in camelCase,
// sent with each segment in its wire spelling, and nothing the program
// left out.
import { describe, expect, test } from "vitest";
import { onWire, type Spelling } from "./settings.ts";

const SPELLING: Spelling = {
  timeout: ["timeout", { navigation: "navigation", run: "run" }],
  filterLists: ["filter_lists", { ads: "ads", firstPartyScripts: "first_party_scripts" }],
  paidFetch: ["paid_fetch", { exchangeRates: "exchange_rates" }],
};

describe("onWire", () => {
  test("spells each segment as the wire does, nested as written", () => {
    expect(
      onWire(
        { timeout: { navigation: 30_000 }, filterLists: { firstPartyScripts: true } },
        SPELLING,
      ),
    ).toEqual({ timeout: { navigation: 30_000 }, filter_lists: { first_party_scripts: true } });
  });

  test("sends `null`, which clears a setting, and leaves out what is `undefined`", () => {
    expect(onWire({ timeout: { navigation: undefined, run: null } }, SPELLING)).toEqual({
      timeout: { run: null },
    });
  });

  test("sends a setting's value as written, its own keys included", () => {
    expect(onWire({ paidFetch: { exchangeRates: { usdPerEur: "1.1" } } }, SPELLING)).toEqual({
      paid_fetch: { exchange_rates: { usdPerEur: "1.1" } },
    });
  });

  test("sends a key it does not know as written, for the engine to refuse by name", () => {
    expect(onWire({ timeout: { navigationDelay: 1 }, crawl: { maxPages: 2 } }, SPELLING)).toEqual({
      timeout: { navigationDelay: 1 },
      crawl: { maxPages: 2 },
    });
  });

  test("sends an area given something other than a table as written", () => {
    expect(onWire({ timeout: 5 }, SPELLING)).toEqual({ timeout: 5 });
  });

  test("sends nothing for settings left out", () => {
    expect(onWire(undefined, SPELLING)).toBeUndefined();
  });
});
