// The base of every error a Goodfetch SDK raises (ADR 0079), which each
// Error code's generated class extends through its group's. Hand-written:
// only the seven groups and the codes' classes come from the catalog.
import type { ErrorGroup } from "./generated/errors.ts";
import type { ModelUsage, SettingLimit } from "./generated/types.ts";
import { isRecord } from "./framing.ts";
import { fromWire, pointInTime, type WirePath, type WireTypes } from "./wire.ts";

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

/**
 * The point in time a code's `details` name as `retry_after`, the time the
 * site's `Retry-After` asks for, in UTC; `null` when they name none or a
 * value that is no RFC 3339 text. The classes of the codes whose details
 * declare it read their `retryAfter` with it (ADRs 0092 and 0118).
 */
export function retryAfterOf(details: Readonly<Record<string, unknown>> | undefined): Date | null {
  const sent = details?.["retry_after"];
  return typeof sent === "string" ? (pointInTime(sent) ?? null) : null;
}

/**
 * The duration a code's `details` name as `retry_after`, how long a Model
 * provider asks to wait before trying again, in milliseconds; `null` when
 * they name none or a value that is no count of milliseconds. The classes of
 * the codes whose details declare it read their `retryAfter` with it (ADRs
 * 0080 and 0101).
 */
export function retryAfterDurationOf(
  details: Readonly<Record<string, unknown>> | undefined,
): number | null {
  const sent = details?.["retry_after"];
  return typeof sent === "number" && Number.isInteger(sent) && sent >= 0 ? sent : null;
}

/**
 * A code's `details` as the type `Details` its class reads them as (ADR
 * 0079), or `undefined` when they lack one of the fields `required` names,
 * as those of an error a program builds without details do. A type that
 * holds fields to rename or points in time is read through `wire`, its path
 * in the Product's wire table, as a response is. Like every engine
 * response, the values are taken as sent, beyond the protocol version check.
 */
export function detailsOf<Details extends object>(
  details: Readonly<Record<string, unknown>> | undefined,
  required: readonly (keyof Details & string)[],
  wire?: { readonly path: WirePath; readonly types: WireTypes },
): Readonly<Details> | undefined {
  const read = wire === undefined ? details : fromWire(details, wire.path, wire.types);
  if (!isRecord(read) || !required.every((field) => field in read)) {
    return undefined;
  }
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the catalog names `Details` as the code's details, which hold each field it requires.
  return read as Readonly<Details>;
}

/** The class of one Error code: its message, then what it carries. */
export type ErrorClass = new (message: string, fields?: ErrorFields) => GoodfetchError;

/** Each operation's name in TypeScript form, by its catalog name, which a Product generates. */
export type OperationNames = Readonly<Record<string, string>>;

/** `operation`'s name in TypeScript form, or its catalog name when `names` holds none, as a raw call's. */
export function spelledOperation(names: OperationNames, operation: string): string {
  return names[operation] ?? operation;
}

/**
 * The message of an error the runtime raises itself, in ADR 0079's layout:
 * the operation in TypeScript form, what failed, then the fix. It never
 * holds a chained cause's text, which the error's `cause` keeps.
 */
export function runtimeMessage(
  whatFailed: string,
  { operation, fix }: { readonly operation?: string; readonly fix?: string } = {},
): string {
  const head = operation === undefined ? whatFailed : `${operation}: ${whatFailed}`;
  return fix === undefined ? head : `${head}. ${fix}`;
}

/** A field a copy of an error shows, as its own fields are. */
const SHOWN = { writable: true, configurable: true, enumerable: true };

/** A field a copy of an error keeps out of its own fields, as `message` and `stack` are. */
const HIDDEN = { writable: true, configurable: true, enumerable: false };

/**
 * A copy of `error` whose `fix` is `fix`: it keeps the error's class,
 * message, stack, other fields and cause, as `inCall`'s does.
 */
export function withFix<Failure extends GoodfetchError>(error: Failure, fix: string): Failure {
  return copyOf(error, { fix: { ...SHOWN, value: fix } });
}

/**
 * A copy of `error` on its prototype, so of its class, with its own
 * properties and stack, and `fields` in their place.
 */
function copyOf<Failure extends GoodfetchError>(
  error: Failure,
  fields: PropertyDescriptorMap,
): Failure {
  // The stack is read as text, which V8 may hold behind an accessor of the original alone.
  const { stack } = error;
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a copy of `error`'s own properties on its prototype is an instance of its class.
  return Object.create(Object.getPrototypeOf(error), {
    ...Object.getOwnPropertyDescriptors(error),
    stack: { ...HIDDEN, value: stack },
    ...fields,
  }) as Failure;
}

/**
 * `error`, raised by the runtime, as the call `operation` fails with it: a
 * copy whose message is headed by the operation in TypeScript form and
 * whose `operation` holds its catalog name, unless it already names one.
 * Each call gets its own copy, so one end of a Connection reads as each
 * call it failed; the copy keeps the error's class, fields and cause.
 */
export function inCall<Failure extends GoodfetchError>(
  error: Failure,
  operation: string,
  names: OperationNames,
): Failure {
  if (error.operation !== undefined) {
    return error;
  }
  const message = runtimeMessage(error.message, { operation: spelledOperation(names, operation) });
  // The stack is read as text, which V8 may hold behind an accessor of the
  // original alone, and keeps its frames under the new message.
  const header = `${error.name}: ${error.message}`;
  const stack = error.stack?.startsWith(header)
    ? `${error.name}: ${message}${error.stack.slice(header.length)}`
    : error.stack;
  return copyOf(error, {
    message: { ...HIDDEN, value: message },
    stack: { ...HIDDEN, value: stack },
    operation: { ...SHOWN, value: operation },
  });
}
