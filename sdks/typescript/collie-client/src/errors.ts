// The errors a Connection raises (ADR 0079): an engine's JSON-RPC error
// turned into its code's generated class, or its group's class keeping the
// code when the SDK does not know it; nothing in the reply is validated
// beyond what picking the class needs (STYLE.md, SDK idioms). And the
// other way, the JSON-RPC error a request the engine sent is answered with.
import {
  errorGroupClasses,
  errorGroupOfProtocolFailure,
  errorGroupOfWireClass,
  GoodfetchError,
  InternalError,
  type ErrorClass,
  type ErrorFields,
  type ErrorGroup,
  type ErrorLimit,
  type ModelUsage,
} from "./generated/index.ts";

/** The error classes a Connection raises by code: its Product's own. */
export type ErrorClasses = Readonly<Record<string, ErrorClass>>;

/**
 * The code a JSON-RPC error with no Error code takes: a bug in Goodfetch,
 * in its group's class.
 */
const NO_CODE = "internal";

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function lines(value: unknown): readonly string[] | undefined {
  return Array.isArray(value) && value.every((line) => typeof line === "string")
    ? value
    : undefined;
}

function settingLimit(value: unknown): ErrorLimit | undefined {
  return isRecord(value) && typeof value["key"] === "string" && typeof value["layer"] === "string"
    ? // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine messages are not validated beyond `hello`'s protocol version (STYLE.md): an object with a setting's key and layer is its limit.
      (value as ErrorLimit)
    : undefined;
}

function modelUsage(value: unknown): ModelUsage | undefined {
  return isRecord(value)
    ? // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine messages are not validated beyond `hello`'s protocol version (STYLE.md): an error's `usage` object is a model usage.
      (value as unknown as ModelUsage)
    : undefined;
}

/**
 * The fields every error carries, read from a reply's `data` in wire names,
 * and the code's own details, which are the rest. A code's details may use
 * a field's name for something else, such as `response_limit_exceeded`'s
 * byte count as `limit`: a value without the field's shape stays in them.
 */
function fieldsOfData(data: Readonly<Record<string, unknown>>): ErrorFields {
  const taken = new Set(["code", "retryable"]);
  function take<Field>(
    key: string,
    read: (value: unknown) => Field | undefined,
  ): Field | undefined {
    const field = read(data[key]);
    if (field !== undefined) {
      taken.add(key);
    }
    return field;
  }
  const fields = {
    fix: take("fix", text),
    operation: take("operation", text),
    callId: take("call_id", text),
    runId: take("run_id", text),
    target: take("target", text),
    url: take("url", text),
    observed: take("observed", text),
    limit: take("limit", settingLimit),
    callLog: take("call_log", lines) ?? [],
    usage: take("usage", modelUsage),
    agentTask: take("agent_task", (value) => (isRecord(value) ? value : undefined)),
  };
  return {
    ...fields,
    details: Object.fromEntries(Object.entries(data).filter(([key]) => !taken.has(key))),
  };
}

/** The wire names of the fields an error carries, `fieldsOfData` read the other way. */
function dataOfFields(error: GoodfetchError): Readonly<Record<string, unknown>> {
  const fields: Readonly<Record<string, unknown>> = {
    fix: error.fix,
    operation: error.operation,
    call_id: error.callId,
    run_id: error.runId,
    target: error.target,
    url: error.url,
    observed: error.observed,
    limit: error.limit,
    call_log: error.callLog.length === 0 ? undefined : error.callLog,
    usage: error.usage,
    agent_task: error.agentTask,
  };
  return Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined));
}

/** The error a reply's JSON-RPC `error` raises. */
export function errorOfReply(error: unknown, classes: ErrorClasses): GoodfetchError {
  const reply = isRecord(error) ? error : {};
  const message =
    typeof reply["message"] === "string" ? reply["message"] : "the engine failed the call";
  const data = isRecord(reply["data"]) ? reply["data"] : {};
  const { code, retryable } = data;
  const fields = fieldsOfData(data);
  const known = typeof code === "string" ? classes[code] : undefined;
  if (known !== undefined) {
    return new known(message, fields);
  }
  // A group's wire class, or a code JSON-RPC predefines, which a failure of
  // the protocol itself takes, such as `-32602` for invalid params (ADR 0079).
  const wireClass = typeof reply["code"] === "number" ? reply["code"] : undefined;
  const group =
    (wireClass === undefined
      ? undefined
      : (errorGroupOfWireClass[wireClass] ?? errorGroupOfProtocolFailure[wireClass])) ?? "internal";
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
  /** The Error code, whether it is retryable, the fields the error carries and its details. */
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
  const wireClass = wireClassOfGroup.get(failure.group);
  if (wireClass === undefined) {
    throw new TypeError(`invariant: the catalog gives the group ${failure.group} a wire class`);
  }
  return {
    code: wireClass,
    message: failure.message,
    data: {
      ...failure.details,
      code: failure.code,
      retryable: failure.retryable,
      ...dataOfFields(failure),
    },
  };
}
