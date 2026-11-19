// The raw catalog call (ADR 0099, G8): `call(operation, params?, options?)`
// on the Connection and on every handle, the way down to an operation the
// SDK does not generate, such as one a newer engine added, or one of
// Foxhound's from a Collie program on a Foxhound engine. Its params and its
// result are the wire's JSON, in snake_case: no name is camelCased and no
// point in time read as a `Date`. On a value, the operation must be in the
// value's namespace, and the value's ID fills its parameter. The rest is
// the main path's: a closed value fails with `closed`, the timeout and the
// signal apply, failures raise the same classes, nothing is retried, and the
// engine validates the params. Hand-written, since no catalog entry shapes it.
import type { CallOptions } from "./caller.ts";
import { ProgramError } from "./generated/errors.ts";
import type { JSONValue } from "./json.ts";

/** What the raw call takes beside its params (ADR 0099). */
export interface RawCallOptions {
  /**
   * The operation's own `timeout` (ADR 0080), in milliseconds, sent as its
   * `timeout` parameter: `null` for none, left out for its setting's. The
   * engine refuses one that is no whole number of 1 or more, naming the
   * operation's setting, which the SDK does not know.
   */
  readonly timeout?: number | null | undefined;
  /** Stops the call: it is cancelled and fails with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What sends a raw call, as the main path sends a generated one. */
export type RawSend = (
  operation: string,
  params: Readonly<Record<string, unknown>>,
  options: CallOptions,
) => Promise<unknown>;

/** What a handle's raw call reaches its engine through: its host. */
export interface RawHost {
  /** The ID each of the handle's calls carries. */
  readonly id: unknown;
  /** Sends `operation` unless the value is closed, its params and result unconverted. */
  raw: RawSend;
}

/** The value a handle's raw call acts on. */
interface RawValue {
  /** Its namespaces: its own, then each of a type it extends. */
  readonly namespaces: readonly string[];
  /** The parameter its ID fills, such as `page_id`. */
  readonly param: string;
  /** Its ID. */
  readonly id: unknown;
}

/** The program's error for a raw call refused before it is sent. */
function refused(operation: string, reason: string, fix: string): ProgramError {
  return new ProgramError(`${operation}: ${reason}`, {
    code: "invalid_params",
    group: "program",
    retryable: false,
    fix,
    details: { operation },
  });
}

/**
 * The raw call's params: the wire's JSON. A parameter naming a second value
 * takes that value's ID, as a Run's `id` (ADR 0099).
 */
export type RawParams = Readonly<Record<string, JSONValue>>;

/**
 * Sends `operation` with `params` through `send`, its timeout and signal
 * applied; on `value`, refuses an operation of another namespace and fills
 * the value's own parameter.
 */
export async function rawCall(
  send: RawSend,
  operation: string,
  params: RawParams,
  options: RawCallOptions,
  value?: RawValue,
): Promise<JSONValue> {
  const sent: Record<string, unknown> = { ...params };
  if (value !== undefined) {
    const [namespace] = operation.split(".", 1);
    if (namespace === undefined || !value.namespaces.includes(namespace)) {
      throw refused(
        operation,
        `the value it is called on takes only operations of ${value.namespaces.map((name) => `\`${name}\``).join(" or ")}`,
        "Call it on the value of its namespace, or on the Connection when it acts on no value.",
      );
    }
    if (value.param in params) {
      throw refused(
        operation,
        `\`${value.param}\` is the value's own, which the call fills`,
        `Leave \`${value.param}\` out, or call the operation on the value it names.`,
      );
    }
    sent[value.param] = value.id;
  }
  if (options.timeout !== undefined) {
    if ("timeout" in params) {
      throw refused(
        operation,
        "`timeout` is given both in its params and as its option",
        "Give the timeout once, as the option.",
      );
    }
    sent["timeout"] = options.timeout;
  }
  const result = await send(operation, sent, { signal: options.signal });
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): a result is JSON.
  return result as JSONValue;
}

let namespacesOf: (handle: Handle) => string[];

/**
 * Lets `handle` also take operations of `namespace`, that of a type its own
 * extends, as a crawl's Run takes `crawl`'s beside a Run's (ADR 0099).
 */
export function alsoInNamespace(handle: Handle, namespace: string): void {
  namespacesOf(handle).push(namespace);
}

/** What every handle a program holds has: the raw call on its value (ADR 0099). */
export abstract class Handle {
  static {
    namespacesOf = (handle) => handle.#namespaces;
  }

  readonly #host: RawHost;
  readonly #namespaces: string[];
  readonly #param: string;

  /** @internal A generated handle passes its host, its namespace and the parameter its ID fills. */
  constructor(host: RawHost, namespace: string, param: string) {
    this.#host = host;
    this.#namespaces = [namespace];
    this.#param = param;
  }

  /**
   * Sends `operation`, an operation of this value's namespace the SDK may
   * not generate, with `params` as the wire's JSON in snake_case, and
   * answers its result as the engine sent it (ADR 0099). The value's ID
   * fills its own parameter, such as `page_id`. An operation of another
   * namespace fails with a `ProgramError` before it is sent, as does a call
   * on a closed value with `closed`. `timeout` is sent as the operation's
   * own; `signal` cancels the call. It is never retried.
   */
  call(
    operation: string,
    params: RawParams = {},
    options: RawCallOptions = {},
  ): Promise<JSONValue> {
    const host = this.#host;
    const send: RawSend = (name, sent, callOptions) => host.raw(name, sent, callOptions);
    return rawCall(send, operation, params, options, {
      namespaces: this.#namespaces,
      param: this.#param,
      id: host.id,
    });
  }
}
