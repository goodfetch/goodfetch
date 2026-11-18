// The program's side of file saving (ADR 0116): a file is written whole or
// not at all. Its bytes go into a hidden temporary file beside the path,
// `.<name>.<random>.partial`, which is synced, then renamed over the path,
// missing folders made first and a file there replaced. A failure, a
// deadline or a cancellation removes the temporary file, so the path holds
// the whole file or what it held before. A failure the path causes is told
// from one of the storage by the OS's error, raised as `path_not_writable`
// or `file_write_failed` with side `program`, the Node system error chained
// as the cause. Where the engine runs on the program's machine, the engine
// writes the path itself under the same rule; otherwise it streams the
// bytes, under credits, and this writes them.
import { execFile } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdir, open, rename, rm, stat, type FileHandle } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import type { ClientConnection } from "./client-connection.ts";
import { FileWriteFailedError, PathNotWritableError } from "./generated/errors.ts";

/** What a write names in its errors' `fix`: the call whose path argument it was, such as `saveAs`. */
export interface FileWriteOptions {
  /** The call the program made, named in the errors' `fix`, such as `saveAs` or `screenshot()`. */
  readonly call: string;
  /** Gives the write up before the rename, removing its temporary file. */
  readonly signal?: AbortSignal | undefined;
}

/** The reasons ADR 0116 gives `path_not_writable`; every other one is the storage's. */
const PATH_REASONS: ReadonlySet<string> = new Set([
  "permission_denied",
  "read_only_filesystem",
  "is_a_directory",
  "not_a_directory",
  "invalid_filename",
  "file_too_large",
  "not_found",
]);

/** ADR 0116's reason for each Node error code a file write meets; any other is `other`. */
const REASONS: Readonly<Record<string, string>> = {
  ENOSPC: "storage_full",
  EDQUOT: "quota_exceeded",
  EACCES: "permission_denied",
  EPERM: "permission_denied",
  EROFS: "read_only_filesystem",
  EISDIR: "is_a_directory",
  ENOTDIR: "not_a_directory",
  ENAMETOOLONG: "invalid_filename",
  EFBIG: "file_too_large",
  ENOENT: "not_found",
};

/** What ADR 0116 tells of a failure: its reason, and the OS's name for it. */
interface Failure {
  readonly reason: string;
  readonly osCode: string | null;
  readonly message: string;
  readonly cause: unknown;
}

/** The Node error code of `error`, a system error, if it has one. */
function codeOf(error: unknown): string | undefined {
  if (typeof error === "object" && error !== null && "code" in error) {
    const { code } = error;
    return typeof code === "string" ? code : undefined;
  }
  return undefined;
}

/** The failure `error` is, as it happened while the write did `doing`. */
function failureOf(error: unknown, doing: string): Failure {
  const code = codeOf(error);
  const told = error instanceof Error ? error.message : String(error);
  return {
    reason: (code === undefined ? undefined : REASONS[code]) ?? "other",
    osCode: code ?? null,
    message: `could not ${doing}: ${told}`,
    cause: error,
  };
}

/** What to change in the path argument of `call`, for `reason`. */
function pathFix(call: string, reason: string): string {
  switch (reason) {
    case "is_a_directory":
      return `Give \`${call}\` a file name, not a folder.`;
    case "not_a_directory":
      return `Give \`${call}\` a path whose parent folders are folders, not files.`;
    case "invalid_filename":
      return `Give \`${call}\` a file name this filesystem accepts.`;
    case "file_too_large":
      return `Give \`${call}\` a path on a filesystem that holds a file this size.`;
    default:
      return `Give \`${call}\` a path in a folder this user can write.`;
  }
}

/** The error the program sees for `failure` at `path`, the path as given. */
function errorOf(failure: Failure, path: string, call: string): Error {
  if (PATH_REASONS.has(failure.reason)) {
    return new PathNotWritableError(
      `The path \`${path}\` cannot be written (${failure.message}); nothing was written there.`,
      {
        fix: pathFix(call, failure.reason),
        details: { reason: failure.reason, os_code: failure.osCode, path },
        cause: failure.cause,
      },
    );
  }
  return new FileWriteFailedError(
    `The file could not be written at \`${path}\` (${failure.message}); the path holds what it held before.`,
    {
      fix: `Free space where \`${path}\` is, then call \`${call}\` again.`,
      details: { reason: failure.reason, os_code: failure.osCode, path, side: "program" },
      cause: failure.cause,
    },
  );
}

/** A failure the write found itself, before the OS gave one. */
function found(reason: string, osCode: string | null, message: string): Failure {
  return { reason, osCode, message, cause: undefined };
}

