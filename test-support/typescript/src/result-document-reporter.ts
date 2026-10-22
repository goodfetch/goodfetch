// The Playwright reporter of result documents (ADRs 0063 and 0068). It
// writes no case log.
//
// - For each test the Journey helper ran, it writes the variant's `journey`
//   document, `<id>@<surface>.json`, from the facts the helper attached,
//   deriving the outcome through `journeyDocument`. One run that runs a
//   variant in several projects (the Release suite's browsers) writes one
//   file for it: a failed run's document is kept over a later one, so the
//   variant is `failed` when any browser failed it, as the Journey report
//   judges a variant. A test that did not pass, though its facts say every
//   step and check did (`failOnErrors` or the accessibility audit failing
//   it in teardown), gets its errors as the facts' `test_errors`, so its
//   variant is `failed`.
// - For each Console conformance target, a project whose `metadata` names
//   it as `conformanceTarget`, it writes one `conformance` document,
//   `<target>.json`, deriving the outcomes through `conformanceDocument`.
//   A test of the project carrying a `capability` annotation runs the case
//   of that capability whose ID is the test's title, which the suite's case
//   list must declare (`conformance_case_unknown` otherwise). Each run of it
//   is an entry whose one check is whether the test passed; a skipped test
//   leaves its case `not_reached`, and a declared capability no test
//   reached stays `unproved`. The rest of the header comes from the facts
//   `@goodfetch/test-support/conformance`'s `test` attached: every test of
//   one target must report the same (`conformance_target_mismatch`). A
//   target no test attached facts for gets no document. A refusal fails the
//   run.
//
// The folder is its `out` option, else `GOODFETCH_TEST_RESULT_DOCUMENTS`
// (`just test` sets it to `target/result-documents/`), else
// `target/result-documents`. The case list is its `caseList` option, else
// `conformance/console/cases.toml` beside the configuration file.
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import type {
  FullConfig,
  FullProject,
  FullResult,
  Reporter,
  TestCase,
  TestResult,
} from "@playwright/test/reporter";
import { readCaseList, type CaseList } from "./case-list.ts";
import { writeConformanceDocument, type ConformanceHeader } from "./conformance-document.ts";
import { TestSupportError } from "./errors.ts";
import type { CaseEntry, JourneyOutcome, Target } from "./generated/result-document.ts";
import { writeJourneyDocument, type JourneyFacts } from "./journey-document.ts";

/** The name of the attachment through which the Journey helper hands a run's facts to the reporter. */
export const JOURNEY_FACTS = "goodfetch-journey-facts";

/** The name of the attachment through which a conformance test hands its target's facts to the reporter. */
export const CONFORMANCE_FACTS = "goodfetch-conformance-facts";

/** The annotation that names the capability whose case a test runs. */
export const CAPABILITY_ANNOTATION = "capability";

/** The key of a project's `metadata` that names its Console conformance target. */
export const CONFORMANCE_TARGET = "conformanceTarget";

/** The one check of a conformance test's entry. */
const TEST_PASSED = "the test passed";

/** A target's identity besides its name: its version or digest, and its Product. */
export type TargetIdentity = Target extends infer One
  ? One extends unknown
    ? Omit<One, "name">
    : never
  : never;

/** What a conformance test reports of its target: its document's header but for the suite and the target's name. */
export interface TargetFacts extends Omit<ConformanceHeader, "suite" | "target"> {
  readonly target: TargetIdentity;
}

/** The options a configuration gives the reporter. */
export interface ResultDocumentReporterOptions {
  /** The folder the documents go to. */
  readonly out?: string | undefined;
  /** The Console conformance suite's case list. */
  readonly caseList?: string | undefined;
}

/** What one target's tests gave so far. */
interface TargetRun {
  /** The facts the first test that attached them reported. */
  facts: TargetFacts | undefined;
  /** Each case's entries, by capability, then case ID. */
  readonly entries: Map<string, Map<string, CaseEntry[]>>;
}

/** A test as far as the reporter reads it. */
export type ReportedTest = Pick<TestCase, "title" | "annotations"> & {
  readonly parent: { project(): Pick<FullProject, "metadata"> | undefined };
};

/** A test's result as far as the reporter reads it. */
export type ReportedResult = Pick<TestResult, "status" | "attachments"> & {
  readonly errors: readonly Pick<TestResult["errors"][number], "message">[];
};

/** Writes the `journey` document of each Journey variant and the `conformance` document of each Console conformance target a run ran. */
export default class ResultDocumentReporter implements Reporter {
  readonly #out: string;
  readonly #caseListOption: string | undefined;
  #caseListPath: string;
  #caseList: CaseList | undefined;
  /** Each variant's outcome as written so far by this run. */
  readonly #written = new Map<string, JourneyOutcome>();
  readonly #targets = new Map<string, TargetRun>();
  readonly #refusals: TestSupportError[] = [];

