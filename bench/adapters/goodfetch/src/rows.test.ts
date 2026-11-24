import { expect, test } from "vitest";
import { nested, rowSettings, reachSettings } from "./rows.ts";

test("a default row sets nothing: every control stays on", () => {
  expect(rowSettings({ kind: "default" })).toEqual({});
});

test("a matched row switches off the controls it names, and sets Filter-list blocking and Consent prompt handling as the tool's defaults", () => {
  const settings = rowSettings({
    kind: "matched",
    against: "crawlee",
    settings: { "fetch.http_from": "transport" },
    competitor_defaults: { filter_list_blocking: false, consent_prompt_handling: false },
  });

  expect(settings).toEqual({
    "fetch.http_from": "transport",
    "filter_lists.ads": false,
    "filter_lists.trackers": false,
    "filter_lists.annoyances": false,
    "consent.handling": "off",
  });
});

test("a matched row against a tool that blocks and answers prompts keeps both on, and says so", () => {
  const settings = rowSettings({
    kind: "matched",
    against: "browser-use",
    settings: {},
    competitor_defaults: { filter_list_blocking: true, consent_prompt_handling: true },
  });

  expect(settings).toEqual({
    "filter_lists.ads": true,
    "filter_lists.trackers": true,
    "filter_lists.annoyances": true,
    "consent.handling": "answer",
  });
});

test("a matched row may not name a clean-up setting the tool's published defaults decide", () => {
  expect(() =>
    rowSettings({
      kind: "matched",
      against: "crawlee",
      settings: { "consent.handling": "detect" },
      competitor_defaults: { filter_list_blocking: false, consent_prompt_handling: false },
    }),
  ).toThrow(/consent\.handling/);
});

test("the Proving ground's host resolves to its address and is let through the egress guard", () => {
  expect(reachSettings("http://2.catalog.test:41234/products/P-1", "127.0.0.1:41234")).toEqual({
    "network.resolve": { "2.catalog.test": ["127.0.0.1"] },
    "egress.private_ranges": ["2.catalog.test"],
  });
});

test("settings nest by camelCase segment, a map's own keys left as they are", () => {
  expect(
    nested({
      "filter_lists.ads": false,
      "consent.handling": "off",
      "network.resolve": { "2.catalog.test": ["127.0.0.1"] },
    }),
  ).toEqual({
    filterLists: { ads: false },
    consent: { handling: "off" },
    network: { resolve: { "2.catalog.test": ["127.0.0.1"] } },
  });
});