/** Sets or clears the hidden attribute of `path` on Windows, where a leading dot hides nothing. */
async function setHidden(path: string, hidden: boolean): Promise<void> {
  if (process.platform !== "win32") {
    return;
  }
  // Node offers no file attribute call: `attrib` is Windows' own.
  await new Promise<void>((settle, reject) => {
    execFile("attrib", [hidden ? "+h" : "-h", path], { windowsHide: true }, (error) => {
      if (error === null) {
        settle();
      } else {
        reject(error);
      }
    });
  });
}

/** Syncs the folder at `folder`, so the rename survives a crash, where the OS lets a folder be synced. */
async function syncFolder(folder: string): Promise<void> {
  if (process.platform === "win32") {
    return;
  }
  try {
    const handle = await open(folder, "r");
    try {
      await handle.sync();
    } finally {
      await handle.close();
    }
  } catch {
    // The file is in place; a folder that cannot be synced only weakens its durability after a crash.
  }
}

/**
 * A file being written to its path under ADR 0116's rule, from bytes that
 * arrive in parts: `write` each part, then `finish`, or `abort` to leave
 * the path as it was.
 */
export class FileSaving {
  readonly #given: string;
  readonly #path: string;
  readonly #partial: string;
  readonly #handle: FileHandle;
  readonly #call: string;
  #ended = false;

  private constructor(
    given: string,
    path: string,
    partial: string,
    handle: FileHandle,
    call: string,
  ) {
    this.#given = given;
    this.#path = path;
    this.#partial = partial;
    this.#handle = handle;
    this.#call = call;
  }

  /**
   * Starts the file for `path`, relative to the working directory or
   * absolute: its missing folders made, then its hidden temporary file
   * created beside it.
   *
   * @throws {PathNotWritableError} when the path cannot hold a file, such as a folder or a path through a file.
   * @throws {FileWriteFailedError} when its storage fails.
   */
  static async create(path: string, options: FileWriteOptions): Promise<FileSaving> {
    const target = resolve(path);
    const name = basename(target);
    const folder = dirname(target);
    const fail = (failure: Failure): Error => errorOf(failure, path, options.call);
    if (name === "" || /[\\/]$/u.test(path)) {
      throw fail(found("invalid_filename", null, `\`${path}\` names no file`));
    }
    const existing = await stat(target).catch(() => undefined);
    if (existing?.isDirectory() === true) {
      throw fail(found("is_a_directory", unixCode("EISDIR"), `\`${path}\` is a folder`));
    }
    try {
      await mkdir(folder, { recursive: true });
    } catch (error) {
      if (codeOf(error) === "EEXIST") {
        throw fail({
          ...found(
            "not_a_directory",
            unixCode("ENOTDIR"),
            `a file is where the folder \`${folder}\` should be`,
          ),
          cause: error,
        });
      }
      throw fail(failureOf(error, "make its folder"));
    }
    const partial = join(folder, `.${name}.${randomBytes(6).toString("hex")}.partial`);
    let handle: FileHandle;
    try {
      handle = await open(partial, "wx");
    } catch (error) {
      throw fail(failureOf(error, "create the temporary file"));
    }
    const saving = new FileSaving(path, target, partial, handle, options.call);
    try {
      await setHidden(partial, true);
    } catch (error) {
      await saving.abort();
      throw fail(failureOf(error, "hide the temporary file"));
    }
    return saving;
  }

