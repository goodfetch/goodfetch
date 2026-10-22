// The `journey` result document a Journey variant run writes (ADR 0063),
// built on the types `xtask codegen` generates from the bundle. Its outcome
// is derived, never set: the rule is `goodfetch-protocol`'s constructor's,
// held to the shared outcome vectors of `test-support/fixtures/outcome-vectors/`,
// and every reader recomputes it through `goodfetch-protocol`'s reader.
import { writeFileSync } from "node:fs";
import type {
  JourneyDocument,
  JourneyOutcome,
  ResultDocument,
} from "./generated/result-document.ts";

/** A `journey` result document, as written. */
export type JourneyResultDocument = Readonly<Extract<ResultDocument, { kind: "journey" }>>;

/** What a `journey` document holds before its outcome is derived. */
export type JourneyFacts = Omit<JourneyDocument, "outcome">;

/**
 * The document of a variant run. It is `proved` when every step passed or is
 * `manual`, every check passed, every named task's oracle answered
 * `verified` and no error failed the test outside them, and `failed`
 * otherwise; it is frozen, so its outcome stays the
 * derived one.
 */
export function journeyDocument(facts: JourneyFacts): JourneyResultDocument {
  return Object.freeze({ ...facts, kind: "journey", outcome: outcome(facts) });
}

/** Writes the document of a variant run to `path` as JSON, and gives its outcome. */
export function writeJourneyDocument(path: string, facts: JourneyFacts): JourneyOutcome {
  const document = journeyDocument(facts);
  writeFileSync(path, `${JSON.stringify(document, null, 2)}\n`);
  return document.outcome;
}

function outcome(facts: JourneyFacts): JourneyOutcome {
  const steps = facts.steps.every((step) => step.outcome === "passed" || step.outcome === "manual");
  const checks = facts.checks.every((check) => check.outcome === "passed");
  const tasks = facts.tasks.every((task) => task.answer?.outcome === "verified");
  const test = (facts.test_errors ?? []).length === 0;
  return steps && checks && tasks && test ? "proved" : "failed";
}
