// One variant run of a Journey (ADR 0063), as the TypeScript Journey helpers
// record it, by the rules of the Rust helper, `goodfetch-test-support`'s
// `JourneyRun`:
// - steps run in the file's order and by its wording; a `manual` step is
//   recorded `manual` without running, whenever the run passes it;
// - a step called out of order, twice, or with a wording the file does not
//   hold is recorded `failed` without running;
// - after the first failure, nothing more runs: every later step and check
//   is `not_reached`, and no result goes to the oracle;
// - at the end, each step never called is recorded, after those called, as
//   `not_reached` (`manual` for a manual one), so a run that skips a step
//   gives a document every reader refuses; a step or check still running is
//   recorded `failed`;
// - a check the file scopes to other variants' surfaces is not the
//   variant's: it is neither run nor written;
// - checks and tasks are written in the file's order, a check never called
//   `not_reached` and a task never judged with no answer;
// - each check naming a task is judged on its own submission, so a task two
//   checks name is written once per check, with that check's answer.
// It records facts only: the document's outcome is derived from them by
// `journeyDocument`.
import type { JourneyFacts } from "./journey-document.ts";
import { belongsTo, namedTasks, type JourneyFile } from "./journey-file.ts";
import type { CheckOutcome, OracleAnswer, StepOutcome } from "./generated/result-document.ts";

/** A step's or check's body: it fails when it throws or rejects. */
export type JourneyBody = () => unknown;

/** What the oracle is sent for a task, beside the task's ID: the tool's `status`, its `records` and any `files`. */
export type TaskSubmission = Readonly<Record<string, unknown>>;

/** What a variant run starts from. */
export interface JourneyRunStart {
  readonly journey: JourneyFile;
  /** The variant's surface, such as `console`. */
  readonly surface: string;
  /** The executable the variant drives, with the Product and version it reports. */
  readonly executable: JourneyFacts["executable"];
  readonly platform: JourneyFacts["platform"];
  /** The runtime, such as `chromium-141`. */
  readonly runtime: string;
  /** The Proving ground started on the Journey's seed, for a Journey with a `[proving_ground]` table. */
  readonly provingGround:
    | { readonly address: string; readonly version: string; readonly seed: number }
    | undefined;
}

interface Recorded {
  readonly text: string;
  readonly task?: string;
  readonly outcome: StepOutcome;
}

interface RecordedCheck {
  readonly text: string;
  readonly task?: string;
  readonly outcome: CheckOutcome;
}

/** The step or check whose body is running. */
interface Running {
  readonly kind: "step" | "check";
  readonly wording: string;
  readonly index: number;
}

/** One variant run, from its start to the facts of its document. */
export class JourneyRun {
  readonly #start: JourneyRunStart;
  readonly #steps: Recorded[] = [];
  /** Which of the file's steps are recorded or running. */
  readonly #taken: boolean[];
  /** The file's checks, each once recorded. */
  readonly #checks: (RecordedCheck | undefined)[];
  /** Checks recorded that the file does not hold. */
  readonly #extraChecks: RecordedCheck[] = [];
  /** The oracle's answer to each check judged, by the check's place in the file. */
  readonly #answers = new Map<number, OracleAnswer>();
  readonly #failures: string[] = [];
  #running: Running | undefined;
  #facts: JourneyFacts | undefined;

  constructor(start: JourneyRunStart) {
    // The variant's own checks: those scoped to other variants' surfaces
    // are neither run nor recorded.
    const expect = (start.journey.expect ?? []).filter((check) => belongsTo(check, start.surface));
    this.#start = { ...start, journey: { ...start.journey, expect } };
    this.#taken = start.journey.steps.map(() => false);
    this.#checks = expect.map(() => undefined);
  }

  /** Each failure the run recorded: a failed step or check, an answer that is not `verified`, or a step, check or task the Journey does not hold in that place. */
  get failures(): readonly string[] {
    return this.#failures;
  }

