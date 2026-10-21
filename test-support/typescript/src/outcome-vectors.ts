// The shared outcome vectors of the `journey` kind (ADR 0063), in
// `test-support/fixtures/outcome-vectors/journey/`: hand-written facts and
// the outcome each must get, which every helper that writes `journey`
// documents passes in its own runner.
import { readdirSync, readFileSync } from "node:fs";
import type {
  CheckOutcome,
  JourneyOutcome,
  OracleOutcome,
  StepOutcome,
} from "./generated/result-document.ts";
import type { JourneyFacts } from "./journey-document.ts";

/** What a vector's facts hold besides its steps, checks and tasks. */
export type JourneyVariant = Omit<JourneyFacts, "steps" | "checks" | "tasks">;

/** One vector, as facts of `variant`. */
export interface JourneyVector {
  /** The vector's file name, without `.json`. */
  readonly name: string;
  readonly facts: JourneyFacts;
  /** The outcome the facts must get. */
  readonly outcome: JourneyOutcome;
}

/** One vector's file. */
interface VectorFile {
  readonly steps: readonly StepOutcome[];
  readonly checks: readonly CheckOutcome[];
  readonly tasks: readonly (OracleOutcome | null)[];
  readonly outcome: JourneyOutcome;
}

const VECTORS = new URL("../../fixtures/outcome-vectors/journey/", import.meta.url);

/**
 * Every `journey` vector, as facts of `variant`: its steps and checks
 * worded by their place, and its tasks numbered in the `catalog` family.
 */
export function journeyVectors(variant: JourneyVariant): JourneyVector[] {
  return readdirSync(VECTORS)
    .filter((name) => name.endsWith(".json"))
    .map((name) => {
      const vector: VectorFile = JSON.parse(readFileSync(new URL(name, VECTORS), "utf8"));
      return {
        name: name.replace(/\.json$/, ""),
        outcome: vector.outcome,
        facts: {
          ...variant,
          steps: vector.steps.map((outcome, index) => ({ text: `Step ${index + 1}`, outcome })),
          checks: vector.checks.map((outcome, index) => ({ text: `Check ${index + 1}`, outcome })),
          tasks: vector.tasks.map((outcome, index) => {
            const id = `catalog/${index + 1}`;
            return outcome === null ? { id } : { id, answer: { task: id, outcome } };
          }),
        },
      };
    });
}
