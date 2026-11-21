// Settings in code (ADR 0083): a scope's `settings`, written in camelCase,
// sent with each segment in its wire spelling, and nothing the program
// left out.
import { setImmediate } from "node:timers/promises";
import * as fc from "fast-check";
import { describe, expect, test } from "vitest";
import { type DeprecatedSettings, onWire, type Spelling } from "./settings.ts";

const SPELLING: Spelling = {
  timeout: ["timeout", { navigation: "navigation", run: "run" }],
  filterLists: ["filter_lists", { ads: "ads", firstPartyScripts: "first_party_scripts" }],
  paidFetch: ["paid_fetch", { exchangeRates: "exchange_rates" }],
  session: [
    "session",
    { pageCheck: { setting: "page_check", value: { fields: { login_page: "loginPage" } } } },
  ],
};

/** No setting deprecated. */
const NONE: DeprecatedSettings = new Map();

/** A segment no spelling knows, `__proto__` and its kin included. */
const unknownSegment = fc
  .oneof(fc.constantFrom("__proto__", "constructor", "toString", ""), fc.string())
  .filter((segment) => !Object.hasOwn(SPELLING, segment) && segment !== "navigation");

/** Settings of segments no spelling knows, each holding any JSON value. */
const unknownSettings = fc.dictionary(unknownSegment, fc.jsonValue(), { maxKeys: 4 });

describe("onWire", () => {
  test("sends what it does not know unchanged, at the top and in an area, `__proto__` included", () => {
    fc.assert(
      fc.property(unknownSettings, (settings) => {
        expect(JSON.stringify(onWire(settings, SPELLING, NONE))).toBe(JSON.stringify(settings));
        expect(JSON.stringify(onWire({ timeout: settings }, SPELLING, NONE))).toBe(
          JSON.stringify({ timeout: settings }),
        );
      }),
    );
  });

  test("spells each segment as the wire does, nested as written", () => {
    expect(
      onWire(
        { timeout: { navigation: 30_000 }, filterLists: { firstPartyScripts: true } },
        SPELLING,
        NONE,
      ),
    ).toEqual({ timeout: { navigation: 30_000 }, filter_lists: { first_party_scripts: true } });
  });

  test("sends `null`, which clears a setting, and leaves out what is `undefined`", () => {
    expect(onWire({ timeout: { navigation: undefined, run: null } }, SPELLING, NONE)).toEqual({
      timeout: { run: null },
    });
  });

  test("sends a setting's value as written, its own keys included", () => {
    expect(onWire({ paidFetch: { exchangeRates: { usdPerEur: "1.1" } } }, SPELLING, NONE)).toEqual({
      paid_fetch: { exchange_rates: { usdPerEur: "1.1" } },
    });
  });

  test("sends the fields a setting's value declares by their wire names, and its data as written", () => {
    expect(
      onWire(
        {
          session: {
            pageCheck: { loginPage: "/login", headers: { "X-Page_Id": "1" } },
          },
        },
        SPELLING,
        NONE,
      ),
    ).toEqual({
      session: { page_check: { login_page: "/login", headers: { "X-Page_Id": "1" } } },
    });
    expect(onWire({ session: { pageCheck: null } }, SPELLING, NONE)).toEqual({
      session: { page_check: null },
    });
  });

  test("sends a key it does not know as written, for the engine to refuse by name", () => {
    expect(
      onWire({ timeout: { navigationDelay: 1 }, crawl: { maxPages: 2 } }, SPELLING, NONE),
    ).toEqual({
      timeout: { navigationDelay: 1 },
      crawl: { maxPages: 2 },
    });
  });

  test("sends an area given something other than a table as written", () => {
    expect(onWire({ timeout: 5 }, SPELLING, NONE)).toEqual({ timeout: 5 });
  });

  test("sends nothing for settings left out", () => {
    expect(onWire(undefined, SPELLING, NONE)).toBeUndefined();
  });

  test(
    "warns once per process of a deprecated setting the program sets, `null` included",
    { tags: ["story:11-182"] },
    async () => {
      const deprecated: DeprecatedSettings = new Map([
        [
          "timeout.run",
          "Since 0.1, removed no earlier than 1.0: use `timeout.navigation` instead. Gone.",
        ],
        ["session.page_check", "Since 0.1, removed no earlier than 1.0: Gone."],
      ]);
      const warnings: Error[] = [];
      const listen = (warning: Error): void => {
        warnings.push(warning);
      };
      process.on("warning", listen);
      try {
        onWire({ timeout: { navigation: 1, run: 2 } }, SPELLING, deprecated);
        onWire({ timeout: { run: null } }, SPELLING, deprecated);
        onWire({ session: { pageCheck: { loginPage: "/login" } } }, SPELLING, deprecated);
        onWire({ timeout: { navigation: 1 } }, SPELLING, deprecated);
        await setImmediate();
      } finally {
        process.off("warning", listen);
      }

      expect(
        warnings.map((warning) => [
          warning.name,
          "code" in warning ? warning.code : undefined,
          warning.message,
        ]),
      ).toEqual([
        [
          "DeprecationWarning",
          "GOODFETCH_DEPRECATED",
          "The setting `timeout.run` is deprecated. Since 0.1, removed no earlier than 1.0: use `timeout.navigation` instead. Gone.",
        ],
        [
          "DeprecationWarning",
          "GOODFETCH_DEPRECATED",
          "The setting `session.page_check` is deprecated. Since 0.1, removed no earlier than 1.0: Gone.",
        ],
      ]);
    },
  );
});
