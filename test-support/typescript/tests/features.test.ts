// Seam B (spec #325): the Feature inventory as the Vitest and Playwright
// configurations read it (ADR 0072), validated against the committed schema,
// and the refusal of a Playwright `covers` annotation naming no Feature.
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import { readFeatureIds, refuseUnknownCovers } from "@goodfetch/test-support/features";

const fixtures = fileURLToPath(new URL("fixtures/features/", import.meta.url));

describe("readFeatureIds", () => {
  test("gives every Feature ID of the inventory, a retired one included", () => {
    expect(readFeatureIds(`${fixtures}valid`)).toEqual(["operation:sample.echo", "story:1-3"]);
  });

  test.for([
    ["fails its schema", "unknown-field"],
    ["is missing", "missing"],
  ])("refuses an inventory that %s", ([, folder]) => {
    expect(() => readFeatureIds(`${fixtures}${folder}`)).toThrow(
      expect.objectContaining({ code: "inventory_invalid" }),
    );
  });
});

describe("refuseUnknownCovers", () => {
  const metadata = { features: ["operation:sample.echo", "story:1-3"] };

  test("accepts covers annotations of listed Features beside other annotations", () => {
    expect(() =>
      refuseUnknownCovers(
        [
          { type: "covers", description: "operation:sample.echo" },
          { type: "issue", description: "https://example.invalid/1" },
        ],
        metadata,
      ),
    ).not.toThrow();
  });

  test.for([
    ["names an unknown Feature", "story:1-9"],
    ["names no Feature", undefined],
  ])("refuses a covers annotation that %s", ([, description]) => {
    expect(() => refuseUnknownCovers([{ type: "covers", description }], metadata)).toThrow(
      expect.objectContaining({ code: "covers_unknown" }),
    );
  });

  // A configuration that passes no inventory cannot vouch for a covers
  // annotation, so the annotation fails rather than going unchecked.
  test.for<[string, Readonly<Record<string, unknown>>]>([
    ["passes no Features", {}],
    ["passes Features that are not IDs", { features: [1] }],
  ])("refuses a covers annotation when the configuration %s", ([, configured]) => {
    expect(() =>
      refuseUnknownCovers([{ type: "covers", description: "story:1-3" }], configured),
    ).toThrow(expect.objectContaining({ code: "covers_unknown" }));
  });

  test("accepts a test with no covers annotation when the configuration passes no Features", () => {
    expect(() =>
      refuseUnknownCovers([{ type: "issue", description: "https://example.invalid/1" }], {}),
    ).not.toThrow();
  });
});
