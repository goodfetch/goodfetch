// The shared outcome vectors (ADR 0063), in
// `test-support/fixtures/outcome-vectors/`: hand-written facts and the
// outcomes each must get, which every helper that writes result documents
// passes in its own runner.
import { readdirSync, readFileSync } from "node:fs";
import type { ConformanceFacts, ConformanceHeader } from "./conformance-document.ts";
import type {
  CaseOutcome,
  CheckOutcome,
  JourneyOutcome,
  OracleOutcome,
  Outcome,
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

/** One `conformance` vector, as one capability's facts under a header. */
export interface ConformanceVector {
  /** The vector's file name, without `.json`. */
  readonly name: string;
  readonly facts: ConformanceFacts;
  /** The outcome each of the capability's cases must get, in order. */
  readonly caseOutcomes: readonly CaseOutcome[];
  /** The outcome the capability must get. */
  readonly outcome: Outcome;
}

/** One `conformance` vector's file. */
interface ConformanceVectorFile {
  readonly cases: readonly {
    readonly entries: readonly { readonly checks: readonly boolean[] }[];
  }[];
  readonly case_outcomes: readonly CaseOutcome[];
  readonly outcome: Outcome;
}

const VECTORS = new URL("../../fixtures/outcome-vectors/", import.meta.url);

/** Each vector file of `kind`: its name without `.json`, and its text. */
function vectorFiles(kind: "journey" | "conformance"): { name: string; text: string }[] {
  const folder = new URL(`${kind}/`, VECTORS);
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
  return vectorFiles("journey").map(({ name, text }) => {
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

/**
 * Every `conformance` vector, as the facts of one capability, named after
 * the vector, under `header`: its cases and checks named by their place.
 */
export function conformanceVectors(header: ConformanceHeader): ConformanceVector[] {
  return vectorFiles("conformance").map(({ name, text }) => {
    const vector: ConformanceVectorFile = JSON.parse(text);
    return conformanceVector(name, vector, header);
  });
}

function conformanceVector(
  name: string,
  vector: ConformanceVectorFile,
  header: ConformanceHeader,
): ConformanceVector {
  return {
    name,
    caseOutcomes: vector.case_outcomes,
    outcome: vector.outcome,
    facts: {
      ...header,
      capabilities: [
        {
          name,
          cases: vector.cases.map(({ entries }, index) => ({
            id: `case-${index + 1}`,
            covers: [],
            entries: entries.map(({ checks }) => ({
              checks: checks.map((passed, at) => ({ name: `Check ${at + 1}`, passed })),
            })),
          })),
        },
      ],
    },
  };
}
