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
// attached to the test, and test support's `JourneyReporter` writes the
// variant's `journey` document from them.
import type { BrowserContext, Page, TestInfo } from "@playwright/test";
import { hostPlatform } from "./case-log.ts";
import { TestSupportError } from "./errors.ts";
import type { OracleAnswer } from "./generated/result-document.ts";
import { journeyDocument, type JourneyFacts } from "./journey-document.ts";
import { JOURNEY_CATALOG, readJourneyFile, type JourneyFile } from "./journey-file.ts";
import { JOURNEY_FACTS } from "./journey-reporter.ts";
import { JourneyRun, type JourneyBody, type TaskSubmission } from "./journey-run.ts";
import { test as served, type ServedDaemon, type SignIn } from "./playwright.ts";
import { ProcessRecord } from "./process-record.ts";
import type { ProvingGroundDescription } from "./proving-ground.ts";

/** What a command of the executable gave. */
export interface CliOutput {
  readonly code: number | null;
  readonly stdout: string;
  readonly stderr: string;
}

/** What a variant's body takes. */
export interface JourneyContext {
  /** Runs the step the Journey words `wording`, when it is the next one, and records its outcome. */
  readonly step: (wording: string, body: JourneyBody) => Promise<void>;
  /** Runs the check the Journey words `wording`, one that names no task, and records its outcome. */
  readonly check: (wording: string, body: JourneyBody) => Promise<void>;
  /** Submits the tool's result for a task the Journey names to the oracle, and records its answer and the checks naming it. */
  readonly expectTask: (
    task: string,
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
}

/** How a variant runs, beyond its Journey and surface. */
export interface JourneyOptions {
  /** The Journey file, for a Journey outside the catalog, such as test support's own tests'; `journeys/<id>.toml` otherwise. */
  readonly file?: string | undefined;
  /** The executable the variant runs, in place of the test tool manifest's of the Journey's Product, for a skew run. */
  readonly engine?: string | undefined;
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
    const cli = (args: readonly string[]): Promise<CliOutput> => command(processes, daemon, args);
    try {
      const run = new JourneyRun({
        journey: journeyVariant.journey,
        surface: journeyVariant.surface,
        executable: await reportedVersion(cli, daemon.executable),
        platform: hostPlatform(),
        runtime: `${browserName}-${browser.version().split(".")[0] ?? ""}`,
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
      // The Journey's own seed, never the test's (ADR 0063).
      provingGroundSeed: read.proving_ground?.seed ?? undefined,
      ...(options.engine === undefined ? {} : { engine: options.engine }),
    });
    variantTest(
      read.title,
      { tag: [`@${id}`, `@${surface}`], annotation: { type: "journey", description: variant } },
      async ({ variantRun, daemon, page, context, signIn }) => {
        const { run, cli } = variantRun;
        await body({
          step: (wording, stepBody) => run.step(wording, () => served.step(wording, stepBody)),
          check: (wording, checkBody) => run.check(wording, () => served.step(wording, checkBody)),
          expectTask: (task, submission) => run.expectTask(task, submission),
          cli,
          daemon,
          provingGround: daemon.provingGround,
          page,
          context,
          signIn,
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

/** Runs the Daemon's executable with `args` in the test's home and cleared environment. */
async function command(
  processes: ProcessRecord,
  daemon: ServedDaemon,
  args: readonly string[],
): Promise<CliOutput> {
  const started = processes.spawn(daemon.executable, args, { env: daemon.env });
  started.child.stdin.end();
  let stdout = "";
  let stderr = "";
  started.child.stdout.setEncoding("utf8");
  started.child.stderr.setEncoding("utf8");
  started.child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  started.child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });
  const { code } = await started.exit;
  return { code, stdout, stderr };
}

/** The Product and Product version the executable reports through `--version`, `goodfetch-<product> <version>`. */
async function reportedVersion(
  cli: (args: readonly string[]) => Promise<CliOutput>,
  executable: string,
): Promise<JourneyFacts["executable"]> {
  const output = await cli(["--version"]);
  const [binary, version] = output.stdout.trim().split(" ");
  const product = binary?.replace(/^goodfetch-/, "");
  if (
    output.code !== 0 ||
    version === undefined ||
    (product !== "collie" && product !== "foxhound")
  ) {
    throw new Error(
      `the executable ${executable} reports no \`goodfetch-<product> <version>\` through \`--version\`: \`${output.stdout}\``,
    );
  }
  return { product, version };
}
