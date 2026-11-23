// Schemas in the program's own types (ADR 0082, G7): any schema that
// implements both Standard Schema and Standard JSON Schema (Zod 4.2 and
// later, ArkType 2.1.28 and later, Valibot 1.2 and later through
// `toStandardJsonSchema` of `@valibot/to-json-schema`), or a JSON Schema
// written by hand, the way down (G8). The interfaces' types are copied from
// `@standard-schema/spec` 1.1.0, as the spec intends, so the SDK depends on
// no schema library. What a program sends has the schema's input type and
// what it receives the output type, built by the schema's own parse; a
// hand-written JSON Schema types both as a JSON value.

import { SchemaValidatorFailedError } from "./generated/errors.ts";
import { pointerToken, type JSONObject, type JSONValue } from "./json.ts";

/** Standard Typed's inferred types (`@standard-schema/spec` 1.1.0). */
interface StandardTypes<Input, Output> {
  /** The input type of the schema. */
  readonly input: Input;
  /** The output type of the schema. */
  readonly output: Output;
}

/** A Standard Schema issue's path segment. */
export interface StandardPathSegment {
  /** The key representing a path segment. */
  readonly key: PropertyKey;
}

/** One issue a Standard Schema's validation found. */
export interface StandardIssue {
  /** The error message of the issue. */
  readonly message: string;
  /** The path of the issue, if any. */
  readonly path?: readonly (PropertyKey | StandardPathSegment)[] | undefined;
}

/** What a Standard Schema's `validate` answers. */
export type StandardResult<Output> =
  | { readonly value: Output; readonly issues?: undefined }
  | { readonly issues: readonly StandardIssue[] };

/** The Standard Schema interface, version 1 (`@standard-schema/spec` 1.1.0). */
export interface StandardSchemaV1<Input = unknown, Output = Input> {
  /** The Standard Schema properties. */
  readonly "~standard": {
    /** The version number of the standard. */
    readonly version: 1;
    /** The vendor name of the schema library. */
    readonly vendor: string;
    /** Inferred types associated with the schema. */
    readonly types?: StandardTypes<Input, Output> | undefined;
    /** Validates unknown input values. */
    readonly validate: (
      value: unknown,
      options?: { readonly libraryOptions?: Record<string, unknown> | undefined },
    ) => StandardResult<Output> | Promise<StandardResult<Output>>;
  };
}

/** The options of a Standard JSON Schema converter. */
export interface StandardJSONSchemaOptions {
  /** The target version of the generated JSON Schema. */
  readonly target: "draft-2020-12" | "draft-07" | "openapi-3.0" | (string & {});
  /** Vendor-specific parameters. */
  readonly libraryOptions?: Record<string, unknown> | undefined;
}

/** The Standard JSON Schema interface, version 1 (`@standard-schema/spec` 1.1.0). */
export interface StandardJSONSchemaV1<Input = unknown, Output = Input> {
  /** The Standard JSON Schema properties. */
  readonly "~standard": {
    /** The version number of the standard. */
    readonly version: 1;
    /** The vendor name of the schema library. */
    readonly vendor: string;
    /** Inferred types associated with the schema. */
    readonly types?: StandardTypes<Input, Output> | undefined;
    /** Methods for generating the input and output JSON Schema. */
    readonly jsonSchema: {
      /** Converts the input type to JSON Schema; may throw where unsupported. */
      readonly input: (options: StandardJSONSchemaOptions) => Record<string, unknown>;
      /** Converts the output type to JSON Schema; may throw where unsupported. */
      readonly output: (options: StandardJSONSchemaOptions) => Record<string, unknown>;
    };
  };
}

/** A schema in the program's own types: Standard Schema with Standard JSON Schema. */
export type TypedSchema<Input = unknown, Output = Input> = StandardSchemaV1<Input, Output> &
  StandardJSONSchemaV1<Input, Output>;

/** A JSON Schema document written by hand, the way down (G8). */
export type JSONSchema = JSONObject;

/** A schema a call takes: a typed one, or a JSON Schema written by hand. */
export type Schema = TypedSchema | JSONSchema;

/** What a program sends under `S`: its input type, or a JSON value for a hand-written one. */
export type SchemaInput<S extends Schema> = S extends TypedSchema
  ? NonNullable<S["~standard"]["types"]>["input"]
  : JSONValue;

