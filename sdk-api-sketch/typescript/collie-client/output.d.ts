// A Run's Key-value store (ADR 0103): every Run keeps one, Collie's included (ADR 0088). Datasets,
// Records, Exports and Sinks are Foxhound's, in `@goodfetch/foxhound` (ADR 0095).

import type { EventIterator } from "./concurrency.js";
import type { JSONValue, Schema, SchemaIssue, SchemaOutput } from "./data.js";
import type { ProgramError } from "./results.js";
import type { CallOptions } from "./time.js";

/** Bytes kept under a key, with their content type. */
export interface StoredBytes {
  readonly content: Uint8Array;
  readonly contentType: string;
}

/**
 * A Run's Key-value store: files, blobs and the program's own state, in the run file, checkpointed
 * with it (ADR 0002). Reached from the program's Run, it writes at once. Reached through a handler's
 * `ctx.run`, which is bound to its attempt (ADR 0093), it stages its writes with the attempt, which
 * commit or are discarded with its Records and enqueues, and its reads see the attempt's own writes
 * first.
 *
 * For each key the last commit wins. A read sees the committed values and its own attempt's staged
 * writes; another attempt's staged writes stay invisible until it commits. There is no
 * compare-and-set: a value concurrent handlers share live stays in the program's own memory,
 * within one process; across worker processes a program counts from committed Records or outcomes.
 */
export interface KeyValueStore {
  /** The JSON value under the key, or null when there is none: an answer, never an error. */
  get(key: string, options?: CallOptions): Promise<JSONValue | null>;
  /** The value under the key, built by the schema's own parse; one it rejects throws `ValueInvalidError`. */
  get<S extends Schema>(key: string, schema: S, options?: CallOptions): Promise<SchemaOutput<S> | null>;
  /** Never null, so null from `get` always means absent: `delete` removes a key. A nested null is kept. */
  set(key: string, value: Exclude<JSONValue, null>, options?: CallOptions): Promise<void>;
  getBytes(key: string, options?: CallOptions): Promise<StoredBytes | null>;
  setBytes(key: string, content: Uint8Array, contentType: string, options?: CallOptions): Promise<void>;
  /** Whether the key existed. */
  delete(key: string, options?: CallOptions): Promise<boolean>;
  keys(options?: { readonly prefix?: string }): EventIterator<string>;
}

/** A Key-value store value that is not JSON or is null (`set`; `fix` names `delete`), fails its schema (`get`), or holds bytes (`get`; `fix` names `getBytes`). */
export declare class ValueInvalidError extends ProgramError {
  readonly code: "value_invalid";
  readonly details: {
    readonly key: string;
    readonly problem: "not_json" | "null" | "schema" | "bytes" | (string & {});
    readonly issues: readonly SchemaIssue[];
  };
}
