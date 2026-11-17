// The TypeScript Journey helper (ADR 0063; TESTING.md, Journeys): one variant
// run of a Journey of `journeys/`, from its start to the `journey` result
// document it writes, by the rules of the Rust `JourneyRun`. A run reads the
// Journey file, validated against `journeys/journey.schema.json`, starts the
// test's home through `goodfetch-test-env` with the Journey's own Proving
// ground seed (never the test's), asks the Product's executable for its
// version, records each step and check in the file's order and wording, and
// writes the document through `writeJourneyDocument`. The helper declares
// no Feature: a Journey test's Features are its Journey's `covers`.
import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { ValidateFunction } from "ajv/dist/2020.js";
import { parse } from "smol-toml";
import { hostPlatform, hostRuntime } from "./case-log.ts";
import { TestSupportError } from "./errors.ts";
import { test } from "./fixtures.ts";
import type {
  CheckResult,
  JourneyOutcome,
  OracleAnswer,
  StepResult,
  TaskResult as DocumentTask,
} from "./generated/result-document.ts";
import {
  journeyDocument,
  writeJourneyDocument,
  type JourneyFacts,
  type JourneyResultDocument,
} from "./journey-document.ts";
import type { ProcessRecord, RecordedProcess } from "./process-record.ts";
import { productEnv, type Product } from "./product-env.ts";
import { judgeTask, type ProvingGroundDescription, type TaskResult } from "./proving-ground.ts";
import { ajv } from "./schemas.ts";
import { startTestEnv, toolPath, type TestEnvDescription } from "./test-env.ts";

/** The catalog's folder, `journeys/` at the repository's root. */
const CATALOG = new URL("../../../journeys/", import.meta.url);

/** The folder each variant run also writes its document to, which `just test` sets. */
const RESULT_DOCUMENTS_VARIABLE = "GOODFETCH_TEST_RESULT_DOCUMENTS";

/** What the helper reads of a Journey file; the schema holds the rest. */
interface JourneyFile {
  readonly id: string;
  readonly product: Product;
  readonly proving_ground?: {
    readonly seed: number;
    readonly tasks?: readonly { readonly id: string }[];
  } | null;
  readonly surfaces: Readonly<Record<string, string>>;
  readonly steps: readonly { readonly text: string; readonly manual?: string | null }[];
  readonly expect: readonly JourneyCheck[];
}

interface JourneyCheck {
  readonly text: string;
  readonly task?: string | null;
  /** The surfaces of the variants the check belongs to; every variant's when absent or empty. */
  readonly surfaces?: readonly string[];
}

const isJourneyFile: ValidateFunction<JourneyFile> = ajv.compile<JourneyFile>(
  JSON.parse(readFileSync(new URL("journey.schema.json", CATALOG), "utf8")),
);

/** How a variant run starts. */
export interface JourneyRunStart {
  /** The Journey file: a catalog file, or a fixture in the helper's own tests. */
  readonly file: string;
  /** The surface of the variant, one the Journey lists, such as `sdk-typescript`. */
  readonly surface: string;
  /** The record the run's processes join. */
  readonly processes: ProcessRecord;
  /** The test's full name, from which its folder name comes. */
  readonly testName: string;
  /** The test tool manifest's path, whose entry for the Journey's Product is the executable driven. */
  readonly toolManifest: string | undefined;
  /** The environment the run's setup starts with, as `startTestEnv`'s `env`. */
  readonly env?: Readonly<Record<string, string | undefined>> | undefined;
}

/** A tool's result for one check's task, without the task: what the oracle judges. */
export type TaskSubmission = Omit<TaskResult, "task">;

/**
 * One variant run of a Journey. A test awaits `step` for each step of the
 * file, in order, then `check` and `expectTask` for its checks, then
 * `finish`, and `close` tears the home down. `journey()` does all but the
 * steps and checks.
 */
