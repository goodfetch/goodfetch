// A Run's Checkpoint and RunTrace copies (ADRs 0102 and 0116):
// `run.checkpoint()` answers a `Checkpoint` and `run.trace()` / `view.trace()`
// a `RunTrace` (or `null` when the Run keeps no trace) only once the engine's
// file is whole. `saveAs(path)` writes the file on the program's machine whole
// or not at all through a hidden temporary file, and `stream()` yields its
// bytes in order.
import { Buffer } from "node:buffer";
import { saveFileOver } from "./file-saving.ts";
import type { RunCopyConnection } from "./handle.ts";
import {
  collieOperations,
  type Checkpoint as WireCheckpoint,
  type CheckpointSaved,
  type RunTrace as WireRunTrace,
  type TraceSaved,
} from "./generated/index.ts";
import { collieWire, type CollieWireType } from "./generated/wire.ts";
import { warnEntry } from "./markers.ts";
import { Sequence } from "./sequence.ts";
import { fromWire } from "./wire.ts";

/** How many 64 KiB chunks `stream()` lets the engine send ahead. */
const CHUNK_CREDITS = 8;
/** How many chunks `stream()` reads before granting as many credits again. */
const CHUNK_REGRANT = 4;

/** What `Checkpoint.saveAs()` takes. */
export interface CheckpointSaveOptions {
  /** Cancels `checkpoint.save_as`, leaving the path as it was. */
  readonly signal?: AbortSignal | undefined;
}

/** What `Checkpoint.stream()` takes. */
export interface CheckpointStreamOptions {
  /** Cancels `checkpoint.stream` and ends the iteration with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What `RunTrace.saveAs()` takes. */
export interface RunTraceSaveOptions {
  /** Cancels `trace.save_as`, leaving the path as it was. */
  readonly signal?: AbortSignal | undefined;
}

/** What `RunTrace.stream()` takes. */
export interface RunTraceStreamOptions {
  /** Cancels `trace.stream` and ends the iteration with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** One `$/chunk` notification's bytes, or none when the params are no chunk. */
function chunkOf(params: unknown): Uint8Array | undefined {
  if (typeof params !== "object" || params === null || !("data" in params)) {
    return undefined;
  }
  const { data } = params;
  return typeof data === "string" ? new Uint8Array(Buffer.from(data, "base64")) : undefined;
}

/**
 * A completed Checkpoint of a Run (ADRs 0002, 0102 and 0116): its descriptor
 * (`runId`, `location`, `size`, `digest`, `runEpoch`, `journalPosition`,
 * `time`) with `saveAs(path)` and `stream()` to copy it to the program's
 * machine.
 */
export class Checkpoint {
  readonly #connection: RunCopyConnection;

  /** The Run's ID. */
  readonly runId: string;
  /** Where it is: the file's path on the engine's machine. */
  readonly location: string;
  /** Its size in bytes. */
  readonly size: number;
  /** The SHA-256 of its bytes, in lowercase hexadecimal. */
  readonly digest: string;
  /** The run epoch of the writer that took it (ADR 0003). */
  readonly runEpoch: number;
  /** The position of the run's journal it holds. */
  readonly journalPosition: number;
  /** When it was taken. */
  readonly time: Date;

  /** @internal `run.checkpoint()` builds a Checkpoint; a program never constructs one. */
  constructor(connection: RunCopyConnection, descriptor: WireCheckpoint) {
    this.#connection = connection;
    this.runId = descriptor.runId;
    this.location = descriptor.location;
    this.size = descriptor.size;
    this.digest = descriptor.digest;
    this.runEpoch = descriptor.runEpoch;
    this.journalPosition = descriptor.journalPosition;
    this.time = descriptor.time;
  }

  /**
   * Saves a Checkpoint's file, which the engine keeps on its machine, whole
   * or not at all (ADRs 0102 and 0116). An Attached engine given the program's
   * path writes it through a hidden temporary file beside it, synced and
   * renamed over it, creating missing folders and replacing a file there; any
   * other engine writes nothing on its own machine and sends the file in
   * `$/chunk` notifications of at most 64 KiB, one per credit granted with
   * `$/credit`, for the SDK to write. Answers the file's size, its SHA-256
   * digest and its run epoch. A path that cannot be written raises
   * `path_not_writable`, and full or failing storage `file_write_failed`,
   * leaving the path as it was.
   *
   * @experimental
   */
  async saveAs(path: string, options: CheckpointSaveOptions = {}): Promise<CheckpointSaved> {
    warnEntry(collieOperations, "checkpoint.save_as");
    const raw = await saveFileOver<{ readonly written: boolean }>(
      this.#connection,
      "checkpoint.save_as",
      { run_id: this.runId, location: this.location },
      path,
      { call: "saveAs", signal: options.signal },
    );
    // Engine answers are not validated beyond `hello`'s protocol version (STYLE.md).
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `checkpoint.save_as` answers `CheckpointSaved` on the wire.
    return fromWire(
      raw,
      { type: "CheckpointSaved" satisfies CollieWireType },
      collieWire.types,
    ) as CheckpointSaved;
  }

