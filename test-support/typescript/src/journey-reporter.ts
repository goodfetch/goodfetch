// The Playwright reporter of the Console's Journey variants (ADRs 0063 and
// 0068): for each test the Journey helper ran, it writes the variant's
// `journey` result document, `<id>@<surface>.json`, from the facts the
// helper attached, deriving the outcome through `journeyDocument`, as every
// runner does. It writes no case log.
//
// The folder is its `out` option, else `GOODFETCH_TEST_RESULT_DOCUMENTS`
// (`just test` sets it to `target/result-documents/`), else
// `target/result-documents`. One run that runs a variant in several
// projects (the Release suite's browsers) writes one file for it: a failed
// run's document is kept over a later one, so the variant is `failed` when
// any browser failed it, as the Journey report judges a variant.
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Reporter, TestResult } from "@playwright/test/reporter";
import type { JourneyOutcome } from "./generated/result-document.ts";
import { writeJourneyDocument, type JourneyFacts } from "./journey-document.ts";

/** The name of the attachment through which the Journey helper hands a run's facts to the reporter. */
export const JOURNEY_FACTS = "goodfetch-journey-facts";

/** The options a configuration gives the reporter. */
export interface JourneyReporterOptions {
  /** The folder the documents go to. */
  readonly out?: string | undefined;
}

/** Writes the `journey` document of each Journey variant a run ran. */
export default class JourneyReporter implements Reporter {
  readonly #out: string;
  /** Each variant's outcome as written so far by this run. */
  readonly #written = new Map<string, JourneyOutcome>();

  constructor(options: JourneyReporterOptions = {}) {
    this.#out =
      options.out ?? process.env["GOODFETCH_TEST_RESULT_DOCUMENTS"] ?? "target/result-documents";
  }

  printsToStdio(): boolean {
    return false;
  }

  onTestEnd(_test: unknown, result: Pick<TestResult, "attachments">): void {
    const body = result.attachments.find(({ name }) => name === JOURNEY_FACTS)?.body;
    if (body === undefined) {
      return;
    }
    const facts: JourneyFacts = JSON.parse(body.toString("utf8"));
    const variant = `${facts.journey_id}@${facts.surface}`;
    if (this.#written.get(variant) === "failed") {
      return;
    }
    mkdirSync(this.#out, { recursive: true });
    this.#written.set(variant, writeJourneyDocument(join(this.#out, `${variant}.json`), facts));
  }
}
