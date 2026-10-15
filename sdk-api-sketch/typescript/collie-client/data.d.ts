// Typed data: schemas, Records and Payloads (decided in #469).
// On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its
// schemas with any library that implements Standard Schema and Standard JSON Schema (Zod 4.2+,
// ArkType 2.1.28+, Valibot 1.2+ through its adapter), or as a JSON Schema document by hand (G7, G8).
// The SDK depends on no schema library: the interfaces below are copied from @standard-schema/spec
// 1.1.0, as that spec intends. Standard Schema only parses, so a value a program sends is already
// JSON: the schema's input side. Goodfetch's own results are passed through neither validated nor
// stripped, so a field a newer engine adds stays on the object, and nothing new throws (G3).

import type { ProgramError } from "./results.js";

/** A JSON value: Goodfetch's own type for data without a declared schema. */
export type JSONValue = null | boolean | number | string | readonly JSONValue[] | JSONObject;
export type JSONObject = { readonly [key: string]: JSONValue };

/** One problem a validator found, at a path inside the value. */
export interface SchemaIssue {
  readonly message: string;
  readonly path: readonly (string | number)[];
}

/**
 * Standard Schema v1 together with Standard JSON Schema v1: a schema that validates natively and
 * converts itself to JSON Schema. A Zod, ArkType or Valibot schema satisfies it as written.
 */
export interface StandardSchemaWithJSON<Input = unknown, Output = Input> {
  readonly "~standard": {
    readonly version: 1;
    readonly vendor: string;
    readonly validate: (
      value: unknown,
      options?: { readonly libraryOptions?: Record<string, unknown> | undefined } | undefined,
    ) =>
      | { readonly value: Output; readonly issues?: undefined }
      | { readonly issues: readonly SchemaIssueLike[] }
      | Promise<{ readonly value: Output; readonly issues?: undefined } | { readonly issues: readonly SchemaIssueLike[] }>;
    readonly types?: { readonly input: Input; readonly output: Output } | undefined;
    readonly jsonSchema: {
      readonly input: (options: JSONSchemaOptions) => Record<string, unknown>;
      readonly output: (options: JSONSchemaOptions) => Record<string, unknown>;
    };
  };
}

/** A Standard Schema issue as a library reports it; the SDK normalises it to `SchemaIssue`. */
export interface SchemaIssueLike {
  readonly message: string;
  readonly path?: ReadonlyArray<PropertyKey | { readonly key: PropertyKey }> | undefined;
}

export interface JSONSchemaOptions {
  readonly target: "draft-2020-12" | "draft-07" | "openapi-3.0" | ({} & string);
  readonly libraryOptions?: Record<string, unknown> | undefined;
}

/** A JSON Schema document written by hand: the library-free way down. Values it describes are `JSONValue`. */
export type JSONSchema = JSONObject;

/** Anything a schema parameter accepts, for extraction, a Dataset or a Label's Payload. */
export type Schema = StandardSchemaWithJSON<unknown, unknown> | JSONSchema;

/** What a program sends: the schema's input type, before its transforms run (enqueue, push). */
export type SchemaInput<S extends Schema> = S extends StandardSchemaWithJSON<infer I, unknown> ? I : JSONValue;
/** What a program receives: the schema's output type, built by its native parse (a handler's Payload, an extraction, a Record read back). */
export type SchemaOutput<S extends Schema> = S extends StandardSchemaWithJSON<unknown, infer O> ? O : JSONValue;

export interface SchemaValidatorFailedDetails {
  /** The schema's JSON Schema `title`, or null when it has none. */
  readonly schema: string | null;
  /** Where in the value the validator was when it threw, when known. */
  readonly path: readonly (string | number)[];
  readonly exceptionType: string;
  readonly exceptionMessage: string;
}
/**
 * The program's own validator threw instead of returning issues, such as a `.refine()` that throws.
 * A rejection is the call's own failure, or, in an extraction, goes back to the model for another
 * attempt (ADR 0012); a throw stops the call at once.
 * `cause` holds the original.
 */
export declare class SchemaValidatorFailedError extends ProgramError {
  readonly code: "schema_validator_failed";
  readonly details: SchemaValidatorFailedDetails;
}
