// The Agent in Collie's SDK API (ADR 0100, decided in #474).
// - `page.agent(task, options)` runs ADR 0013's built-in Agent on one task, to its end: a thin,
//   journaled loop over the Operation catalog's page operations, the program's Agent tools and a
//   `note` action. It starts on the page, may use the tabs it opens, and carries nothing into the
//   next call (G2). The whole task is one gesture of the page (ADR 0085); it gives up the page's turn
//   while an Agent tool or its Success check runs.
// - The program states the Success check with `expect` (ADR 0091) in `successCheck`. The engine runs
//   it each time the model reports done; a failure goes back to the model, and the Verdict reads its
//   latest evaluation (ADR 0029). A model's "done" is a signal, never proof (ADR 0013).
// - The result is ADR 0079's `AiWorkResult` plus the steps, their `ModelUsage` (ADR 0101) and the
//   page the task ended on. A step cap, a stall rule, the task's timeout, a Limit and an abort are
//   values: `ended: stopped` with its reason, and a Grounded `partial`.
// - The steps are a lossless sequence that starts at the task's first step (ADR 0081).
// - With the Run's `plan` (ADR 0091), the task is one Plan entry, replayed whole with no model call
//   (before each Effect, every target reachable without it is checked). On Drift, or a failed Success
//   check after the replay, the loop takes over live from there: no Repair (amends ADR 0009).

import type { EventIterator } from "./concurrency.js";
import type { JsonValue, Schema, SchemaOutput } from "./data.js";
import type { Page } from "./lifecycle.js";
import type { ModelReference, ModelUsage } from "./models.js";
import type { Locator } from "./page.js";
import type { Drift } from "./ai.js";
import type { AiWorkResult, GoodfetchError, ProgramError } from "./results.js";
import type { CallOptions } from "./time.js";

// Steps.

/** Where a step came from. `model`: decided by the model now. `plan`: replayed from the Plan, no model call. */
export type AgentStepOrigin = "model" | "plan" | (string & {});

/** How one action of a step ended. A failure goes back to the model; it never throws from `page.agent`. */
export type AgentActionOutcome =
  | { readonly kind: "done" }
  | { readonly kind: "failed"; readonly error: GoodfetchError }
  /** The Effect journal already held this Effect, so nothing was done again (ADR 0010). */
  | { readonly kind: "journal" }
  | { readonly kind: "unknown"; readonly raw: string };

/** One action a step took. */
export interface AgentAction {
  /**
   * The catalog operation, such as `locator.click` or `page.goto`; `tool:<name>` for an Agent tool;
   * `note` for the built-in action that notes the answer so far.
   */
  readonly operation: string;
  /** Its arguments, `%name%` placeholders where a variable or a secret was typed, never the value. */
  readonly arguments: readonly JsonValue[];
  /** The target of a page action, an ordinary locator resolved again at each use. */
  readonly target: Locator | undefined;
  /**
   * A page action in this language, as the engine renders it (ADR 0091), so a person can turn a task
   * into plain page API calls: `await page.getByRole('link', { name: 'Travel' }).click();`.
   */
  readonly code: string | undefined;
  readonly outcome: AgentActionOutcome;
}

/** One step: one model decision and the actions it took, or one step replayed from the Plan. */
export interface AgentStep {
  /** From 1. */
  readonly index: number;
  readonly origin: AgentStepOrigin;
  /**
   * On the first live step after a replay drifted, or after its Success check failed: what the Plan
   * expected and what the page showed (ADR 0091's `Drift`). The replay ended there and the loop took over.
   */
  readonly drift: Drift | undefined;
  /** The model's own words for this step: a signal, never proof. Empty for a replayed step. */
  readonly message: string;
  readonly actions: readonly AgentAction[];
  /** The URL of the page the task is on after the step. */
  readonly url: string;
  /** Its actions failed, or repeated earlier ones on targets whose page had not changed since. */
  readonly stalled: boolean;
  /** This step's model calls and their price: none for a replayed step. */
  readonly usage: ModelUsage;
}

// The result and the task handle.

