// Foxhound's typed data (ADR 0082, decided in #469): the handles on a crawl's Datasets and Labels, and
// the Payload's Error codes. Schemas and JSON values are the Collie client package's (ADR 0095).

import type { JSONObject, ProgramError, SchemaIssue } from "@goodfetch/collie-client";

/**
 * A handle on a crawl's Dataset, typed by the schema it was declared with, or taking any JSON object
 * when it declares none. The engine validates every Record against the schema's JSON Schema. A crawl
 * declares it with `crawl.dataset()`, a handler pushes with `ctx.push()`, and a program reads with
 * `run.records()` (ADR 0103, output.d.ts). Standard Schema only parses, so a pushed value is already
 * JSON: the schema's input side.
 */
export interface Dataset<Input = JSONObject, Output = Input> {
  readonly name: string;
  /** The fields that identify a Record (ADR 0103); empty when it declares none. */
  readonly key: readonly string[];
  /** Phantom: carries the type a push takes and the type a Record is read back as. */
  readonly "~types"?: { readonly input: Input; readonly output: Output };
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
  /** Empty when the Payload was refused for its size; `limit` then names `crawl.payload.max_size`. */
  readonly issues: readonly SchemaIssue[];
  /** The Payload's size as serialized JSON. */
  readonly sizeBytes: number;
}
/** A Payload failed its Label's schema, was not JSON, or exceeded `crawl.payload.max_size`. Thrown by the enqueue call, or the failure of a request whose handler's parse rejected a Payload that a changed native rule no longer accepts. */
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