  /**
   * Runs the step the file words `wording`, when it is the next one and
   * nothing failed before it, and records its outcome: `failed` when `body`
   * throws.
   */
  async step(wording: string, body: JourneyBody): Promise<void> {
    if (this.#facts !== undefined) {
      return;
    }
    const steps = this.#start.journey.steps;
    const found = this.#untakenStep(wording);
    const manualReached =
      found !== undefined &&
      steps.slice(0, found + 1).every((step, index) => this.#taken[index] || isManual(step));
    this.#takeManualSteps();
    if (manualReached) {
      return;
    }
    const next = this.#taken.indexOf(false);
    let misuse: string | undefined;
    if (found !== undefined && found !== next) {
      this.#taken[found] = true;
      misuse =
        next === -1
          ? "is called after every step"
          : `is called before step ${next + 1} \`${steps[next]?.text}\``;
    } else if (found === undefined) {
      misuse = steps.some((step) => step.text === wording)
        ? "is called a second time"
        : `is not a step of ${this.#start.journey.id}`;
    }
    if (misuse !== undefined || found === undefined) {
      this.#failures.push(`step \`${wording}\` ${misuse}`);
      this.#steps.push({ text: wording, outcome: "failed" });
      return;
    }
    this.#taken[found] = true;
    if (this.#failures.length > 0) {
      this.#steps.push({ text: wording, outcome: "not_reached" });
      return;
    }
    const failure = await this.#runBody({ kind: "step", wording, index: found }, body);
    if (this.#facts !== undefined) {
      return;
    }
    if (failure === undefined) {
      this.#steps.push({ text: wording, outcome: "passed" });
    } else {
      this.#failures.push(`step ${found + 1} \`${wording}\` failed: ${failure}`);
      this.#steps.push({ text: wording, outcome: "failed" });
    }
  }

  /**
   * Runs the check the file words `wording`, one that names no task, when
   * nothing failed before it, and records its outcome: `failed` when `body`
   * throws. A check that names a task is recorded by `expectTask`.
   */
  async check(wording: string, body: JourneyBody): Promise<void> {
    if (this.#facts !== undefined) {
      return;
    }
    const checks = this.#start.journey.expect ?? [];
    const index = checks.findIndex((check) => check.text === wording && isUnset(check.task));
    if (index === -1 || this.#checks[index] !== undefined) {
      const misuse =
        index === -1
          ? `is not a check of ${this.#start.journey.id} that names no task`
          : "is called a second time";
      this.#failures.push(`check \`${wording}\` ${misuse}`);
      this.#extraChecks.push({ text: wording, outcome: "failed" });
      return;
    }
    if (this.#failures.length > 0) {
      this.#checks[index] = { text: wording, outcome: "not_reached" };
      return;
    }
    const failure = await this.#runBody({ kind: "check", wording, index }, body);
    if (this.#facts !== undefined) {
      return;
    }
    if (failure !== undefined) {
      this.#failures.push(`check \`${wording}\` failed: ${failure}`);
    }
    this.#checks[index] = { text: wording, outcome: failure === undefined ? "passed" : "failed" };
  }

  /**
   * Submits the tool's result for the check the file words `wording`, one
   * that names a task, to its Proving ground's oracle for that task, when
   * nothing failed before, and records the check as `passed` when the answer
   * is `verified` and `failed` otherwise. Each check naming a task is judged
   * on its own submission, so two checks naming one task take two answers.
   * Returns the answer.
   */
  async expectTask(wording: string, submission: TaskSubmission): Promise<OracleAnswer | undefined> {
    if (this.#facts !== undefined) {
      return undefined;
    }
    const journey = this.#start.journey;
    const index = (journey.expect ?? []).findIndex(
      (check) => check.text === wording && !isUnset(check.task),
    );
    const task = journey.expect?.[index]?.task;
    if (index === -1 || isUnset(task) || this.#checks[index] !== undefined) {
      const misuse =
        index === -1
          ? `is not a check of ${journey.id} that names a task`
          : "is judged a second time";
      this.#failures.push(`check \`${wording}\` ${misuse}`);
      this.#extraChecks.push({ text: wording, outcome: "failed" });
      return undefined;
    }
    if (this.#failures.length > 0) {
      return undefined;
    }
    let answer: OracleAnswer;
    try {
      answer = await this.#judge(task, submission);
    } catch (error) {
      this.#failures.push(`check \`${wording}\`, task \`${task}\`: ${message(error)}`);
      return undefined;
    }
    const verified = answer.outcome === "verified";
    if (!verified) {
      this.#failures.push(
        `the oracle answers \`${answer.outcome}\` for task \`${task}\` of check \`${wording}\``,
      );
    }
    this.#checks[index] = { text: wording, task, outcome: verified ? "passed" : "failed" };
    this.#answers.set(index, answer);
    return answer;
  }