/**
 * What a task returns: ADR 0079's three separate fields (`ended`, `verdict`, `reported`), the output,
 * and what the task did. `output` is a valid `T` whose values Grounding found on the pages the task
 * observed or in its Agent tools' results; a value it could not find leaves the Verdict at
 * `needs-review`, naming the field. A task replayed whole ends `finished` once its Success check
 * ran, with no `reported`, since no model claimed anything.
 */
export interface AgentResult<T> extends AiWorkResult<T> {
  /** Every step, in order. */
  readonly steps: readonly AgentStep[];
  /** The whole task's model calls and their price (ADR 0101). */
  readonly usage: ModelUsage;
  /**
   * The page the task ended on: its own page, or a tab it opened, which the program now holds. The
   * other tabs it opened are closed as `page.close()` closes them: on a Borrowed browser, released.
   */
  readonly page: Page;
}

/**
 * A running task. It started when `page.agent` was called; await it for the result. `steps()` reads
 * the steps as they happen. A hand-written promise shape, as `Call<T>` is a hand-written builder.
 */
export interface AgentTask<T> extends Promise<AgentResult<T>> {
  /**
   * The task's steps, lossless, from step 1 whenever it is called, ending when the task ends, and
   * throwing the task's error when it failed; a failure seen here counts as handled, so a program that
   * reads only the steps meets no unhandled rejection. Each call is a reader of its own; an unread
   * open iterator slows the task (ADR 0081).
   */
  steps(): EventIterator<AgentStep>;
}

// Agent tools.

/** ADR 0014's categories of Effects that ask a person for confirmation unless the run allows them; area 4's spec spells them. */
export type ConfirmationCategory =
  | "payment"
  | "account_creation"
  | "accepting_terms"
  | "sending_messages"
  | "deletion"
  | (string & {});

/** What an Agent tool receives besides its input. */
export interface AgentToolCall {
  /** The page the task is on when the model calls the tool. The task gives up its turn meanwhile, so the tool's gestures run. */
  readonly page: Page;
  /** Aborted when the task ends or is cancelled while the tool runs. */
  readonly signal: AbortSignal;
}

/** What `agentTool` takes. */
export interface AgentToolDefinition<S extends Schema> {
  /** Unique within a task, and no built-in action's name, or the task fails with `AgentToolInvalidError`. */
  readonly name: string;
  /** What the model reads to decide when to call it. */
  readonly description: string;
  /** The input's schema in ADR 0082's forms, checked before `run` is called. */
  readonly input: S;
  /** `false` by default: the tool is an Effect, journaled and never redone (ADR 0010), as a site's WebMCP tool (ADR 0011). */
  readonly readOnly?: boolean;
  /**
   * Asks a person for confirmation before each call unless the run allows the category (ADR 0014);
   * payment and accepting terms always ask, since their allowances name domains (ADRs 0045, 0048) and a
   * tool has none. A decline ends the task with `EffectNotConfirmedError`.
   */
  readonly category?: ConfirmationCategory;
  /**
   * Runs where the program's code runs (ADR 0081). Its JSON is what the model reads, bounded by
   * `observation.max_size`, and a source Grounding accepts. A `GoodfetchError` it throws ends the task
   * with its own code, and any other exception with `AgentToolFailedError`; a cancellation is never
   * wrapped. An answer the model should act on is a value it returns.
   */
  run(input: SchemaOutput<S>, call: AgentToolCall): Promise<JsonValue>;
}

/** A function of the program that the Agent may call as an action: an Agent tool. */
export interface AgentTool {
  readonly name: string;
  readonly description: string;
  readonly readOnly: boolean;
  readonly category: ConfirmationCategory | undefined;
}

/** Makes an Agent tool, inferring `run`'s input type from its schema. */
export declare function agentTool<S extends Schema>(definition: AgentToolDefinition<S>): AgentTool;

// Options.

/**
 * Options of a task. `timeout` is `timeout.agent` (10 minutes) for this task: reaching it cancels the
 * step in flight and stops the task with the reason `timeout`; it does not throw. An enclosing call's
 * timeout, such as a crawl handler's, throws `DeadlineExceededError` naming that call. `timeout.ai`
 * does not apply to the task. Aborting `signal` cancels the task.
 */
