// Collie's AI verbs and Plans (ADR 0091, decided in #473).
// - `act`, `observe` and `extract` live on the page, and on a locator, whose one element bounds what
//   the model sees and what a step may target. There is no AI runtime object and no active page (G2).
// - Each call is one step of the Run's Plan (ADR 0009): replayed without a model when the Plan holds
//   it and its Target fingerprint still matches, compiled by the model when it is missing, repaired
//   once on Drift. Starting a Run with `plan` names the file; `plan.frozen` forbids any model call.
// - Every verb returns a `StepResult`: its `output`, how the `step` was resolved, and its `usage`.
//   Failures throw (ADR 0079); single steps carry no Verdict, which the Run's checks give (ADR 0029).
// - `expect` runs a Success check under Playwright's assertion names, evaluated live by the engine
//   and read by the Run's Verdict; a condition still false at its timeout throws.
// - `ariaSnapshot` and `getByRef` are the observation an agent loop reads, under Playwright's names.
// Model references and `ModelUsage` are ADR 0101's (models.d.ts). Secret placeholders in instructions are ADR 0092's.

import type { JsonValue, Schema, SchemaOutput } from "./data.js";
import type { Page } from "./lifecycle.js";
import type { ModelReference, ModelUsage, StructuredOutputStrategy } from "./models.js";
import type { Locator, Response } from "./page.js";
import type { ModelError, PageError, ProgramError, RefusedError } from "./results.js";
import type { CallOptions, Count } from "./time.js";

// Results.

/**
 * How a step was resolved. `plan`: replayed from the Plan, no model call. `model`: compiled now,
 * since the Plan held no entry for it. `repair`: its entry drifted and one Repair recompiled it.
 * `journal`: the Effect journal already held this step's Effect, so nothing was done again (ADR 0010).
 */
export type StepSource = "plan" | "model" | "repair" | "journal" | (string & {});

/** What drifted, when a step was repaired: what the Plan expected and what the page showed. */
export interface Drift {
  /** The Target fingerprint the Plan kept, rendered for a person. */
  readonly expected: string;
  /** What stood there instead, or `null` when nothing matched. */
  readonly observed: string | null;
}

export interface StepReport {
  readonly source: StepSource;
  /** The instruction as the Plan keys it, with `%name%` placeholders and never their values. */
  readonly instruction: string;
  /** Present when `source` is `repair`. */
  readonly drift: Drift | undefined;
}

/** What every AI verb returns. A value of `output` is always complete; a failure throws. */
export interface StepResult<T> {
  readonly output: T;
  readonly step: StepReport;
  /** The model calls this step made and their price (ADR 0101): none for a step replayed from the Plan. */
  readonly usage: ModelUsage;
}

/** The action `act` performed: through the guard, like any locator action (ADR 0019). */
export interface PerformedAction {
  /** The page API method, such as `click` or `fill`. */
  readonly method: string;
  /** Its arguments, `%name%` placeholders where a variable was typed. */
  readonly arguments: readonly JsonValue[];
  /** The target, an ordinary locator resolved again at each use. */
  readonly target: Locator;
  /** The same call in this language, as the engine renders it: `await page.getByRole('link', { name: 'love' }).click();`. */
  readonly code: string;
}

/** One element `observe` found for its instruction. */
export interface Observed {
  readonly locator: Locator;
  /** The model's description of the element: a signal, never proof. */
  readonly description: string;
  /** The locator in this language, as the engine renders it: `page.getByRole('button', { name: 'Sign in' })`. */
  readonly code: string;
}

// Options.

/** Options every AI verb takes. Its timeout is `timeout.ai`, which nests the action's own. */
export interface AiCallOptions extends CallOptions {
  /** The model for this call alone: that setting at the run layer (ADR 0083). */
  readonly model?: ModelReference;
}

export interface ActOptions extends AiCallOptions {
  /**
   * Plain values for the instruction's `%name%` placeholders. A placeholder no variable fills is a
   * Secret placeholder, bound by the setting `secrets.<name>` (ADR 0092), never by this map. The
   * model, the Plan, logs and the Run trace see only `%name%`; the engine types the value at input
   * (ADR 0014). A placeholder neither fills, a variable no placeholder uses, or a name that is both
   * throws `VariablesMismatchError`.
   */
  readonly variables?: Readonly<Record<string, string>>;
}

/**
 * `compiled`: the model writes each field's locators and transform once, kept in the Plan, and
 * values are copied from the page at every call (ADR 0013). `model`: the model reads the page at
 * every call, every value checked by Grounding, nothing kept in the Plan.
 */
export type ExtractMode = "compiled" | "model";

export interface ExtractOptions extends AiCallOptions {
  /** `extract.mode` for this call alone. Default: `compiled`. */
  readonly mode?: ExtractMode;
}

