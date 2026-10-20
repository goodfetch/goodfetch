// The errors a Connection raises (ADR 0079): an engine's JSON-RPC error
// turned into its code's generated class, or its group's class keeping the
// code when the SDK does not know it; nothing in the reply is validated
// beyond what picking the class needs (STYLE.md, SDK idioms).
import {
  errorGroupClasses,
  errorGroupOfWireClass,
  type ErrorClass,
  type GoodfetchError,
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
