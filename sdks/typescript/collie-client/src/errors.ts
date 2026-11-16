// The errors a Connection raises (ADR 0079): an engine's JSON-RPC error
// turned into its code's generated class, or its group's class keeping the
// code when the SDK does not know it; nothing in the reply is validated
// beyond what picking the class needs (STYLE.md, SDK idioms). And the
// other way, the JSON-RPC error a request the engine sent is answered with.
import {
  errorGroupClasses,
  errorGroupOfWireClass,
  GoodfetchError,
  InternalError,
  type ErrorClass,
  type ErrorGroup,
} from "./generated/index.ts";

/** The error classes a Connection raises by code: its Product's own. */
export type ErrorClasses = Readonly<Record<string, ErrorClass>>;

/**
 * The code a JSON-RPC error with no Error code takes, such as a protocol
 * failure answered with `-32600`: a bug in Goodfetch, in its group's class.
 */
const NO_CODE = "internal";

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The error a reply's JSON-RPC `error` raises. */
export function errorOfReply(error: unknown, classes: ErrorClasses): GoodfetchError {
  const reply = isRecord(error) ? error : {};
  const message =
    typeof reply["message"] === "string" ? reply["message"] : "the engine failed the call";
  const data = isRecord(reply["data"]) ? reply["data"] : {};
  const { code, retryable, fix, ...details } = data;
  const fields = { fix: typeof fix === "string" ? fix : undefined, details };
  const known = typeof code === "string" ? classes[code] : undefined;
  if (known !== undefined) {
    return new known(message, fields);
  }
  const wireClass = typeof reply["code"] === "number" ? reply["code"] : undefined;
  const group =
    (wireClass === undefined ? undefined : errorGroupOfWireClass[wireClass]) ?? "internal";
  return new errorGroupClasses[group](message, {
    ...fields,
    code: typeof code === "string" ? code : NO_CODE,
    group,
    retryable: retryable === true,
  });
}

/** The wire class of each group (ADR 0038): `errorGroupOfWireClass` read the other way. */
const wireClassOfGroup: ReadonlyMap<ErrorGroup, number> = new Map(
  Object.entries(errorGroupOfWireClass).map(([wireClass, group]) => [group, Number(wireClass)]),
);

/** A JSON-RPC `error`, as the engine reads one (ADR 0038). */
export interface ErrorReply {
  /** The group's wire class. */
  readonly code: number;
  readonly message: string;
  /** The Error code, whether it is retryable, its `fix` when it has one, and its details. */
  readonly data: Readonly<Record<string, unknown>>;
}

/**
 * The JSON-RPC `error` a request the engine sent is answered with when
 * serving it failed with `error`. An error that is no Goodfetch error is a
 * bug in what served it: it is answered `internal`.
 */
export function replyOfError(error: unknown): ErrorReply {
  const failure =
    error instanceof GoodfetchError
      ? error
      : new InternalError(error instanceof Error ? error.message : String(error), {
          code: NO_CODE,
          group: "internal",
          retryable: false,
          cause: error,
        });
  return {
    code: wireClassOfGroup.get(failure.group) ?? INTERNAL_WIRE_CLASS,
    message: failure.message,
    data: {
      ...failure.details,
      code: failure.code,
      retryable: failure.retryable,
      ...(failure.fix === undefined ? {} : { fix: failure.fix }),
    },
  };
}

/** The wire class of `internal`, a bug in Goodfetch. */
const INTERNAL_WIRE_CLASS = -32_007;