  /**
   * Writes `bytes` after those written before.
   *
   * @throws {FileWriteFailedError} when the storage fails, such as when it is full; the temporary file is removed.
   * @throws {PathNotWritableError} for a file too large for its filesystem; the temporary file is removed.
   */
  async write(bytes: Uint8Array): Promise<void> {
    try {
      let written = 0;
      while (written < bytes.byteLength) {
        const { bytesWritten } = await this.#handle.write(bytes, written);
        written += bytesWritten;
      }
    } catch (error) {
      await this.abort();
      throw errorOf(failureOf(error, "write the file"), this.#given, this.#call);
    }
  }

  /**
   * Syncs the file, then renames it over the path, and syncs its folder.
   *
   * @throws {FileWriteFailedError} or {PathNotWritableError} as `write` does; the path then holds what it held before.
   */
  async finish(): Promise<void> {
    let doing = "sync the file";
    try {
      await this.#handle.sync();
      doing = "close the file";
      this.#ended = true;
      await this.#handle.close();
      doing = "show the file";
      await setHidden(this.#partial, false);
      doing = "rename it over the path";
      await rename(this.#partial, this.#path);
    } catch (error) {
      await this.abort();
      throw errorOf(failureOf(error, doing), this.#given, this.#call);
    }
    await syncFolder(dirname(this.#path));
  }

  /** Removes the temporary file, leaving the path as it was; idempotent. */
  async abort(): Promise<void> {
    if (!this.#ended) {
      this.#ended = true;
      await this.#handle.close().catch(() => undefined);
    }
    await rm(this.#partial, { force: true }).catch(() => undefined);
  }
}

/** `name`, the OS's name of an error the write finds before the OS gives one, where the OS names it so. */
function unixCode(name: string): string | null {
  return process.platform === "win32" ? null : name;
}

/** Throws `signal`'s reason once it is aborted. */
function throwIfAborted(signal: AbortSignal | undefined): void {
  signal?.throwIfAborted();
}

/**
 * Writes `content` at `path` whole or not at all (ADR 0116): a relative
 * path resolves against the working directory, missing folders are made
 * and a file there is replaced. An aborted `signal`, as a deadline's,
 * rejects with its reason and leaves the path as it was.
 *
 * @throws {PathNotWritableError} when the path causes the failure.
 * @throws {FileWriteFailedError} when the storage does, with side `program`.
 */
export async function writeFileWhole(
  path: string,
  content: Uint8Array | AsyncIterable<Uint8Array>,
  options: FileWriteOptions,
): Promise<void> {
  throwIfAborted(options.signal);
  const saving = await FileSaving.create(path, options);
  try {
    if (content instanceof Uint8Array) {
      await saving.write(content);
    } else {
      for await (const part of content) {
        throwIfAborted(options.signal);
        await saving.write(part);
      }
    }
    throwIfAborted(options.signal);
  } catch (error) {
    await saving.abort();
    throw error;
  }
  await saving.finish();
}

/** How many `$/chunk` notifications the saver lets the engine send ahead of the ones it has written. */
const CHUNKS_AHEAD = 16;

/** What a save call answers: whether the engine wrote the path itself. */
export interface Saved {
  readonly written: boolean;
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
 * Makes `method`, a save call such as `download.save_as`, and leaves its
 * file at `path` whole or not at all (ADR 0116), whoever writes it. Where
 * the engine runs on the program's machine, the call carries the path,
 * made absolute against the working directory, and the engine may write
 * it; otherwise, or when the engine answers it did not, the engine streams
 * the bytes in `$/chunk` notifications, `CHUNKS_AHEAD` credits ahead, and
 * this writes them. A failure or an aborted `signal` leaves the path as it
 * was.
 *
 * @throws {PathNotWritableError} or {FileWriteFailedError} from the engine's write or this one's.
 */
export async function saveFile<Result extends Saved>(
  connection: ClientConnection,
  method: string,
  params: Readonly<Record<string, unknown>>,
  path: string,
  options: FileWriteOptions,
): Promise<Result> {
  throwIfAborted(options.signal);
  const local = connection.engine().kind !== "remote";
  const parts: Uint8Array[] = [];
  let failed: unknown;
  let saving: FileSaving | undefined;
  let draining: Promise<void> | undefined;
  // Writes the chunks in order, one after the other; each written lets the engine send one more.
  const drain = async (): Promise<void> => {
    for (let next = parts.shift(); next !== undefined; next = parts.shift()) {
      if (failed !== undefined) {
        continue;
      }
      try {
        saving ??= await FileSaving.create(path, options);
        await saving.write(next);
        stream.grant(1);
      } catch (error) {
        failed = error;
      }
    }
    draining = undefined;
  };
  const stream = connection.stream(method, local ? { ...params, path: resolve(path) } : params, {
    signal: options.signal,
    notified: (notification, chunk) => {
      const bytes = notification === "$/chunk" ? chunkOf(chunk) : undefined;
      if (bytes !== undefined) {
        parts.push(bytes);
        draining ??= drain();
      }
    },
  });
  stream.grant(CHUNKS_AHEAD);
  let result: Result;
  try {
    // Engine answers are not validated beyond `hello`'s protocol version (STYLE.md).
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the save call's result is the operation's, which carries `written`.
    result = (await stream.result) as Result;
    await draining;
    if (failed !== undefined) {
      throw failed;
    }
    if (result.written) {
      await saving?.abort();
      return result;
    }
    throwIfAborted(options.signal);
    saving ??= await FileSaving.create(path, options);
  } catch (error) {
    await draining;
    await saving?.abort();
    throw error;
  }
  await saving.finish();
  return result;
}
