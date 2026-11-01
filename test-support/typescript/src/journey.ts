// The Playwright Journey helper (ADRs 0063, 0068 and 0094): a `console`
// variant of a Journey is written as its steps and checks only.
//
//   journey("J-001", "console", async ({ step, check, expectTask, cli, daemon, page }) => {
//     await step("Open the local Console's address", () => page.goto(daemon.url));
//     await check("The local Console opens with no login", () => …);
//   });
//
// `journey(id, surface, body)` reads `journeys/<id>.toml`, validated against
// `journeys/journey.schema.json`, and declares one test of test support's
// Playwright `test`, tagged `@<id>` and `@<surface>` (TESTING.md, Journeys)
// and carrying a `journey` annotation, `<id>@<surface>`. Its `daemon`
// fixture runs the executable of the Product the file's `product` names,
// with the Proving ground of its `[proving_ground]` table on the Journey's
// own seed, and none without one; neither `GOODFETCH_TEST_SEED` nor a
// `provingGroundSeed` set around the call changes it. The engine executable
// is a value: the test tool manifest's of the Product, or `engine` for a
// skew run (ADR 0066).
//
// The body's `step`, `check` and `expectTask` record by the Rust helper's
// rules (`JourneyRun`); a step pinned `on` another surface drives that
// surface in its body, the CLI through `cli(args)`, the executable run in
// the variant's home and cleared environment. Once the body ends, the test
// fails unless the run is `proved`. Whatever happened, the run's facts are
// attached to the test, and test support's `ResultDocumentReporter` writes the
// variant's `journey` document from them.
import type { BrowserContext, Page, TestInfo } from "@playwright/test";
import { browserRuntime, hostPlatform } from "./case-log.ts";
import { daemonCommand, reportedVersion, type CliOutput } from "./daemon-command.ts";
import { TestSupportError } from "./errors.ts";
import type { OracleAnswer } from "./generated/result-document.ts";
import { journeyDocument } from "./journey-document.ts";
import { JOURNEY_CATALOG, readJourneyFile, type JourneyFile } from "./journey-file.ts";
import { JourneyRun, type JourneyBody, type TaskSubmission } from "./journey-run.ts";
import {
  test as served,
  type PageErrorWatch,
  type ServedDaemon,
  type SignIn,
} from "./playwright.ts";
import { ProcessRecord } from "./process-record.ts";
import type { ProvingGroundDescription } from "./proving-ground.ts";
import { JOURNEY_FACTS } from "./result-document-reporter.ts";

export type { CliOutput } from "./daemon-command.ts";

/** What a variant's body takes. */
export interface JourneyContext {
  /** Runs the step the Journey words `wording`, when it is the next one, and records its outcome. */
  readonly step: (wording: string, body: JourneyBody) => Promise<void>;
  /** Runs the check the Journey words `wording`, one that names no task, and records its outcome. */
  readonly check: (wording: string, body: JourneyBody) => Promise<void>;
  /** Submits the tool's result for the check the Journey words `wording`, one that names a task, to the oracle, and records that check from its answer. */
  readonly expectTask: (
    wording: string,
    submission: TaskSubmission,
  ) => Promise<OracleAnswer | undefined>;
  /** Runs the variant's executable with `args`, in its home and cleared environment, as a step pinned `on = "cli"` does. */
  readonly cli: (args: readonly string[]) => Promise<CliOutput>;
  /** The variant's Daemon and the Console it serves. */
  readonly daemon: ServedDaemon;
  /** The Proving ground started on the Journey's seed, for a Journey with a `[proving_ground]` table. */
  readonly provingGround: ProvingGroundDescription | undefined;
  readonly page: Page;
  readonly context: BrowserContext;
  readonly signIn: SignIn;
  /** What the variant's pages did wrong, for a step that expects a refusal, such as a page reconnecting to a stopped Daemon. */
  readonly failOnErrors: PageErrorWatch;
}

/** How a variant runs, beyond its Journey and surface. */
export interface JourneyOptions {
  /** The Journey file, for a Journey outside the catalog, such as test support's own tests'; `journeys/<id>.toml` otherwise. */
  readonly file?: string | undefined;
  /** The executable the variant runs, in place of the test tool manifest's of the Journey's Product, for a skew run. */
  readonly engine?: string | undefined;
  /** Whether its Console keeps one port across its Daemon's restarts, as the `namedConsolePort` fixture option says. */
  readonly namedConsolePort?: boolean | undefined;
}

