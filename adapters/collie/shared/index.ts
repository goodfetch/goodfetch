// Shared internal helpers for Collie's TypeScript framework adapters
// (ADRs 0011, 0070, 0095 and 0099; spec #608): canonical JSON value
// conversion, parameter normalization, and structured Goodfetch Error mapping.

/** Readonly JSON value structurally identical to `@goodfetch/collie`'s `JSONValue`. */
export type ReadonlyJsonValue =
  | null
  | string
  | number
  | boolean
  | readonly ReadonlyJsonValue[]
  | { readonly [key: string]: ReadonlyJsonValue };

/** Mutable JSON value compatible with Vercel AI SDK `JSONValue` and readonly `JSONValue`. */
export type MutableJsonValue =
  | null
  | string
  | number
  | boolean
  | MutableJsonObject
  | MutableJsonArray;

/** Mutable plain JSON object with {@link MutableJsonValue} entries. */
export type MutableJsonObject = { [key: string]: MutableJsonValue };

/** Mutable JSON array with {@link MutableJsonValue} elements. */
export type MutableJsonArray = MutableJsonValue[];

/** Plain JSON object with {@link MutableJsonValue} entries. */
export type JsonObject = MutableJsonObject;

/** Structural shape of a `@goodfetch/collie` `GoodfetchError` instance. */
export interface GoodfetchErrorLike {
  readonly code: string;
  readonly group: string;
  readonly retryable: boolean;
  readonly message: string;
  readonly operation?: string | undefined;
  readonly fix?: string | undefined;
  readonly target?: string | undefined;
  readonly url?: string | undefined;
  readonly observed?: string | undefined;
  readonly limit?: unknown;
  readonly details: Readonly<Record<string, unknown>>;
}

/** Structured Goodfetch Error returned as a tool error to the model. */
export interface CollieToolError {
  readonly code: string;
  readonly group: string;
  readonly retryable: boolean;
  readonly message: string;
  readonly operation?: string | undefined;
  readonly fix?: string | undefined;
  readonly target?: string | undefined;
  readonly url?: string | undefined;
  readonly observed?: string | undefined;
  readonly limit?: MutableJsonValue | undefined;
  readonly details?: Readonly<Record<string, MutableJsonValue>> | undefined;
}

/** Discriminated tool execution outcome shared across Collie framework adapters. */
export type CollieToolOutput =
  | { readonly ok: true; readonly result: ReadonlyJsonValue }
  | { readonly ok: false; readonly error: CollieToolError };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Recursively converts an arbitrary runtime value into a plain {@link MutableJsonValue}. */
export function toJsonValue(value: unknown): MutableJsonValue {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => toJsonValue(item));
  }
  if (typeof value === "object") {
    return toJsonObject(Object.fromEntries(Object.entries(value)));
  }
  return typeof value === "bigint" ? value.toString() : "";
}

/** Converts a record into a {@link JsonObject}, dropping `undefined` entries. */
export function toJsonObject(record: Readonly<Record<string, unknown>>): JsonObject {
  const out: JsonObject = {};
  for (const [key, entry] of Object.entries(record)) {
    if (entry !== undefined) {
      out[key] = toJsonValue(entry);
    }
  }
  return out;
}

/** Normalizes untrusted framework tool arguments into a readonly JSON object. */
export function toRawParams(input: unknown): Readonly<Record<string, ReadonlyJsonValue>> {
  if (!isRecord(input)) {
    return {};
  }
  return toJsonObject(input);
}

/** Maps a Goodfetch Error onto a structured {@link CollieToolError}. */
export function toToolError(error: GoodfetchErrorLike, fallbackOperation: string): CollieToolError {
  const details = Object.keys(error.details).length > 0 ? toJsonObject(error.details) : undefined;
  return {
    code: error.code,
    group: error.group,
    retryable: error.retryable,
    message: error.message,
    operation: error.operation ?? fallbackOperation,
    ...(error.fix === undefined ? {} : { fix: error.fix }),
    ...(error.target === undefined ? {} : { target: error.target }),
    ...(error.url === undefined ? {} : { url: error.url }),
    ...(error.observed === undefined ? {} : { observed: error.observed }),
    ...(error.limit === undefined ? {} : { limit: toJsonValue(error.limit) }),
    ...(details === undefined ? {} : { details }),
  };
}

/** Serializes a {@link CollieToolError} to a plain {@link JsonObject}. */
export function errorToJsonObject(error: CollieToolError): JsonObject {
  const out: JsonObject = {
    code: error.code,
    group: error.group,
    retryable: error.retryable,
    message: error.message,
  };
  if (error.operation !== undefined) {
    out["operation"] = error.operation;
  }
  if (error.fix !== undefined) {
    out["fix"] = error.fix;
  }
  if (error.target !== undefined) {
    out["target"] = error.target;
  }
  if (error.url !== undefined) {
    out["url"] = error.url;
  }
  if (error.observed !== undefined) {
    out["observed"] = error.observed;
  }
  if (error.limit !== undefined) {
    out["limit"] = toJsonValue(error.limit);
  }
  if (error.details !== undefined) {
    out["details"] = toJsonValue(error.details);
  }
  return out;
}
