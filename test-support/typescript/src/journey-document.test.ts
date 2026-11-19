import { describe, expect, test } from "vitest";
import type { JourneyOutcome } from "./generated/result-document.ts";
import { journeyDocument, type JourneyFacts } from "./journey-document.ts";
import { journeyVectors } from "./outcome-vectors.ts";

const cases = journeyVectors({
  journey_id: "J-002",
  surface: "sdk-typescript",
  executable: { product: "foxhound", version: "0.0.0" },
  platform: { os: "linux", arch: "x86_64" },
  runtime: "node-24",
});

/** A variant whose one step and one check passed and that names no task. */
const proved: JourneyFacts = {
  journey_id: "J-002",
  surface: "sdk-typescript",
  executable: { product: "foxhound", version: "0.0.0" },
  platform: { os: "linux", arch: "x86_64" },
  runtime: "node-24",
  steps: [{ text: "Start a crawl", outcome: "passed" }],
  checks: [{ text: "The crawl ended", outcome: "passed" }],
  tasks: [],
};

describe("journeyDocument", () => {
  test.for(cases)("gives the outcome of the vector $name", ({ facts, outcome }) => {
    const document = journeyDocument(facts);

    expect(document.outcome).toBe(outcome);
  });

  test("derives the outcome over one the facts carry", () => {
    const smuggled = { ...proved, outcome: "failed" } as JourneyFacts;

    const document = journeyDocument(smuggled);

    expect(document.outcome).toBe("proved");
  });

  test("leaves no way to change the outcome it derived", () => {
    const document = journeyDocument(proved);

    expect(() => {
      (document as { outcome: JourneyOutcome }).outcome = "failed";
    }).toThrow(TypeError);
  });
});