  constructor(options: ResultDocumentReporterOptions = {}) {
    this.#out =
      options.out ?? process.env["GOODFETCH_TEST_RESULT_DOCUMENTS"] ?? "target/result-documents";
    this.#caseListOption = options.caseList;
    this.#caseListPath = options.caseList ?? join("conformance", "console", "cases.toml");
  }

  printsToStdio(): boolean {
    return false;
  }

  onBegin(config: Pick<FullConfig, "configFile" | "rootDir">): void {
    this.#caseListPath =
      this.#caseListOption ??
      join(
        config.configFile === undefined ? config.rootDir : dirname(config.configFile),
        "conformance",
        "console",
        "cases.toml",
      );
  }

  onTestEnd(test: ReportedTest, result: ReportedResult): void {
    this.#journey(result);
    try {
      this.#conformance(test, result);
    } catch (error) {
      if (!(error instanceof TestSupportError)) {
        throw error;
      }
      this.#refusals.push(error);
      throw error;
    }
  }

  async onEnd(): Promise<{ status: FullResult["status"] } | undefined> {
    for (const [name, run] of this.#targets) {
      if (run.facts !== undefined) {
        this.#writeConformance(name, run.facts, run.entries);
      }
    }
    return this.#refusals.length > 0 ? { status: "failed" } : undefined;
  }

  #journey(result: ReportedResult): void {
    const body = result.attachments.find(({ name }) => name === JOURNEY_FACTS)?.body;
    if (body === undefined) {
      return;
    }
    const attached: JourneyFacts = JSON.parse(body.toString("utf8"));
    // The test can fail after the body recorded its facts: `failOnErrors`
    // and the accessibility audit fail it in teardown (ADR 0068).
    const facts: JourneyFacts =
      result.status === "passed" ? attached : { ...attached, test_errors: testErrors(result) };
    const variant = `${facts.journey_id}@${facts.surface}`;
    if (this.#written.get(variant) === "failed") {
      return;
    }
    mkdirSync(this.#out, { recursive: true });
    this.#written.set(variant, writeJourneyDocument(join(this.#out, `${variant}.json`), facts));
  }

  #conformance(test: ReportedTest, result: ReportedResult): void {
    const capabilities = test.annotations
      .filter(({ type }) => type === CAPABILITY_ANNOTATION)
      .map(({ description }) => description ?? "");
    const target: unknown = test.parent.project()?.metadata[CONFORMANCE_TARGET];
    if (capabilities.length === 0 || typeof target !== "string") {
      return;
    }
    const run = this.#targets.get(target) ?? { facts: undefined, entries: new Map() };
    this.#targets.set(target, run);
    const body = result.attachments.find(({ name }) => name === CONFORMANCE_FACTS)?.body;
    if (body !== undefined) {
      const facts: TargetFacts = JSON.parse(body.toString("utf8"));
      if (run.facts !== undefined && JSON.stringify(run.facts) !== JSON.stringify(facts)) {
        throw new TestSupportError(
          "conformance_target_mismatch",
          `the test "${test.title}" reports the target ${target} as ${JSON.stringify(facts)}, another test as ${JSON.stringify(run.facts)}`,
        );
      }
      run.facts = facts;
    }
    const caseList = this.#declared();
    for (const capability of capabilities) {
      const declared = caseList.capabilities
        .find(({ name }) => name === capability)
        ?.cases.some(({ id }) => id === test.title);
      if (declared !== true) {
        throw new TestSupportError(
          "conformance_case_unknown",
          `the test "${test.title}" runs a case of capability \`${capability}\` that ${this.#caseListPath} does not declare`,
        );
      }
      if (result.status === "skipped") {
        continue;
      }
      const cases = run.entries.get(capability) ?? new Map<string, CaseEntry[]>();
      run.entries.set(capability, cases);
      cases.set(test.title, [
        ...(cases.get(test.title) ?? []),
        { checks: [{ name: TEST_PASSED, passed: result.status === "passed" }] },
      ]);
    }
  }

  #declared(): CaseList {
    this.#caseList ??= readCaseList(this.#caseListPath);
    return this.#caseList;
  }

  #writeConformance(
    name: string,
    facts: TargetFacts,
    entries: ReadonlyMap<string, ReadonlyMap<string, CaseEntry[]>>,
  ): void {
    const caseList = this.#declared();
    mkdirSync(this.#out, { recursive: true });
    writeConformanceDocument(join(this.#out, `${name}.json`), {
      ...facts,
      suite: caseList.suite,
      target: { name, ...facts.target },
      capabilities: caseList.capabilities.map((capability) => ({
        name: capability.name,
        cases: capability.cases.map(({ id, covers }) => ({
          id,
          covers: [...covers],
          entries: entries.get(capability.name)?.get(id) ?? [],
        })),
      })),
    });
  }
}

/** What failed a test that did not pass, one line each; its status when Playwright gave no error. */
function testErrors(result: ReportedResult): string[] {
  const messages = result.errors.map(({ message }) => (message ?? "").split("\n")[0] ?? "");
  const named = messages.filter((message) => message.length > 0);
  return named.length > 0 ? named : [`the test ended \`${result.status}\``];
}