export interface JourneyRun {
  /** The Journey's ID. */
  readonly id: string;
  /** The variant's surface. */
  readonly surface: string;
  /** The Journey's Product. */
  readonly product: Product;
  /** The executable the variant drives: the tool manifest's entry for the Product. */
  readonly engine: string;
  /** The test's home, as `goodfetch-test-env` describes it. */
  readonly testEnv: TestEnvDescription;
  /** The home's environment with the Product's engine path set: what an SDK of the Product runs under. */
  readonly env: Readonly<Record<string, string | undefined>>;
  /** The Proving ground started on the Journey's seed, for a Journey with a `[proving_ground]` table. */
  readonly provingGround: ProvingGroundDescription | undefined;
  /** Runs the executable with `args` in the home: how a step pinned `on = "cli"` runs in any variant. */
  command(args: readonly string[]): RecordedProcess;
  /** Records the surface's package, such as an `sdk-typescript` variant's npm package and its version. */
  surfacePackage(name: string, version: string): void;
  /**
   * Runs the step the file words `wording`, when it is the next one and
   * nothing failed before it, and records its outcome: `failed` when `body`
   * throws or rejects. Gives what `body` returned, or nothing when it did
   * not run or failed.
   */
  step<T>(wording: string, body: () => T | Promise<T>): Promise<T | undefined>;
  /**
   * Runs the check the file words `wording`, one that names no task, when
   * nothing failed before it, and records its outcome: `failed` when `body`
   * throws or rejects.
   */
  check(wording: string, body: () => unknown): Promise<void>;
  /**
   * Submits the tool's result for the check the file words `check`, one that
   * names a task, to the Proving ground's oracle for that task, when nothing
   * failed before, and records the check `passed` when the answer is
   * `verified` and `failed` otherwise. Each check naming a task is judged on
   * its own submission. Gives the answer.
   */
  expectTask(check: string, submission: TaskSubmission): Promise<OracleAnswer | undefined>;
  /**
   * Records the steps and checks not reached and writes the document as
   * `<id>@<surface>.json` into the test's folder, and into the folder
   * `GOODFETCH_TEST_RESULT_DOCUMENTS` names when it is set.
   */
  finish(): FinishedJourneyRun;
  /** Tears the test's home down, keeping its folder when `failed` is set. */
  close(outcome?: { readonly failed: boolean }): Promise<void>;
}

/** A finished run: the document written and where. */
export interface FinishedJourneyRun {
  /** Where the document was written in the test's folder. */
  readonly path: string;
  /** The document written. */
  readonly document: JourneyResultDocument;
  /** The variant's outcome as its document states it. */
  readonly outcome: JourneyOutcome;
  /** Each failure the run recorded: a failed step or check, an answer that is not `verified`, or a misuse. */
  readonly failures: readonly string[];
  /**
   * @throws {TestSupportError} `variant_not_proved` unless the document states
   * `proved` and the run recorded no failure.
   */
  assertProved(): void;
}

/**
 * Starts the variant on `start.surface` of the Journey `start.file` holds.
 *
 * @throws {TestSupportError} `journey_unreadable` or `journey_invalid` for a
 * file that is not a Journey, `surface_unlisted` when it lists no such
 * surface, and `engine_version` when the executable does not report its
 * Product and version; and as `startTestEnv`.
 */
export async function startJourneyRun(start: JourneyRunStart): Promise<JourneyRun> {
  const parsed = readJourney(start.file);
  // The variant's own checks: those scoped to other variants' surfaces are
  // not run, and not recorded.
  const variant: JourneyFile = {
    ...parsed,
    expect: parsed.expect.filter(
      (check) => (check.surfaces ?? []).length === 0 || check.surfaces?.includes(start.surface),
    ),
  };
  if (!Object.hasOwn(variant.surfaces, start.surface)) {
    throw new TestSupportError(
      "surface_unlisted",
      `${variant.id} lists no surface \`${start.surface}\`: it lists ${Object.keys(variant.surfaces).join(", ")}`,
    );
  }
  const started = await startTestEnv({
    processes: start.processes,
    testName: start.testName,
    toolManifest: start.toolManifest,
    env: start.env,
    // The Journey's own seed, never the test's (ADR 0063).
    provingGroundSeed: variant.proving_ground?.seed,
  });
  try {
    const testEnv = started.description;
    const engine = toolPath(testEnv.tools, variant.product);
    const command = (args: readonly string[]): RecordedProcess =>
      start.processes.spawn(engine, args, { env: testEnv.env, cwd: testEnv.folder });
    const executable = await reportedVersion(command, engine);
    return new Run({
      journey: variant,
      surface: start.surface,
      engine,
      testEnv,
      executable,
      command,
      close: (outcome) => started.close(outcome),
    });
  } catch (error) {
    await started.close({ failed: true });
    throw error;
  }
}