/** The AI verbs, on a page and on a locator. */
export interface AiVerbs {
  /** Performs one action described in words. Throws `TargetNotFoundError` when nothing on the page fits it. */
  act(instruction: string, options?: ActOptions): Promise<StepResult<PerformedAction>>;
  /** Finds the elements an instruction describes, without acting. None is an answer: an empty list. */
  observe(instruction: string, options?: AiCallOptions): Promise<StepResult<Observed[]>>;
  /** Reads typed data. A required field not on the page throws `ExtractionIncompleteError`. */
  extract<S extends Schema>(instruction: string, schema: S, options?: ExtractOptions): Promise<StepResult<SchemaOutput<S>>>;
}

// Observations.

export interface AriaSnapshotOptions extends CallOptions {
  /**
   * `default`: Playwright's YAML, as `toMatchAriaSnapshot` compares it. `ai`: with element
   * references, page text marked untrusted (ADR 0014), and bounded by `observation.max_size`, past
   * which the deepest subtrees collapse into one line keeping their reference, and the output says so.
   */
  readonly mode?: "default" | "ai";
  readonly depth?: number;
  /** Adds each element's box, `[box=x,y,width,height]`. */
  readonly boxes?: boolean;
}

/** Playwright's accessibility snapshot. Secret values are masked and cookies never appear, in both modes. */
export interface AriaSnapshots {
  ariaSnapshot(options?: AriaSnapshotOptions): Promise<string>;
}

/** On a page: resolves an element reference from an `ai` snapshot. */
export interface ElementReferences {
  /**
   * The element an `ai` snapshot gave this reference, such as `e12` or `f1e3`. References last as
   * long as their element, whichever snapshot showed them; one whose element is gone throws
   * `TargetNotFoundError` at once.
   */
  getByRef(ref: string): Locator;
}

// The Plan.

/** What became of the Run's Plan, read from the Run's report (#478 places it). */
export interface PlanReport {
  /** The file the Run named, or `null` when it named none and the Plan lives only in its Run trace. */
  readonly file: string | null;
  /** The version the Run read, or `null` for a new Plan. */
  readonly versionUsed: string | null;
  /**
   * `unchanged`; `promoted`: written, after a `verified` Run; `pending_review`: a new version a
   * person must accept, from a Run that was not `verified` or a Plan with Effects (ADR 0009);
   * `frozen`: `plan.frozen` was set.
   */
  readonly status: "unchanged" | "promoted" | "pending_review" | "frozen" | (string & {});
  /** The instructions of the steps compiled or repaired in this Run. */
  readonly changedSteps: readonly string[];
}

// Success checks: `expect`.

/** Options of a Success check. Its timeout is `timeout.wait` (30 s), not Playwright's 5 s (G1). */
export type ExpectOptions = CallOptions;
/** As in Playwright's assertions: a string matches the whole value (text after whitespace is normalized), a RegExp a pattern. */
export type TextExpected = string | RegExp;