  /**
   * Sends a Checkpoint's bytes in order, in `$/chunk` notifications of at
   * most 64 KiB, one per credit granted with `$/credit`, and answers its size,
   * its SHA-256 digest and its run epoch once every byte is sent (ADRs 0102
   * and 0116). Nothing is written on either machine: a program that writes the
   * bytes itself meets its own language's file errors.
   *
   * @experimental
   */
  stream(options: CheckpointStreamOptions = {}): Sequence<Uint8Array> {
    warnEntry(collieOperations, "checkpoint.stream");
    const connection = this.#connection;
    const params = { run_id: this.runId, location: this.location };
    return new Sequence({
      open: (opened) => connection.stream("checkpoint.stream", params, opened),
      itemOf: (method, notification) => (method === "$/chunk" ? chunkOf(notification) : undefined),
      credits: CHUNK_CREDITS,
      regrant: CHUNK_REGRANT,
      signal: options.signal,
    });
  }
}

/**
 * A Run's trace copy (ADRs 0102 and 0116): its descriptor (`runId`,
 * `location`, `size`, `digest`) with `saveAs(path)` and `stream()` to copy it
 * to the program's machine.
 */
export class RunTrace {
  readonly #connection: RunCopyConnection;

  /** The Run's ID. */
  readonly runId: string;
  /** Where it is: the trace file's path on the engine's machine. */
  readonly location: string;
  /** Its size in bytes. */
  readonly size: number;
  /** The SHA-256 of its bytes, in lowercase hexadecimal. */
  readonly digest: string;

  /** @internal `run.trace()` and `RunView.trace()` build a RunTrace; a program never constructs one. */
  constructor(connection: RunCopyConnection, descriptor: WireRunTrace) {
    this.#connection = connection;
    this.runId = descriptor.runId;
    this.location = descriptor.location;
    this.size = descriptor.size;
    this.digest = descriptor.digest;
  }

  /**
   * Saves a Run trace's file, which the engine keeps on its machine, whole or
   * not at all (ADRs 0102 and 0116). An Attached engine given the program's
   * path writes it through a hidden temporary file beside it, synced and
   * renamed over it, creating missing folders and replacing a file there; any
   * other engine writes nothing on its own machine and sends the file in
   * `$/chunk` notifications of at most 64 KiB, one per credit granted with
   * `$/credit`, for the SDK to write. Answers the file's size and its SHA-256
   * digest. A path that cannot be written raises `path_not_writable`, and full
   * or failing storage `file_write_failed`, leaving the path as it was.
   *
   * @experimental
   */
  async saveAs(path: string, options: RunTraceSaveOptions = {}): Promise<TraceSaved> {
    warnEntry(collieOperations, "trace.save_as");
    const raw = await saveFileOver<TraceSaved>(
      this.#connection,
      "trace.save_as",
      { run_id: this.runId, location: this.location },
      path,
      { call: "saveAs", signal: options.signal },
    );
    return raw;
  }

  /**
   * Sends a Run trace's bytes in order, in `$/chunk` notifications of at most
   * 64 KiB, one per credit granted with `$/credit`, and answers its size and
   * its SHA-256 digest once every byte is sent (ADRs 0102 and 0116). Nothing
   * is written on either machine: a program that writes the bytes itself meets
   * its own language's file errors.
   *
   * @experimental
   */
  stream(options: RunTraceStreamOptions = {}): Sequence<Uint8Array> {
    warnEntry(collieOperations, "trace.stream");
    const connection = this.#connection;
    const params = { run_id: this.runId, location: this.location };
    return new Sequence({
      open: (opened) => connection.stream("trace.stream", params, opened),
      itemOf: (method, notification) => (method === "$/chunk" ? chunkOf(notification) : undefined),
      credits: CHUNK_CREDITS,
      regrant: CHUNK_REGRANT,
      signal: options.signal,
    });
  }
}