/**
 * Declares the Vitest test of the variant of `journeys/<id>.toml` on
 * `surface`, tagged with both: it starts the run, gives it to `body` to take
 * the steps and checks, finishes it and fails unless it is proved. A throw
 * outside a step or check fails the test with no document, as the Rust
 * helper's panic does.
 */
export function journey(
  id: string,
  surface: string,
  body: (run: JourneyRun) => void | Promise<void>,
): void {
  test(`${id}@${surface}`, { tags: [id, surface] }, async ({ processes, toolManifest, task }) => {
    const run = await startJourneyRun({
      file: fileURLToPath(new URL(`${id}.toml`, CATALOG)),
      surface,
      processes,
      testName: task.fullName,
      toolManifest,
    });
    let failed = true;
    try {
      if (run.id !== id) {
        throw new TestSupportError(
          "journey_other",
          `journeys/${id}.toml holds ${run.id}, not ${id}`,
        );
      }
      await body(run);
      run.finish().assertProved();
      failed = false;
    } finally {
      await run.close({ failed });
    }
  });
}

function readJourney(file: string): JourneyFile {
  let text: string;
  try {
    text = readFileSync(file, "utf8");
  } catch (error) {
    throw new TestSupportError("journey_unreadable", `the Journey file ${file} cannot be read`, {
      cause: error,
    });
  }
  let value: unknown;
  try {
    value = parse(text);
  } catch (error) {
    throw new TestSupportError("journey_invalid", `the Journey file ${file} is not TOML`, {
      cause: error,
    });
  }
  if (!isJourneyFile(value)) {
    throw new TestSupportError(
      "journey_invalid",
      `the Journey file ${file} fails journeys/journey.schema.json: ${ajv.errorsText(isJourneyFile.errors)}`,
    );
  }
  return value;
}

/** The Product and Product version an executable reports through `--version`: `goodfetch-<product> <version>`. */
async function reportedVersion(
  command: (args: readonly string[]) => RecordedProcess,
  engine: string,
): Promise<{ product: Product; version: string }> {
  const asked = command(["--version"]);
  asked.child.stdin.end();
  asked.child.stderr.resume();
  let stdout = "";
  asked.child.stdout.setEncoding("utf8");
  asked.child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  const exit = await asked.exit;
  const match = /^goodfetch-(collie|foxhound) (\S+)$/.exec(stdout.trim());
  const product = match?.[1];
  const version = match?.[2];
  if (
    exit.code !== 0 ||
    version === undefined ||
    (product !== "collie" && product !== "foxhound")
  ) {
    throw new TestSupportError(
      "engine_version",
      `${engine} --version does not report its Product and version (code ${exit.code}): ${exit.error?.message ?? stdout}`,
      exit.error === undefined ? undefined : { cause: exit.error },
    );
  }
  return { product, version };
}

function failureOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

class Run implements JourneyRun {
  readonly id: string;
  readonly surface: string;
  readonly product: Product;
  readonly engine: string;
  readonly testEnv: TestEnvDescription;
  readonly env: Readonly<Record<string, string | undefined>>;
  readonly provingGround: ProvingGroundDescription | undefined;
  readonly command: (args: readonly string[]) => RecordedProcess;
  readonly close: (outcome?: { readonly failed: boolean }) => Promise<void>;
  readonly #journey: JourneyFile;
  readonly #executable: { readonly product: Product; readonly version: string };
  readonly #steps: (StepResult | undefined)[] = [];
  /** Which of the file's steps are recorded. */
  readonly #taken: boolean[];
  /** The file's checks, each once recorded. */
  readonly #checks: (CheckResult | undefined)[];
  /** Checks recorded that the file does not hold. */
  readonly #extraChecks: CheckResult[] = [];
  /** The oracle's answer to each check judged, by the check's place in the file. */
  readonly #answers = new Map<number, OracleAnswer>();
  readonly #failures: string[] = [];
  #surfacePackage: { name: string; version: string } | undefined;
  /** Steps, checks and judgements started and not settled. */
  #running = 0;