/** Playwright's page assertions. Each call is a Success check the Run's Verdict reads. */
export interface PageExpectations {
  readonly not: PageExpectations;
  /** A string must equal the URL, as in Playwright; a RegExp matches a pattern. */
  toHaveURL(url: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveTitle(title: TextExpected, options?: ExpectOptions): Promise<void>;
  toMatchAriaSnapshot(expected: string, options?: ExpectOptions): Promise<void>;
}

/** Playwright's locator assertions, all but `toHaveScreenshot`. */
export interface LocatorExpectations {
  readonly not: LocatorExpectations;
  toBeAttached(options?: ExpectOptions): Promise<void>;
  toBeChecked(options?: ExpectOptions & { readonly checked?: boolean }): Promise<void>;
  toBeDisabled(options?: ExpectOptions): Promise<void>;
  toBeEditable(options?: ExpectOptions): Promise<void>;
  toBeEmpty(options?: ExpectOptions): Promise<void>;
  toBeEnabled(options?: ExpectOptions): Promise<void>;
  toBeFocused(options?: ExpectOptions): Promise<void>;
  toBeHidden(options?: ExpectOptions): Promise<void>;
  toBeInViewport(options?: ExpectOptions & { readonly ratio?: number }): Promise<void>;
  toBeVisible(options?: ExpectOptions): Promise<void>;
  toContainText(expected: TextExpected | readonly TextExpected[], options?: ExpectOptions): Promise<void>;
  toContainClass(expected: string | readonly string[], options?: ExpectOptions): Promise<void>;
  toHaveAccessibleDescription(expected: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveAccessibleErrorMessage(expected: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveAccessibleName(expected: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveAttribute(name: string, value?: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveClass(expected: TextExpected | readonly TextExpected[], options?: ExpectOptions): Promise<void>;
  toHaveCount(count: Count, options?: ExpectOptions): Promise<void>;
  toHaveCSS(name: string, value: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveId(id: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveJSProperty(name: string, value: JsonValue, options?: ExpectOptions): Promise<void>;
  toHaveRole(role: string, options?: ExpectOptions): Promise<void>;
  toHaveText(expected: TextExpected | readonly TextExpected[], options?: ExpectOptions): Promise<void>;
  toHaveValue(value: TextExpected, options?: ExpectOptions): Promise<void>;
  toHaveValues(values: readonly TextExpected[], options?: ExpectOptions): Promise<void>;
  toMatchAriaSnapshot(expected: string, options?: ExpectOptions): Promise<void>;
}

export interface ResponseExpectations {
  readonly not: ResponseExpectations;
  /** A status from 200 to 299. */
  toBeOK(): Promise<void>;
}

/**
 * Runs a Success check: the engine evaluates the condition live, waiting until it holds or its
 * timeout passes, records it in the Run trace, and the Run's Verdict reads its latest evaluation
 * (ADR 0029). Still false at the timeout: throws `SuccessCheckFailedError`. To ask without
 * counting, use `isVisible()` or `count()`; to wait, ADR 0080's waits.
 */
export declare function expect(page: Page): PageExpectations;
export declare function expect(locator: Locator): LocatorExpectations;
export declare function expect(response: Response): ResponseExpectations;

// The AI verbs' Error codes (ADR 0079's naming rule).

/** `act` found no element for its instruction, or `getByRef` named an element that is gone. */
export declare class TargetNotFoundError extends PageError {
  readonly code: "target_not_found";
  readonly details: { readonly instruction: string | null; readonly ref: string | null };
}

/** A step drifted and could not be repaired: `plan.frozen`, no model configured, or its one Repair drifted too. */
export declare class PlanDriftError extends PageError {
  readonly code: "plan_drift";
  readonly details: {
    readonly instruction: string;
    readonly drift: Drift;
    readonly whyNoRepair: "frozen" | "no_model" | "repair_drifted" | (string & {});
  };
}

/** `plan.frozen` is set and the Plan holds no entry for this call; compile it unfrozen, review and commit it. */
export declare class PlanStepMissingError extends ProgramError {
  readonly code: "plan_step_missing";
  readonly details: { readonly instruction: string; readonly file: string | null };
}

/** The Plan file cannot be read or does not match its format; a format version it cannot read follows ADR 0030's refusals. */
export declare class PlanInvalidError extends ProgramError {
  readonly code: "plan_invalid";
  readonly details: { readonly file: string; readonly problem: string };
}

/** A call the Plan holds no entry for needs a model, and none is configured; `fix` names `ai.model` (ADR 0101). */
export declare class ModelNotConfiguredError extends ProgramError {
  readonly code: "model_not_configured";
  readonly details: { readonly instruction: string };
}

/** Required fields were not found on the page. `found` holds the fields that were, never typed as complete. */
export declare class ExtractionIncompleteError extends PageError {
  readonly code: "extraction_incomplete";
  readonly details: {
    readonly found: JsonValue;
    readonly missing: readonly { readonly path: readonly (string | number)[]; readonly reason: "absent" | "no_match" | (string & {}) }[];
  };
}

/** The model's answers never passed the JSON Schema, the program's validators (ADR 0082) or Grounding within ADR 0012's retries. */
export declare class ExtractionInvalidError extends ModelError {
  readonly code: "extraction_invalid";
  readonly details: {
    readonly attempts: number;
    readonly model: ModelReference;
    /** The strategy of the last attempt (ADR 0101). */
    readonly strategy: StructuredOutputStrategy;
    /** The last attempt's problems, per field. */
    readonly issues: readonly { readonly path: readonly (string | number)[]; readonly message: string }[];
  };
}

/** An `act` whose action the engine classed an Effect failed after the action was sent. Never retry: a person reconciles it (ADR 0029). `cause` holds the original. */
export declare class EffectUncertainError extends PageError {
  readonly code: "effect_uncertain";
  readonly retryable: false;
  readonly details: { readonly instruction: string; readonly journalId: string };
}

/** An AI Effect in one of ADR 0014's categories was not confirmed: a person declined its Human pause, or the pause expired (ADR 0092). */
export declare class EffectNotConfirmedError extends RefusedError {
  readonly code: "effect_not_confirmed";
  readonly details: { readonly instruction: string; readonly category: string; readonly by: "person" | "timeout" | (string & {}) };
}

/** `expect`'s condition was still false at its timeout. `limit` names `timeout.wait` and its layer. */
export declare class SuccessCheckFailedError extends PageError {
  readonly code: "success_check_failed";
  readonly details: {
    /** The Success check in this language: `expect(page).toHaveURL(/^https:\/\/quotes\.toscrape\.com\/author\//)`. */
    readonly check: string;
    readonly expected: string;
    /** The last value seen, or `null` when no element matched. */
    readonly observed: string | null;
    readonly timeout: number;
  };
}

/** A `%name%` placeholder that neither `variables` nor a bound Secret placeholder fills, a variable no placeholder uses, or a name that is both. */
export declare class VariablesMismatchError extends ProgramError {
  readonly code: "variables_mismatch";
  readonly details: {
    readonly missing: readonly string[];
    readonly unused: readonly string[];
    readonly conflicting: readonly string[];
  };
}
