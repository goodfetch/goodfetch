// A crawl's Dataset Export (ADRs 0103, 0116 and 0119; issue 1488):
// `CrawlRun.export` calls `run.export` and resolves to an `Export` handle
// only once the engine's file is whole. `saveAs(path)` writes the file on
// the program's machine whole or not at all through a hidden temporary file,
// and `stream()` yields its bytes in order.
import {
  Sequence,
  fromWire,
  saveFile,
  warnEntry,
  type ClientConnection,
} from "@goodfetch/collie-client/runtime";
import {
  foxhoundOperations,
  type CrawlSettings,
  type Export as WireExport,
  type ExportCompression,
  type ExportFormat,
  type ExportId,
  type ExportSaved,
} from "./generated/index.ts";
import { foxhoundWire, type FoxhoundWireType } from "./generated/wire.ts";

/** How many 64 KiB chunks `stream()` lets the engine send ahead. */
const CHUNK_CREDITS = 8;
/** How many chunks `stream()` reads before granting as many credits again. */
const CHUNK_REGRANT = 4;

/** What `CrawlRun.export()` takes. */
export interface ExportOptions {
  /** The format, named, never inferred from a file name (ADR 0103). */
  readonly format: ExportFormat;
  /** The compression; absent, `none`. */
  readonly compression?: ExportCompression | undefined;
  /** `true` writes only Records read past a Wall, `false` only the others (ADR 0052). */
  readonly wall?: boolean | undefined;
  /** The Export's own settings (such as `export.csv.bom`), over the connection's settings (ADR 0083). */
  readonly settings?: CrawlSettings | undefined;
  /** Cancels `run.export` before the file is whole. */
  readonly signal?: AbortSignal | undefined;
}

/** What `Export.saveAs()` takes. */
export interface ExportSaveOptions {
  /** Cancels `export.save_as`, leaving the path as it was. */
  readonly signal?: AbortSignal | undefined;
}

/** What `Export.stream()` takes. */
export interface ExportStreamOptions {
  /** Cancels `export.stream` and ends the iteration with the signal's reason. */
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

let createExport: (connection: ClientConnection, descriptor: WireExport) => Export;

/**
 * A completed Export of one Dataset of a crawl's Run (ADRs 0103 and 0116):
 * its descriptor, whose `size`, `records`, `lastNumber` and `sha256` match
 * the whole file in the Run's folder, with `saveAs(path)` and `stream()`.
 */
export class Export {
  static {
    createExport = (connection, descriptor) => new Export(connection, descriptor);
  }

  readonly #connection: ClientConnection;

  /** The Export's ID. */
  readonly id: ExportId;
  /** Its format. */
  readonly format: ExportFormat;
  /** Its compression. */
  readonly compression: ExportCompression;
  /** How many Records it holds. */
  readonly records: number;
  /** The number of the last Record it holds, 0 when it holds none. */
  readonly lastNumber: number;
  /** Its size in bytes. */
  readonly size: number;
  /** The SHA-256 of its bytes, in lowercase hexadecimal. */
  readonly sha256: string;
  /** The run epoch of the Run's state it was written from (ADR 0003). */
  readonly runEpoch: number;
  /** The Dataset's fields marked with `"x-goodfetch-personal": true` (ADR 0016). */
  readonly personalColumns: readonly string[];

  private constructor(connection: ClientConnection, descriptor: WireExport) {
    this.#connection = connection;
    this.id = descriptor.exportId;
    this.format = descriptor.format;
    this.compression = descriptor.compression;
    this.records = descriptor.records;
    this.lastNumber = descriptor.lastNumber;
    this.size = descriptor.size;
    this.sha256 = descriptor.sha256;
    this.runEpoch = descriptor.runEpoch;
    this.personalColumns = descriptor.personalColumns ?? [];
  }

  /**
   * Saves an Export's file, which the engine keeps in its Run's folder, whole
   * or not at all. An Attached engine given the program's path writes it
   * through a hidden temporary file beside it, synced and renamed over it,
   * creating missing folders and replacing a file there; any other engine
   * writes nothing on its own machine and sends the file in `$/chunk`
   * notifications of at most 64 KiB, one per credit granted with `$/credit`,
   * for the SDK to write. Answers the file's size, its SHA-256 and the run
   * epoch the Export was written under. A path that cannot be written raises
   * `path_not_writable`, and full or failing storage `file_write_failed`,
   * leaving the path as it was.
   *
   * @experimental
   */
  async saveAs(path: string, options: ExportSaveOptions = {}): Promise<ExportSaved> {
    warnEntry(foxhoundOperations, "export.save_as");
    const raw = await saveFile<{ readonly written: boolean }>(
      this.#connection,
      "export.save_as",
      { export_id: this.id },
      path,
      { call: "saveAs", signal: options.signal },
    );
    // Engine answers are not validated beyond `hello`'s protocol version (STYLE.md).
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `export.save_as` answers `ExportSaved` on the wire.
    return fromWire(
      raw,
      { type: "ExportSaved" satisfies FoxhoundWireType },
      foxhoundWire.types,
    ) as ExportSaved;
  }

  /**
   * Sends an Export's bytes, which the engine keeps in its Run's folder, in
   * order, in `$/chunk` notifications of at most 64 KiB, one per credit
   * granted with `$/credit`, and answers its size, its SHA-256 and the run
   * epoch it was written under once every byte is sent. Nothing is written on
   * either machine: a program that writes the bytes itself meets its own
   * language's file errors.
   *
   * @experimental
   */
  stream(options: ExportStreamOptions = {}): Sequence<Uint8Array> {
    warnEntry(foxhoundOperations, "export.stream");
    const connection = this.#connection;
    const exportId = this.id;
    return new Sequence({
      open: (opened) => connection.stream("export.stream", { export_id: exportId }, opened),
      itemOf: (method, notification) => (method === "$/chunk" ? chunkOf(notification) : undefined),
      credits: CHUNK_CREDITS,
      regrant: CHUNK_REGRANT,
      signal: options.signal,
    });
  }
}

/** Package-internal factory for `CrawlRun.export`. */
export function makeExport(connection: ClientConnection, descriptor: WireExport): Export {
  return createExport(connection, descriptor);
}
