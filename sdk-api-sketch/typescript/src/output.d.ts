// Crawl output in code (ADR 0103, decided in #479).
// A crawl declares its Datasets as it declares its Labels, each a typed handle (ADR 0082), with no
// default Dataset. A handler pushes Records through its context, which stages them with its attempt:
// they commit with it, numbered in commit order, or are discarded when it ends early (ADR 0003).
// A program reads committed Records back from the crawl's Run as a stream that starts at the
// Dataset's first Record, follows the Run live, and never slows the crawl, since Records are already
// in run state. Sinks are settings (`sinks.<name>.*`, settings.d.ts); an Export is a file the engine
// writes, which the program saves or streams wherever the engine runs. Datasets and Sinks are
// Foxhound's (ADR 0084), and so are the Records and Exports made from them; every Run, Collie's
// included, has a Key-value store (ADR 0088).

import type { EventIterator } from "./concurrency.js";
import type { JsonValue, Schema, SchemaIssue, SchemaOutput } from "./data.js";
import type { ProgramError } from "./results.js";
import type { CallOptions } from "./time.js";

/**
 * The fields a Dataset's `key` may name: the schema's top-level fields, or any string for a Dataset
 * with no schema or a hand-written JSON Schema. The SDK also checks, when the Dataset is declared,
 * that the schema requires them (`DatasetKeyInvalidError`).
 */
export type KeyField<Output> = JsonValue extends Output ? string : keyof Output & string;

/** Options of declaring a Dataset. */
export interface DatasetOptions<S extends Schema> {
  /** The Records' schema (ADR 0082): the type checker catches a wrong Record, and the engine validates each one. */
  readonly schema: S;
  /**
   * The fields that identify a Record, for the same thing collected twice (two pages, or two runs).
   * The Dataset stays append-only. Sinks' keyed write modes use it unless a Sink sets its own key,
   * and change tracking identifies a Record against a Baseline by it (ADRs 0026, 0036).
   */
  readonly key?: readonly KeyField<SchemaOutput<S>>[];
}

/** One committed Record, as a program reads it back. */
export interface DatasetRecord<T> {
  /** Its number in its Dataset: 1 for the first, in commit order, with no gaps. */
  readonly number: number;
  /** Built by the schema's own parse: its output type (ADR 0082). */
  readonly value: T;
  /** The crawl request whose attempt committed it. */
  readonly requestId: string;
  readonly url: string;
  readonly committedAt: Date;
}

/** Options of reading Records. */
export interface RecordsOptions {
  /** Starts after this Record number, to resume a reader. Built-in: 0, the Dataset's first Record. */
  readonly after?: number;
}

/** A Record a "Record before commit" Hook rejected (ADR 0033), or that a Hook's transform left outside the schema. */
export interface RejectedRecord {
  /** Its number in the Dataset's rejection table: 1 for the first, in commit order, with no gaps. */
  readonly number: number;
  /** As JSON, never parsed by the schema it failed (G3). */
  readonly record: JsonValue;
  readonly reason: string;
  /** The Hook that rejected it, or null when the schema did, after a Hook's transform. */
  readonly hook: string | null;
  readonly requestId: string;
  readonly url: string;
}

/** What a request's commit did to its Records in one Dataset, counted on its `succeeded` outcome. */
export interface RecordCounts {
  readonly committed: number;
  /** Dropped with a reason by a "Record before commit" Hook: counted, never stored. */
  readonly dropped: number;
  /** In the Dataset's rejection table, read with `run.rejectedRecords()`. */
  readonly rejected: number;
}

/** The Export formats of ADR 0026. */
export type ExportFormat =
  | "json"
  | "jsonl"
  | "csv"
  | "xlsx"
  | "xml"
  | "parquet"
  | "html_table"
  | "rss"
  | "atom"
  | "avro"
  | "arrow_ipc"
  | "markdown";

/** Options of writing an Export. Each format's own options are `export.*` settings (area 6). */
export interface ExportOptions extends CallOptions {
  /** Named, never inferred from a file name, so a typo is a type error. */
  readonly format: ExportFormat;
  /** Built-in: none. */
  readonly compression?: "gzip" | "zstd" | null;
}

/**
 * A file the engine wrote from a Dataset's Records committed when the call was made, wherever the
 * engine runs: a linked engine, a Daemon or Goodfetch Cloud (ADR 0001). Playwright's `Download` names.
 */
export interface Export {
  readonly format: ExportFormat;
  readonly records: number;
  /** The number of the last Record it holds. */
  readonly lastNumber: number;
  /** In bytes. */
  readonly size: number;
  readonly sha256: string;
  /** Copies it to a path on the program's machine. */
  saveAs(path: string, options?: CallOptions): Promise<void>;
  /** Its bytes, read once: an async iterable, not the Node `Readable` of Playwright's `createReadStream()`. */
  stream(): AsyncIterable<Uint8Array>;
}

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
 * compare-and-set: a value concurrent handlers share live stays in the program's own memory.
 */
export interface KeyValueStore {
  /** The JSON value under the key, or null when there is none: an answer, never an error. */
  get(key: string, options?: CallOptions): Promise<JsonValue | null>;
  /** The value under the key, built by the schema's own parse; one it rejects throws `ValueInvalidError`. */
  get<S extends Schema>(key: string, schema: S, options?: CallOptions): Promise<SchemaOutput<S> | null>;
  /** Never null, so null from `get` always means absent: `delete` removes a key. A nested null is kept. */
  set(key: string, value: Exclude<JsonValue, null>, options?: CallOptions): Promise<void>;
  getBytes(key: string, options?: CallOptions): Promise<StoredBytes | null>;
  setBytes(key: string, content: Uint8Array, contentType: string, options?: CallOptions): Promise<void>;
  /** Whether the key existed. */
  delete(key: string, options?: CallOptions): Promise<boolean>;
  keys(options?: { readonly prefix?: string }): EventIterator<string>;
}

// The Error codes of crawl output, all in the `program` group and none retryable (ADR 0103).

/** A second Dataset of one name in one crawl. Thrown at once, with no I/O. */
export declare class DatasetDuplicateError extends ProgramError {
  readonly code: "dataset_duplicate";
  readonly details: { readonly dataset: string };
}
/**
 * A push to a Dataset another crawl declared, or a resume whose crawl no longer declares a Dataset
 * that holds Records (`records` of them); nothing ran.
 */
export declare class DatasetUnknownError extends ProgramError {
  readonly code: "dataset_unknown";
  readonly details: { readonly dataset: string; readonly records: number };
}
/** A `key` naming a field the schema does not require, thrown when the Dataset is declared; or a resume whose Dataset's key changed. */
export declare class DatasetKeyInvalidError extends ProgramError {
  readonly code: "dataset_key_invalid";
  readonly details: { readonly dataset: string; readonly fields: readonly string[] };
}

export interface RecordInvalidDetails {
  readonly dataset: string;
  readonly issues: readonly SchemaIssue[];
  /** The Record's number when reading one back failed its schema's parse; null on a push. */
  readonly number: number | null;
}
/**
 * A pushed Record failed its Dataset's schema, natively or in the engine, was not JSON, or lacked its
 * key's fields; it was not staged. Caught, the attempt goes on; escaping the handler, the request
 * ends `failed` with it, not retried. Also thrown by a reader whose native parse rejects a Record.
 */
export declare class RecordInvalidError extends ProgramError {
  readonly code: "record_invalid";
  readonly details: RecordInvalidDetails;
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
