// The base of every error a Goodfetch SDK raises (ADR 0079), which each
// Error code's generated class extends through its group's. Hand-written:
// only the seven groups and the codes' classes come from the catalog.
import type { ErrorGroup } from "./generated/errors.ts";
import type { ModelUsage, SettingLimit } from "./generated/types.ts";

/**
 * What marks a Goodfetch error, shared by every copy of the package a bundle
 * holds, since `instanceof` fails across two copies (ADR 0079).
 */
const GOODFETCH_ERROR: unique symbol = Symbol.for("goodfetch.error");

/**
 * The setting that decided an error's outcome: its key, value, the layer
 * that set it and, for the run layer, where the value came from (ADR 0083).
 * The generated `SettingLimit` with its value as any JSON.
 */
export type ErrorLimit = Omit<SettingLimit, "value"> & { readonly value: unknown };

/** What an error carries beyond its code, group and retryable flag (ADR 0079). */
export interface ErrorFields {
  /** The operation the call made, by its canonical catalog name, such as `locator.click`. */
  readonly operation?: string | undefined;
  /** The call's ID, which finds it in the Run trace. */
  readonly callId?: string | undefined;
  /** The Run the call belonged to, inside a Run. */
  readonly runId?: string | undefined;
  /** What the call acted on, as the engine rendered it in this SDK's language. */
  readonly target?: string | undefined;
  /** The page's URL, where one applies. */
  readonly url?: string | undefined;
  /** What was there instead of what the call needed. */
  readonly observed?: string | undefined;
  /** The setting that decided the outcome: its key, value, layer and, for the run layer, its origin. */
  readonly limit?: ErrorLimit | undefined;
  /** What to do about it, when the engine or the runtime knows. */
  readonly fix?: string | undefined;
  /** The engine's steps for the call, as Playwright's call log. */
  readonly callLog?: readonly string[] | undefined;
  /** The usage of the model calls the call made before it failed, when it made any (ADR 0101). */
  readonly usage?: ModelUsage | undefined;
  /** On an error `page.agent` raises after its task took steps: its steps, partial answer and uncertain tools (ADR 0100). */
  readonly agentTask?: Readonly<Record<string, unknown>> | undefined;
  /** The code's details, as the engine sent them, in wire names. */
  readonly details?: Readonly<Record<string, unknown>> | undefined;
  /** The error that caused it, in this process. */
  readonly cause?: unknown;
}

/** What a class gives its base: the code, its group and retryable flag, and the class's name. */
export interface ErrorIdentity<Code extends string> {
  readonly code: Code;
  readonly group: ErrorGroup;
  readonly retryable: boolean;
  /** The class's name, which a minifier would otherwise rename; the class's own by default. */
  readonly name?: string | undefined;
}

/**
 * The base of every error a Goodfetch SDK raises (ADR 0079). Check `code`
 * rather than `instanceof`, which fails when a bundle holds two copies of
 * the package; `isGoodfetchError` and each Product's guard read it so.
 */
export class GoodfetchError<Code extends string = string> extends Error {
  /** The Error code, as the engine sent it. */
  readonly code: Code;
  /** The code's group, named for its remedy. */
  readonly group: ErrorGroup;
  /** Whether the same call may succeed when made again. */
  readonly retryable: boolean;
  /** The operation the call made, by its canonical catalog name; the message spells it as the caller does. */
  readonly operation: string | undefined;
  /** The call's ID, which finds it in the Run trace. */
  readonly callId: string | undefined;
  /** The Run the call belonged to, inside a Run. */
  readonly runId: string | undefined;
  /** What the call acted on, as the engine rendered it in this SDK's language. */
  readonly target: string | undefined;
  /** The page's URL, where one applies. */
  readonly url: string | undefined;
  /** What was there instead of what the call needed. */
  readonly observed: string | undefined;
  /** The setting that decided the outcome, where one did. */
  readonly limit: ErrorLimit | undefined;
  /** What to do about it, when known. */
  readonly fix: string | undefined;
  /** The engine's steps for the call; empty when it sent none. Never a Secret's value (ADR 0015). */
  readonly callLog: readonly string[];
  /** The usage of the model calls the call made before it failed, when it made any. */
  readonly usage: ModelUsage | undefined;
  /** On an error `page.agent` raises after its task took steps, what the task did. */
  readonly agentTask: Readonly<Record<string, unknown>> | undefined;
  /** The code's details, in wire names; empty when it has none. */
  readonly details: Readonly<Record<string, unknown>>;

  constructor(message: string, init: ErrorFields & ErrorIdentity<Code>) {
    super(message, init.cause === undefined ? undefined : { cause: init.cause });
    this.name = init.name ?? new.target.name;
    this.code = init.code;
    this.group = init.group;
    this.retryable = init.retryable;
    this.operation = init.operation;
    this.callId = init.callId;
    this.runId = init.runId;
    this.target = init.target;
    this.url = init.url;
    this.observed = init.observed;
    this.limit = init.limit;
    this.fix = init.fix;
    this.callLog = init.callLog ?? [];
    this.usage = init.usage;
    this.agentTask = init.agentTask;
    this.details = init.details ?? {};
  }
}

Object.defineProperty(GoodfetchError.prototype, GOODFETCH_ERROR, { value: true });

/**
 * Whether `value` is an error a Goodfetch SDK raised, from this copy of the
 * package or another one in the same bundle.
 */
export function isGoodfetchError(value: unknown): value is GoodfetchError {
  return typeof value === "object" && value !== null && GOODFETCH_ERROR in value;
}

/** The class of one Error code: its message, then what it carries. */
export type ErrorClass = new (message: string, fields?: ErrorFields) => GoodfetchError;
