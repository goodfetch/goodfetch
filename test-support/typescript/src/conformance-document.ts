// The `conformance` result document a suite run writes (ADRs 0033, 0063 and
// 0068), built on the types `xtask codegen` generates from the bundle. Its
// outcomes are derived, never set: the rules are `goodfetch-protocol`'s
// constructor's, held to the shared outcome vectors of
// `test-support/fixtures/outcome-vectors/conformance/`, and every reader
// recomputes them through `goodfetch-protocol`'s reader.
import { writeFileSync } from "node:fs";
import type {
  CaseEntry,
  CaseOutcome,
  CaseResult,
  ConformanceDocument,
  Outcome,
  ResultDocument,
} from "./generated/result-document.ts";

/** A `conformance` result document, as written. */
export type ConformanceResultDocument = Readonly<Extract<ResultDocument, { kind: "conformance" }>>;

/** A document's suite, target, platform, runtime and Proving ground. */
export type ConformanceHeader = Omit<ConformanceDocument, "capabilities">;

/** One declared case and each time it ran, before its outcome is derived. */
export type CaseFacts = Omit<CaseResult, "outcome">;

/** One declared capability and its cases, before their outcomes are derived. */
export interface CapabilityFacts {
  readonly name: string;
  readonly cases: readonly CaseFacts[];
}

/** What a `conformance` document holds before its outcomes are derived. */
export interface ConformanceFacts extends ConformanceHeader {
  /** Each capability the suite's case list declares, in its order. */
  readonly capabilities: readonly CapabilityFacts[];
}

/**
 * The document of a suite run. A case is `not_reached` with no entry,
 * `passed` when every entry has a check and every check held, and `failed`
 * otherwise; a capability is `failed` when a case failed, `proved` when it
 * declares a case and every one passed, and `unproved` otherwise. It is
 * frozen, so its outcomes stay the derived ones.
 */
export function conformanceDocument(facts: ConformanceFacts): ConformanceResultDocument {
  return Object.freeze({
    ...facts,
    kind: "conformance",
    capabilities: facts.capabilities.map(({ name, cases }) => {
      const results = cases.map(({ id, covers, entries }) => ({
        id,
        covers,
        entries,
        outcome: caseOutcome(entries),
      }));
      return { name, cases: results, outcome: capabilityOutcome(results) };
    }),
  });
}

/** Writes the document of a suite run to `path` as JSON. */
export function writeConformanceDocument(path: string, facts: ConformanceFacts): void {
  writeFileSync(path, `${JSON.stringify(conformanceDocument(facts), null, 2)}\n`);
}

function caseOutcome(entries: readonly CaseEntry[]): CaseOutcome {
  if (entries.length === 0) {
    return "not_reached";
  }
  const held = entries.every(
    ({ checks }) => checks.length > 0 && checks.every((check) => check.passed),
  );
  return held ? "passed" : "failed";
}

function capabilityOutcome(cases: readonly CaseResult[]): Outcome {
  if (cases.some((one) => one.outcome === "failed")) {
    return "failed";
  }
  return cases.length > 0 && cases.every((one) => one.outcome === "passed") ? "proved" : "unproved";
}