  constructor(start: {
    readonly journey: JourneyFile;
    readonly surface: string;
    readonly engine: string;
    readonly testEnv: TestEnvDescription;
    readonly executable: { readonly product: Product; readonly version: string };
    readonly command: (args: readonly string[]) => RecordedProcess;
    readonly close: (outcome?: { readonly failed: boolean }) => Promise<void>;
  }) {
    const { journey: file, testEnv } = start;
    this.id = file.id;
    this.surface = start.surface;
    this.engine = start.engine;
    this.testEnv = testEnv;
    this.command = start.command;
    this.close = start.close;
    this.product = file.product;
    this.provingGround = file.proving_ground == null ? undefined : testEnv.proving_ground;
    this.env = productEnv(testEnv, file.product);
    this.#journey = file;
    this.#executable = start.executable;
    this.#taken = file.steps.map(() => false);
    this.#checks = file.expect.map(() => undefined);
  }

  surfacePackage = (name: string, version: string): void => {
    this.#surfacePackage = { name, version };
  };

  step = async <T>(wording: string, body: () => T | Promise<T>): Promise<T | undefined> => {
    const steps = this.#journey.steps;
    const found = this.#untakenStep(wording);
    const manualReached =
      found !== undefined &&
      steps.slice(0, found + 1).every((step, index) => this.#taken[index] || step.manual != null);
    this.#takeManualSteps();
    if (manualReached) {
      // A manual step the run reached: recorded `manual` just above.
      return undefined;
    }
    const next = this.#taken.indexOf(false);
    let misuse: string | undefined;
    if (found === undefined) {
      misuse = steps.some((step) => step.text === wording)
        ? "is called a second time"
        : `is not a step of ${this.id}`;
    } else if (found !== next) {
      this.#taken[found] = true;
      misuse = `is called before step ${next + 1} \`${steps[next]?.text ?? ""}\``;
    }
    if (misuse !== undefined || found === undefined) {
      this.#fail(`step \`${wording}\` ${misuse ?? ""}`);
      this.#steps.push({ text: wording, outcome: "failed" });
      return undefined;
    }
    this.#taken[found] = true;
    if (this.#failed()) {
      this.#steps.push({ text: wording, outcome: "not_reached" });
      return undefined;
    }
    // The step's place is kept while its body runs, so the steps stay in
    // the order they were called.
    const place = this.#steps.push(undefined) - 1;
    this.#running += 1;
    try {
      const value = await body();
      this.#steps[place] = { text: wording, outcome: "passed" };
      return value;
    } catch (error) {
      this.#fail(`step ${found + 1} \`${wording}\` failed: ${failureOf(error)}`);
      this.#steps[place] = { text: wording, outcome: "failed" };
      return undefined;
    } finally {
      this.#running -= 1;
    }
  };

  check = async (wording: string, body: () => unknown): Promise<void> => {
    const index = this.#journey.expect.findIndex(
      (check) => check.text === wording && check.task == null,
    );
    if (index === -1 || this.#checks[index] !== undefined) {
      const misuse =
        index === -1
          ? `is not a check of ${this.id} that names no task`
          : "is called a second time";
      this.#fail(`check \`${wording}\` ${misuse}`);
      this.#extraChecks.push({ text: wording, outcome: "failed" });
      return;
    }
    if (this.#failed()) {
      this.#checks[index] = { text: wording, outcome: "not_reached" };
      return;
    }
    this.#checks[index] = { text: wording, outcome: "not_reached" };
    this.#running += 1;
    try {
      await body();
      this.#checks[index] = { text: wording, outcome: "passed" };
    } catch (error) {
      this.#fail(`check \`${wording}\` failed: ${failureOf(error)}`);
      this.#checks[index] = { text: wording, outcome: "failed" };
    } finally {
      this.#running -= 1;
    }
  };

