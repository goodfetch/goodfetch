import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import type {
  CheckOutcome,
  JourneyOutcome,
  OracleOutcome,
  StepOutcome,
} from "./generated/result-document.ts";
import { journeyDocument, type JourneyFacts } from "./journey-document.ts";

/** One shared outcome vector of the `journey` kind (ADR 0063). */
interface Vector {
  readonly description: string;
  readonly steps: readonly StepOutcome[];
  readonly checks: readonly CheckOutcome[];
  readonly tasks: readonly (OracleOutcome | null)[];
  readonly outcome: JourneyOutcome;
}

const vectors = new URL("../../fixtures/outcome-vectors/journey/", import.meta.url);

const cases = readdirSync(vectors)
  .filter((name) => name.endsWith(".json"))
  .map((name) => {
    const vector: Vector = JSON.parse(readFileSync(new URL(name, vectors), "utf8"));
    return { name: name.replace(/\.json$/, ""), vector };
  });

/** A variant holding what `vector` states, its tasks numbered in the `catalog` family. */
function facts(vector: Vector): JourneyFacts {
  return {
    journey_id: "J-002",
    surface: "sdk-typescript",
    executable: { product: "foxhound", version: "0.0.0" },
    platform: { os: "linux", arch: "x86_64" },
    runtime: "node-24",
    steps: vector.steps.map((outcome, index) => ({ text: `Step ${index + 1}`, outcome })),
    checks: vector.checks.map((outcome, index) => ({ text: `Check ${index + 1}`, outcome })),
    tasks: vector.tasks.map((outcome, index) => {
      const id = `catalog/${index + 1}`;
      return outcome === null ? { id } : { id, answer: { task: id, outcome } };
    }),
  };
}

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
  test.for(cases)("gives the outcome of the vector $name", ({ vector }) => {
    const document = journeyDocument(facts(vector));

    expect(document.outcome).toBe(vector.outcome);
  });

  test("writes the journey kind", () => {
    const document = journeyDocument(proved);

    expect(document.kind).toBe("journey");
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
