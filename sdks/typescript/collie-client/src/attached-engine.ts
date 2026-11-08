// An Attached engine's process (ADRs 0024 and 0078): started from its
// executable with exactly the program's environment map, in a process group
// of its own so a terminal's Ctrl+C reaches only the host, its stdio the
// Connection's transport. Its stderr is piped, never inherited: each line is
// passed to the host's logger at warning, and the last 64 KiB are kept for
// the error its unexpected exit raises.
import { spawn, type ChildProcess } from "node:child_process";
import { constants } from "node:fs";
import { access, stat } from "node:fs/promises";
import { Readable, Writable } from "node:stream";
import { StringDecoder } from "node:string_decoder";
import { ATTACHED_ENGINE_COMMAND, EngineNotFoundError } from "./generated/index.ts";
import type { Logger } from "./logger.ts";
import type { Transport } from "./rpc.ts";

/** How much of the engine's stderr its exit keeps (ADR 0078). */
const STDERR_TAIL_BYTES = 64 * 1024;

/** How an engine's process ended. */
export interface EngineExit {
  /** Its exit code; `null` when a signal ended it. */
  readonly exitCode: number | null;
  /** The signal that ended it, such as `SIGKILL`. */
  readonly signal: string | null;
  /** The last 64 KiB of its stderr. */
  readonly stderrTail: string;
}

/** What starting an Attached engine takes. */
export interface AttachedEngineStart {
  /** The engine's executable. */
  readonly path: string;
  /** Its whole environment. */
  readonly env: Readonly<Record<string, string>>;
  /** Where the path came from, which `engine_not_found`'s `fix` names. */
  readonly notFoundFix: string;
  /** The host's logger, which gets the engine's stderr lines. */
  readonly logger?: Logger | undefined;
}

/** A running Attached engine. */
export class AttachedEngine {
  /** Its executable. */
  readonly path: string;
  /** Its stdio, the Connection's transport. */
  readonly transport: Transport;
  /** Settles once its process has ended and its stderr is read. */
  readonly exited: Promise<EngineExit>;
  readonly #child: ChildProcess;

  private constructor(path: string, child: ChildProcess, logger: Logger | undefined) {
    const { stdin, stdout, stderr } = child;
    if (stdin === null || stdout === null || stderr === null) {
      throw new TypeError("invariant: the engine's stdio is piped");
    }
    this.path = path;
    this.#child = child;
    // A write after the engine ended fails the transport's writer, which
    // the Connection reports; the stream's own error event carries nothing more.
    stdin.on("error", () => {});
    this.transport = {
      readable: Readable.toWeb(stdout) as ReadableStream<Uint8Array>,
      writable: Writable.toWeb(stdin) as WritableStream<Uint8Array>,
    };
    let tail = Buffer.alloc(0);
    let partial = "";
    const decoder = new StringDecoder("utf8");
    stderr.on("data", (bytes: Buffer) => {
      tail = Buffer.concat([tail, bytes]);
      tail = tail.subarray(Math.max(0, tail.length - STDERR_TAIL_BYTES));
      const lines = (partial + decoder.write(bytes)).split("\n");
      partial = lines.pop() ?? "";
      for (const line of lines) {
        logger?.warn(line, { source: "engine stderr" });
      }
    });
    this.exited = new Promise((resolve) => {
      child.once("close", (exitCode: number | null, signal: NodeJS.Signals | null) => {
        partial += decoder.end();
        if (partial !== "") {
          logger?.warn(partial, { source: "engine stderr" });
        }
        resolve({ exitCode, signal, stderrTail: textOfTail(tail) });
      });
    });
  }

  /**
   * Starts the engine at `path`; `engine_not_found` when it is no
   * executable file.
   */
  static async start(start: AttachedEngineStart): Promise<AttachedEngine> {
    const notFound = (cause?: unknown): EngineNotFoundError =>
      new EngineNotFoundError(`no engine executable at ${start.path}`, {
        fix: start.notFoundFix,
        details: { path: start.path },
        cause,
      });
    try {
      await access(start.path, constants.X_OK);
      if (!(await stat(start.path)).isFile()) {
        throw notFound();
      }
    } catch (cause) {
      throw cause instanceof EngineNotFoundError ? cause : notFound(cause);
    }
    const child = spawn(start.path, [ATTACHED_ENGINE_COMMAND], {
      env: { ...start.env },
      stdio: ["pipe", "pipe", "pipe"],
      detached: true,
      windowsHide: true,
    });
    await new Promise<void>((resolve, reject) => {
      child.once("spawn", resolve);
      child.once("error", (cause) => reject(notFound(cause)));
    });
    return new AttachedEngine(start.path, child, start.logger);
  }

  /**
   * Waits for the engine to stop once its stdin has ended, as it does at
   * the end of its input, and kills it when it has not within `timeout`
   * milliseconds.
   */
  async stop(timeout: number): Promise<EngineExit> {
    let timer: NodeJS.Timeout | undefined;
    const late = new Promise<"late">((resolve) => {
      timer = setTimeout(() => resolve("late"), timeout);
    });
    try {
      const exit = await Promise.race([this.exited, late]);
      if (exit !== "late") {
        return exit;
      }
      this.kill();
      return await this.exited;
    } finally {
      clearTimeout(timer);
    }
  }

  /** Ends the engine's process at once. */
  kill(): void {
    if (this.#child.exitCode === null && this.#child.signalCode === null) {
      this.#child.kill("SIGKILL");
    }
  }
}

/** The text of the stderr tail `bytes`, a character the cut split at its start left out. */
function textOfTail(bytes: Buffer): string {
  let start = 0;
  // UTF-8 continuation bytes are 0b10xxxxxx.
  while (start < bytes.length && ((bytes[start] ?? 0) & 0xc0) === 0x80) {
    start += 1;
  }
  return bytes.subarray(start).toString("utf8");
}
