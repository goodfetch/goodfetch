// Typed data: schemas, Records and Payloads (decided in #469).
// On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its
// schemas with any library that implements Standard Schema and Standard JSON Schema (Zod 4.2+,
// ArkType 2.1.28+, Valibot 1.2+ through its adapter), or as a JSON Schema document by hand (G7, G8).
// The SDK depends on no schema library: the interfaces below are copied from @standard-schema/spec
// 1.1.0, as that spec intends. Goodfetch's own results are passed through unstripped, so a field a
// newer engine adds stays on the object, and nothing new throws (G3).

import type { ProgramError } from "./results.js";

/** A JSON value: Goodfetch's own type for data without a declared schema. */
export type JsonValue = null | boolean | number | string | readonly JsonValue[] | JsonObject;
export type JsonObject = { readonly [key: string]: JsonValue };

/** One problem a validator found, at a path inside the value. */
export interface SchemaIssue {
  readonly message: string;
  readonly path: readonly (string | number)[];
}

/**
 * Standard Schema v1 together with Standard JSON Schema v1: a schema that validates natively and
 * converts itself to JSON Schema. A Zod, ArkType or Valibot schema satisfies it as written.
 */
export interface StandardSchemaWithJson<Input = unknown, Output = Input> {
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
      readonly input: (options: JsonSchemaOptions) => Record<string, unknown>;
      readonly output: (options: JsonSchemaOptions) => Record<string, unknown>;
    };
  };
}

/** A Standard Schema issue as a library reports it; the SDK normalises it to `SchemaIssue`. */
export interface SchemaIssueLike {
  readonly message: string;
  readonly path?: ReadonlyArray<PropertyKey | { readonly key: PropertyKey }> | undefined;
}

export interface JsonSchemaOptions {
  readonly target: "draft-2020-12" | "draft-07" | "openapi-3.0" | ({} & string);
  readonly libraryOptions?: Record<string, unknown> | undefined;
}

/** A JSON Schema document written by hand: the library-free way down. Values it describes are `JsonValue`. */
export type JsonSchema = JsonObject;

/** Anything a schema parameter accepts, for extraction, a Dataset or a Label's Payload. */
export type Schema = StandardSchemaWithJson<unknown, unknown> | JsonSchema;

/** What a program sends: the schema's input type, before its transforms run (enqueue, push). */
export type SchemaInput<S extends Schema> = S extends StandardSchemaWithJson<infer I, unknown> ? I : JsonValue;
/** What a program receives: the schema's output type, built by its native parse (a handler's Payload, an extraction, a Record read back). */
export type SchemaOutput<S extends Schema> = S extends StandardSchemaWithJson<unknown, infer O> ? O : JsonValue;

/**
 * A Run's Dataset, typed by the schema it was declared with, or taking any JSON object when it
 * declares none. The engine validates every Record against the schema's JSON Schema; how a Dataset
 * is declared, how Records are read and what becomes of a rejected one are #479's.
 */
export interface Dataset<Input = JsonObject, Output = Input> {
  readonly name: string;
  /** Provisional: push or pull is #479's. Takes the input type; a value that is not JSON, and that the schema does not encode, throws `ProgramError`. */
  push(record: Input): Promise<void>;
  /** Phantom: carries the output type a Record is read back as. */
  readonly "~output"?: Output;
}

/**
 * A handle on a Label, carrying its Payload's types. A program names the destination of an enqueue
 * with this handle, never with a bare string, so a misspelled Label or a wrong Payload is a type
 * error. How a crawl declares its Labels and handlers is #477's; a Label without a Payload is
 * `Label<undefined>`.
 */
export interface Label<PayloadInput = undefined, PayloadOutput = PayloadInput> {
  readonly name: string;
  /** Phantom: carries the Payload's input and output types. */
  readonly "~payload"?: { readonly input: PayloadInput; readonly output: PayloadOutput };
}

/** The Payload type an enqueue to this Label takes. */
export type PayloadInput<L> = L extends Label<infer I, unknown> ? I : never;
/** The Payload type this Label's handler receives. */
export type PayloadOutput<L> = L extends Label<unknown, infer O> ? O : never;

// The Error codes of typed data, all in the `program` group (ADR 0079).

export interface PayloadInvalidDetails {
  readonly label: string;
  /** Empty when the Payload was refused for its size; `limit` then names `crawl.payload.max_bytes`. */
  readonly issues: readonly SchemaIssue[];
  /** The Payload's size as serialized JSON. */
  readonly sizeBytes: number;
}
/** A Payload failed its Label's schema, was not JSON, or exceeded `crawl.payload.max_bytes`. Thrown by the enqueue call. */
export declare class PayloadInvalidError extends ProgramError {
  readonly code: "payload_invalid";
  readonly details: PayloadInvalidDetails;
}

export interface PayloadSchemaChangedDetails {
  readonly label: string;
  /** Pending requests of this Label whose Payload fails its current schema. */
  readonly failing: number;
  readonly pending: number;
  /** The first failing Payload's problem. */
  readonly firstIssue: SchemaIssue;
}
/** A resume found pending Payloads that fail their Label's changed schema; nothing ran. Fix the schema (a default for the new field) or start a new Run. */
export declare class PayloadSchemaChangedError extends ProgramError {
  readonly code: "payload_schema_changed";
  readonly details: PayloadSchemaChangedDetails;
}

export interface SchemaValidatorFailedDetails {
  /** The schema library, from Standard Schema's `vendor`. */
  readonly vendor: string;
  /** Where in the value the validator was when it threw, when known. */
  readonly path: readonly (string | number)[];
  readonly exceptionType: string;
  readonly exceptionMessage: string;
}
/**
 * The program's own validator threw instead of returning issues, such as a `.refine()` that throws.
 * A rejection goes back to the model for another attempt (ADR 0012); a throw stops the call at once.
 * `cause` holds the original.
 */
export declare class SchemaValidatorFailedError extends ProgramError {
  readonly code: "schema_validator_failed";
  readonly details: SchemaValidatorFailedDetails;
}