/** What a program receives under `S`: its output type, or a JSON value for a hand-written one. */
export type SchemaOutput<S extends Schema> = S extends TypedSchema
  ? NonNullable<S["~standard"]["types"]>["output"]
  : JSONValue;

/** One issue the schema's own parse found, at its JSON Pointer. */
export interface SchemaIssue {
  /** The JSON Pointer (RFC 6901) of the value it is about, empty at the root. */
  readonly path: string;
  /** The schema library's own message. */
  readonly message: string;
}

/** What the schema's own parse of a value gave: the typed value, or its issues. */
export type Parsed<Output> =
  | { readonly value: Output }
  | { readonly issues: readonly SchemaIssue[] };

/**
 * `value` parsed by `schema`'s own validation (ADR 0082): the output value,
 * or each issue with its path as a JSON Pointer. A hand-written JSON Schema
 * has no parse in the program, so the value stays the JSON it is. A
 * validation that throws rather than giving issues fails with
 * `schema_validator_failed`, the thrown value as its cause.
 */
export async function parse<S extends Schema>(
  schema: S,
  value: JSONValue,
): Promise<Parsed<SchemaOutput<S>>> {
  if (!isTyped(schema)) {
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a hand-written schema's output type is `JSONValue`, which `value` is.
    return { value: value as SchemaOutput<S> };
  }
  let result: StandardResult<unknown>;
  try {
    result = await schema["~standard"].validate(value);
  } catch (thrown) {
    // Only the library's own issues are a rejection: anything else its
    // validation throws is the program's bug, so the call stops here.
    throw validatorFailed(schema, thrown);
  }
  if (result.issues) {
    return {
      issues: result.issues.map((issue) => ({
        path: pointer(issue.path ?? []),
        message: issue.message,
      })),
    };
  }
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the schema's own parse gave its output type.
  return { value: result.value as SchemaOutput<S> };
}

/**
 * The JSON Schema the engine receives for `schema` (ADR 0082): a typed
 * one's input side, the form the values it parses travel in, as draft
 * 2020-12, and a hand-written one as it is.
 */
export function wireSchema(schema: Schema): Readonly<Record<string, unknown>> {
  return isTyped(schema)
    ? schema["~standard"].jsonSchema.input({ target: "draft-2020-12" })
    : schema;
}

/**
 * The `schema_validator_failed` of `schema`, whose validation threw
 * `thrown` (ADR 0082): the message names the exception's type and message,
 * the details name the schema by its JSON Schema `title`, and no path,
 * which a throw past the library does not carry.
 */
function validatorFailed(schema: TypedSchema, thrown: unknown): SchemaValidatorFailedError {
  const title = titleOf(schema);
  const type = thrown instanceof Error ? thrown.name : thrown === null ? "null" : typeof thrown;
  const message = thrown instanceof Error ? thrown.message : String(thrown);
  const named = title === null ? "the schema" : `the schema ${title}`;
  return new SchemaValidatorFailedError(`${named}'s own validation threw ${type}: ${message}`, {
    cause: thrown,
    fix: "Fix the validator so that it rejects through its library, as a `.refine()` that returns false or an issue it adds, or never throws.",
    details: { schema: title, path: null, exception_type: type, exception_message: message },
  });
}

/** `schema`'s JSON Schema `title`, or null without one or a JSON Schema to read it from. */
function titleOf(schema: TypedSchema): string | null {
  let title: unknown;
  try {
    ({ title } = wireSchema(schema));
  } catch {
    // Standard JSON Schema's converter may throw where it has no JSON
    // Schema for the type: the schema then has no title to name.
    return null;
  }
  return typeof title === "string" ? title : null;
}

/** Whether `schema` is a typed one rather than a JSON Schema written by hand. */
function isTyped(schema: Schema): schema is TypedSchema {
  return "~standard" in schema;
}

/** A Standard Schema issue's path as a JSON Pointer (RFC 6901). */
function pointer(path: readonly (PropertyKey | StandardPathSegment)[]): string {
  return path
    .map((segment) => {
      const key = typeof segment === "object" ? segment.key : segment;
      return `/${pointerToken(String(key))}`;
    })
    .join("");
}