  /**
   * Ends the run and gives what it recorded, in the result document's field
   * names: each step never called is recorded after those called, and a
   * step or check still running is `failed`. Later calls give the same
   * facts, and the run records nothing more.
   */
  facts(): JourneyFacts {
    this.#facts ??= this.#finish();
    return this.#facts;
  }

  #finish(): JourneyFacts {
    const { journey, surface, executable, platform, runtime, provingGround } = this.#start;
    const running = this.#running;
    if (running !== undefined) {
      this.#failures.push(`${running.kind} \`${running.wording}\` did not finish`);
      if (running.kind === "step") {
        this.#steps.push({ text: running.wording, outcome: "failed" });
      } else {
        this.#checks[running.index] = { text: running.wording, outcome: "failed" };
      }
    }
    this.#takeManualSteps();
    journey.steps.forEach((step, index) => {
      if (!this.#taken[index]) {
        this.#taken[index] = true;
        this.#steps.push({ text: step.text, outcome: isManual(step) ? "manual" : "not_reached" });
      }
    });
    const checks = (journey.expect ?? []).map(
      (check, index): RecordedCheck =>
        this.#checks[index] ?? {
          text: check.text,
          ...(isUnset(check.task) ? {} : { task: check.task }),
          outcome: "not_reached",
        },
    );
    // Each named task, once per check naming it with that check's answer in
    // the checks' order, or once when no check names it.
    const tasks = namedTasks(journey).flatMap((id) => {
      const judged = (journey.expect ?? []).flatMap((check, index) => {
        if (check.task !== id) {
          return [];
        }
        const answer = this.#answers.get(index);
        return [answer === undefined ? { id } : { id, answer }];
      });
      return judged.length === 0 ? [{ id }] : judged;
    });
    return {
      journey_id: journey.id,
      surface,
      executable,
      platform,
      runtime,
      ...(provingGround === undefined
        ? {}
        : { proving_ground: { version: provingGround.version, seed: provingGround.seed } }),
      steps: [...this.#steps],
      checks: [...checks, ...this.#extraChecks],
      tasks,
    };
  }

  /** The first step of the file worded `wording` not yet taken. */
  #untakenStep(wording: string): number | undefined {
    const index = this.#start.journey.steps.findIndex(
      (step, at) => step.text === wording && !this.#taken[at],
    );
    return index === -1 ? undefined : index;
  }

  /** Records each manual step the run has reached: those before the next automated step not yet taken. */
  #takeManualSteps(): void {
    const steps = this.#start.journey.steps;
    for (let index = this.#taken.indexOf(false); index !== -1; index = this.#taken.indexOf(false)) {
      const step = steps[index];
      if (step === undefined || !isManual(step)) {
        break;
      }
      this.#taken[index] = true;
      this.#steps.push({ text: step.text, outcome: "manual" });
    }
  }

  async #runBody(running: Running, body: JourneyBody): Promise<string | undefined> {
    this.#running = running;
    try {
      await body();
      return undefined;
    } catch (error) {
      return message(error);
    } finally {
      if (this.#running === running) {
        this.#running = undefined;
      }
    }
  }

  async #judge(task: string, submission: TaskSubmission): Promise<OracleAnswer> {
    const provingGround = this.#start.provingGround;
    if (provingGround === undefined) {
      throw new Error("the Journey starts no Proving ground");
    }
    const response = await fetch(`http://${provingGround.address}/oracle`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ task, ...submission }),
    });
    const text = await response.text();
    if (response.status !== 200) {
      throw new Error(`the oracle answers ${response.status}: ${text}`);
    }
    const answer: OracleAnswer = JSON.parse(text);
    return answer;
  }
}

function isManual(step: { readonly manual?: string | null | undefined }): boolean {
  return !isUnset(step.manual);
}

function isUnset(value: string | null | undefined): value is null | undefined {
  return value === undefined || value === null;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