export interface AgentOptions extends CallOptions {
  /**
   * The Success check, stated with `expect` (ADR 0091) on the page the task is on, run each time the
   * model reports done. Each `expect` in it is a Success check the engine evaluates live, within
   * `timeout.wait` unless given a shorter timeout. Without one, the Verdict is `needs-review` (ADR 0013).
   * A `PageError` it throws (`SuccessCheckFailedError`, a target not unique) is the condition's answer,
   * given back to the model; another `GoodfetchError` ends the task with its own code; any other
   * exception, with `SuccessCheckThrewError`.
   */
  readonly successCheck?: (page: Page) => Promise<void>;
  /** `agent.max_steps` for this task: 20 by default. Reaching it stops only the task. */
  readonly maxSteps?: number;
  /**
   * `agent.domains` for this task: by default the registrable domain of the page the task starts on.
   * An entry matches its host and their subdomains; `"*"` allows wherever the run's domain policy does.
   * It bounds the task's navigations, never the subresources a page loads.
   */
  readonly domains?: readonly string[];
  readonly tools?: readonly AgentTool[];
  /**
   * Plain values for the task's `%name%` placeholders, as `act` takes them (ADR 0091). A placeholder no
   * variable fills is a Secret placeholder bound by `secrets.<name>` (ADR 0092).
   */
  readonly variables?: Readonly<Record<string, string>>;
  /** The model for this task alone: `ai.model` at the run layer (ADR 0101). */
  readonly model?: ModelReference;
}

/** The Agent, on a page. */
export interface AgentApi {
  /** Runs one task with an output in the schema's type. */
  agent<S extends Schema>(task: string, options: AgentOptions & { readonly schema: S }): AgentTask<SchemaOutput<S>>;
  /** Runs one task whose answer is the page it leaves; the model's message is in `reported`. */
  agent(task: string, options?: AgentOptions): AgentTask<undefined>;
}

// Settings.

/** `agent.*` (ADR 0100). `timeout.agent` is in the timeout family. */
export interface AgentSettings {
  /** Steps a task may take: 20 by default. `maxSteps` sets it for one task. */
  readonly maxSteps?: number;
  /** Stalled steps in a row that stop a task: 3 by default. */
  readonly maxStalledSteps?: number;
  /** Where a task may navigate: by default the registrable domain of the page it starts on. No `null`: `["*"]` lifts it. */
  readonly domains?: readonly string[];
  /**
   * Whether the model may ask a person through a Human pause: `false` by default (ADR 0092). The answer,
   * a decline or the pause running out goes back to the model.
   */
  readonly humanPause?: boolean;
}

/** On an error `page.agent` throws after the task took steps (ADR 0100). */
export interface AgentTaskProgress {
  readonly steps: readonly AgentStep[];
  /** The partial answer as JSON: only values valid against their part of the schema and Grounded. */
  readonly partial: JsonValue | undefined;
  /** The Agent tools whose Effect the task left uncertain, for a person to reconcile (ADR 0029). */
  readonly uncertainEffects: readonly string[];
}

// The Agent's Error codes (ADR 0079's naming rule).

export interface AgentToolFailedDetails {
  readonly tool: string;
  readonly exceptionType: string;
  readonly exceptionMessage: string;
  /** The tool was not read-only, so its Effect is left uncertain for a person to reconcile (ADR 0029). */
  readonly effectUncertain: boolean;
}
/** An Agent tool threw something other than a `GoodfetchError`: the task ended. `cause` holds the original, in the process that threw it. */
export declare class AgentToolFailedError extends ProgramError {
  readonly code: "agent_tool_failed";
  readonly details: AgentToolFailedDetails;
}

/** Two tools share a name, or one takes a built-in action's: refused before the task starts. */
export declare class AgentToolInvalidError extends ProgramError {
  readonly code: "agent_tool_invalid";
  readonly details: { readonly tool: string; readonly problem: "duplicate" | "built_in_name" | (string & {}) };
}

/** The Success check function threw something other than `SuccessCheckFailedError`: a bug, never a rejection. `cause` holds it. */
export declare class SuccessCheckThrewError extends ProgramError {
  readonly code: "success_check_threw";
  readonly details: { readonly exceptionType: string; readonly exceptionMessage: string };
}
