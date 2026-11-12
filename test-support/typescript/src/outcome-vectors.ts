// The shared `journey` outcome vectors (ADR 0063), in
// `test-support/fixtures/outcome-vectors/journey/`: hand-written facts and
// the outcomes each must get, which every helper that writes `journey`
// result documents passes in its own runner.
import { readdirSync, readFileSync } from "node:fs";
import type {
  CheckOutcome,
  JourneyOutcome,
  OracleOutcome,
  StepOutcome,
} from "./generated/result-document.ts";
import type { JourneyFacts } from "./journey-document.ts";

/** What a vector's facts hold besides its steps, checks and tasks. */
export type JourneyVariant = Omit<JourneyFacts, "steps" | "checks" | "tasks" | "test_errors">;

/** One vector, as facts of `variant`. */
export interface JourneyVector {
  /** The vector's file name, without `.json`. */
  readonly name: string;
  readonly facts: JourneyFacts;
  /** The outcome the facts must get. */
  readonly outcome: JourneyOutcome;
}

/** One `journey` vector's file. */
interface VectorFile {
  readonly steps: readonly StepOutcome[];
  readonly checks: readonly CheckOutcome[];
  readonly tasks: readonly (OracleOutcome | null)[];
  readonly test_errors?: readonly string[];
  readonly outcome: JourneyOutcome;
}

const VECTORS = new URL("../../fixtures/outcome-vectors/", import.meta.url);

/** Each `journey` vector file: its name without `.json`, and its text. */
function vectorFiles(): { name: string; text: string }[] {
  const folder = new URL("journey/", VECTORS);
  return readdirSync(folder)
    .filter((name) => name.endsWith(".json"))
    .map((name) => ({
      name: name.replace(/\.json$/, ""),
      text: readFileSync(new URL(name, folder), "utf8"),
    }));
}

/**
 * Every `journey` vector, as facts of `variant`: its steps and checks
 * worded by their place, and its tasks numbered in the `catalog` family.
 */
export function journeyVectors(variant: JourneyVariant): JourneyVector[] {
  return vectorFiles().map(({ name, text }) => {
    const vector: VectorFile = JSON.parse(text);
    return journeyVector(name, vector, variant);
  });
}

function journeyVector(name: string, vector: VectorFile, variant: JourneyVariant): JourneyVector {
  return {
    name,
    outcome: vector.outcome,
    facts: {
      ...variant,
      steps: vector.steps.map((outcome, index) => ({ text: `Step ${index + 1}`, outcome })),
      checks: vector.checks.map((outcome, index) => ({ text: `Check ${index + 1}`, outcome })),
      tasks: vector.tasks.map((outcome, index) => {
        const id = `catalog/${index + 1}`;
        return outcome === null ? { id } : { id, answer: { task: id, outcome } };
      }),
      ...(vector.test_errors === undefined ? {} : { test_errors: [...vector.test_errors] }),
    },
  };
}