/** The variant a test runs: its Journey and surface. */
interface Variant {
  readonly journey: JourneyFile;
  readonly surface: string;
}

/** A variant's run and the commands it runs. */
interface VariantRun {
  readonly run: JourneyRun;
  readonly cli: (args: readonly string[]) => Promise<CliOutput>;
}

const variantTest = served.extend<{ journeyVariant: Variant | undefined; variantRun: VariantRun }>({
  journeyVariant: [undefined, { option: true }],
  variantRun: async ({ journeyVariant, daemon, browser, browserName }, use, testInfo) => {
    if (journeyVariant === undefined) {
      throw new Error("invariant: `journey` sets the variant of every test it declares");
    }
    const processes = new ProcessRecord();
    const cli = (args: readonly string[]): Promise<CliOutput> =>
      daemonCommand(processes, daemon, args);
    try {
      const run = new JourneyRun({
        journey: journeyVariant.journey,
        surface: journeyVariant.surface,
        executable: await reportedVersion(cli, daemon.executable),
        platform: hostPlatform(),
        runtime: browserRuntime(browserName, browser.version()),
        provingGround: daemon.provingGround,
      });
      try {
        await use({ run, cli });
      } finally {
        await attachFacts(testInfo, run);
      }
    } finally {
      await processes.close();
    }
  },
});

/**
 * Declares the test of the variant of Journey `id` on `surface`, whose
 * `body` takes its steps and checks.
 *
 * @throws {TestSupportError} `journey_invalid` when the Journey file cannot
 * be read or fails its schema, `other_journey` when it holds another
 * Journey, and `unlisted_surface` when the Journey has no variant on
 * `surface`.
 */
export function journey(
  id: string,
  surface: string,
  body: (context: JourneyContext) => Promise<void>,
  options: JourneyOptions = {},
): void {
  const file = options.file ?? new URL(`${id}.toml`, JOURNEY_CATALOG);
  const read = readJourneyFile(file);
  if (read.id !== id) {
    throw new TestSupportError(
      "other_journey",
      `the Journey file ${String(file)} holds Journey ${read.id}, not ${id}`,
    );
  }
  if (!(surface in read.surfaces)) {
    throw new TestSupportError("unlisted_surface", `Journey ${id} lists no surface \`${surface}\``);
  }
  const variant = `${id}@${surface}`;
  variantTest.describe(variant, () => {
    variantTest.use({
      journeyVariant: { journey: read, surface },
      product: read.product,
      // The Journey's own seed, never the test's (ADR 0063); `null`, not
      // `undefined`, for none, which would take the project's seed.
      provingGroundSeed: read.proving_ground?.seed ?? null,
      ...(options.engine === undefined ? {} : { engine: options.engine }),
      ...(options.namedConsolePort === true ? { namedConsolePort: true } : {}),
    });
    variantTest(
      read.title,
      { tag: [`@${id}`, `@${surface}`], annotation: { type: "journey", description: variant } },
      async ({ variantRun, daemon, page, context, signIn, failOnErrors }) => {
        const { run, cli } = variantRun;
        await body({
          step: (wording, stepBody) => run.step(wording, () => served.step(wording, stepBody)),
          check: (wording, checkBody) => run.check(wording, () => served.step(wording, checkBody)),
          expectTask: (wording, submission) => run.expectTask(wording, submission),
          cli,
          daemon,
          provingGround: daemon.provingGround,
          page,
          context,
          signIn,
          failOnErrors,
        });
        const facts = run.facts();
        const outcome = journeyDocument(facts).outcome;
        if (outcome !== "proved" || run.failures.length > 0) {
          throw new TestSupportError(
            "variant_not_proved",
            [`Journey variant ${variant} is \`${outcome}\`, not \`proved\`:`]
              .concat(run.failures.map((failure) => `- ${failure}`))
              .join("\n"),
          );
        }
      },
    );
  });
}

/** Hands the run's facts to the reporter, once the test has ended however it did. */
async function attachFacts(testInfo: TestInfo, run: JourneyRun): Promise<void> {
  await testInfo.attach(JOURNEY_FACTS, {
    body: JSON.stringify(run.facts()),
    contentType: "application/json",
  });
}