  expectTask = async (
    check: string,
    submission: TaskSubmission,
  ): Promise<OracleAnswer | undefined> => {
    const index = this.#journey.expect.findIndex(
      (entry) => entry.text === check && entry.task != null,
    );
    const task = this.#journey.expect[index]?.task;
    if (task == null || this.#checks[index] !== undefined) {
      const misuse =
        task == null ? `is not a check of ${this.id} that names a task` : "is judged a second time";
      this.#fail(`check \`${check}\` ${misuse}`);
      this.#extraChecks.push({ text: check, outcome: "failed" });
      return undefined;
    }
    if (this.#failed()) {
      return undefined;
    }
    if (this.provingGround === undefined) {
      this.#fail(`check \`${check}\`, task \`${task}\`: the Journey starts no Proving ground`);
      return undefined;
    }
    this.#running += 1;
    let answer: OracleAnswer;
    try {
      answer = await judgeTask(this.provingGround, { ...submission, task });
    } catch (error) {
      this.#fail(`check \`${check}\`, task \`${task}\`: ${failureOf(error)}`);
      return undefined;
    } finally {
      this.#running -= 1;
    }
    const verified = answer.outcome === "verified";
    if (!verified) {
      this.#fail(
        `the oracle answers \`${answer.outcome}\` for task \`${task}\` of check \`${check}\``,
      );
    }
    this.#checks[index] = { text: check, task, outcome: verified ? "passed" : "failed" };
    this.#answers.set(index, answer);
    return answer;
  };

  finish = (): FinishedJourneyRun => {
    if (this.#running > 0) {
      this.#fail("the run finishes while a step or check still runs: await each one");
    }
    this.#takeManualSteps();
    this.#journey.steps.forEach((step, index) => {
      if (!this.#taken[index]) {
        this.#taken[index] = true;
        this.#steps.push({
          text: step.text,
          outcome: step.manual == null ? "not_reached" : "manual",
        });
      }
    });
    const facts = this.#facts();
    const variant = `${this.id}@${this.surface}`;
    const file = `${variant}.json`;
    const path = join(this.testEnv.folder, file);
    const outcome = writeJourneyDocument(path, facts);
    // The test's own copy is the one returned: in the suite's folder,
    // another test running the same variant may write after it.
    const suiteFolder = process.env[RESULT_DOCUMENTS_VARIABLE];
    if (suiteFolder !== undefined && suiteFolder !== "") {
      mkdirSync(suiteFolder, { recursive: true });
      writeJourneyDocument(join(suiteFolder, file), facts);
    }
    const failures = [...this.#failures];
    return {
      path,
      document: journeyDocument(facts),
      outcome,
      failures,
      assertProved: () => {
        if (outcome === "proved" && failures.length === 0) {
          return;
        }
        const listed = failures.map((failure) => `\n- ${failure}`).join("");
        throw new TestSupportError(
          "variant_not_proved",
          `Journey variant ${variant} is \`${outcome}\`, not \`proved\` (${path}):${listed}`,
        );
      },
    };
  };

  /** The first step of the file worded `wording` that is not recorded yet. */
  #untakenStep(wording: string): number | undefined {
    const index = this.#journey.steps.findIndex(
      (step, place) => step.text === wording && !this.#taken[place],
    );
    return index === -1 ? undefined : index;
  }

  /** Records each manual step the run has reached: those before the next automated step not yet recorded. */
  #takeManualSteps(): void {
    for (let index = this.#taken.indexOf(false); index !== -1; index = this.#taken.indexOf(false)) {
      const step = this.#journey.steps[index];
      if (step === undefined || step.manual == null) {
        return;
      }
      this.#taken[index] = true;
      this.#steps.push({ text: step.text, outcome: "manual" });
    }
  }

  #failed(): boolean {
    return this.#failures.length > 0;
  }

  #fail(failure: string): void {
    this.#failures.push(failure);
  }

  /** What the run recorded, in the result document's field names. */
  #facts(): JourneyFacts {
    const expect = this.#journey.expect;
    const checks: CheckResult[] = [
      ...expect.map(
        (check, index): CheckResult =>
          this.#checks[index] ?? {
            text: check.text,
            ...(check.task == null ? {} : { task: check.task }),
            outcome: "not_reached",
          },
      ),
      ...this.#extraChecks,
    ];
    // Each named task, once per check naming it with that check's answer
    // in the checks' order, or once when no check names it.
    const tasks: DocumentTask[] = (this.#journey.proving_ground?.tasks ?? []).flatMap(({ id }) => {
      const judged = expect.flatMap((check, index): DocumentTask[] => {
        if (check.task !== id) {
          return [];
        }
        const answer = this.#answers.get(index);
        return [answer === undefined ? { id } : { id, answer }];
      });
      return judged.length === 0 ? [{ id }] : judged;
    });
    const steps = this.#steps.flatMap((step) => (step === undefined ? [] : [step]));
    const provingGround = this.provingGround;
    return {
      journey_id: this.id,
      surface: this.surface,
      executable: { ...this.#executable },
      platform: hostPlatform(),
      runtime: hostRuntime(),
      steps,
      checks,
      tasks,
      ...(this.#surfacePackage === undefined ? {} : { surface_package: this.#surfacePackage }),
      ...(provingGround === undefined
        ? {}
        : { proving_ground: { version: provingGround.version, seed: provingGround.seed } }),
    };
  }
}
