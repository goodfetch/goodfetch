// Crawl output in code (ADR 0103, decided in #479).
// A crawl declares its Datasets as it declares its Labels, each a typed handle (ADR 0082), with no
// default Dataset. A handler pushes Records through its context, which stages them with its attempt:
// they commit with it, numbered in commit order, or are discarded when it ends early (ADR 0003).
// A program reads committed Records back from the crawl's Run as a stream that starts at the
// Dataset's first Record, follows the Run live, and never slows the crawl, since Records are already
// in run state. Sinks are settings (`sinks.<name>.*`, settings.d.ts); an Export is a file the engine
// writes, which the program saves or streams wherever the engine runs. Datasets and Sinks are
// Foxhound's (ADR 0088), and so are the Records and Exports made from them; every Run's Key-value
// store, Collie's included, is the client package's.

import type { CallOptions, JSONValue, ProgramError, Schema, SchemaIssue, SchemaOutput } from "@goodfetch/collie-client";

/**
 * The fields a Dataset's `key` may name: the schema's top-level fields, or any string for a Dataset
 * with no schema or a hand-written JSON Schema. The SDK also checks, when the Dataset is declared,
 * that the schema requires them (`DatasetKeyInvalidError`).
 */
export type KeyField<Output> = JSONValue extends Output ? string : keyof Output & string;

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
  readonly record: JSONValue;
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
  format(): ExportFormat;
  records(): number;
  /** The number of the last Record it holds. */
  lastNumber(): number;
  /** In bytes. */
  size(): number;
  sha256(): string;
  /**
   * Copies it to a path on the program's machine through a hidden temporary file renamed over the
   * path once whole, so the path holds the whole Export or what it held before (ADR 0108). It creates
   * missing parent folders and replaces an existing file, as Playwright's `saveAs` does. A path it
   * cannot write throws `PathNotWritableError`; full or failing storage throws `FileWriteFailedError`
   * with `side: "program"`, and calling it again after freeing space works, since the engine still
   * holds the Export.
   */
  saveAs(path: string, options?: CallOptions): Promise<void>;
  /** Its bytes, read once: an async iterable, not the Node `Readable` of Playwright's `createReadStream()`. */
  stream(): AsyncIterable<Uint8Array>;
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

