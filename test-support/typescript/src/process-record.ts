// The process record (ADR 0069): every process a test starts through test
// support is recorded, and the test fails if one is still alive after its
// teardown. The fixtures give each test its own record, as `processes`.
import { spawn, type ChildProcessByStdio } from "node:child_process";
import type { Readable, Writable } from "node:stream";
import { TestSupportError } from "./errors.ts";
import { reachMutationChild } from "./mutation-children.ts";

/** How a recorded process ended: its exit code, or the signal that ended it, and the error that kept it from starting, if one did. */
export interface Exit {
  readonly code: number | null;
  readonly signal: NodeJS.Signals | null;
  readonly error?: Error | undefined;
}

/** A process started through a record, with its three standard streams piped. */
export interface RecordedProcess {
  readonly child: ChildProcessByStdio<Writable, Readable, Readable>;
  /** Settles once the process has ended and its streams have closed. */
  readonly exit: Promise<Exit>;
}

/** What a recorded process starts with. */
export interface SpawnOptions {
  /** The process's whole environment. */
  readonly env: Readonly<Record<string, string | undefined>>;
  /** Its working folder; the test runner's own when absent. */
  readonly cwd?: string | undefined;
  /**
   * Whether it leads a process group of its own, as the program a terminal
   * runs does, so a signal sent to its group never reaches the test runner;
   * the runner's group when absent. Unix only: Windows ignores it here.
   */
  readonly detached?: boolean | undefined;
}

/** One test's record of the processes it started. */
export class ProcessRecord {
  readonly #processes: RecordedProcess[] = [];

  /**
   * Starts `command` with `args` and records it. In a StrykerJS run, a Node
   * program runs the run's mutant, and what it covers counts for the test.
   */
  spawn(command: string, args: readonly string[], options: SpawnOptions): RecordedProcess {
    // oxlint-disable-next-line no-underscore-dangle -- StrykerJS's own name for its namespace.
    const mutation = reachMutationChild(options.env, globalThis.__stryker__);
    const child = spawn(command, args, {
      env: mutation.env,
      cwd: options.cwd,
      detached: process.platform !== "win32" && options.detached === true,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    // A failed spawn emits `error`, then `close`; `exit` reports it as ended,
    // with the error that says why, such as a missing program's ENOENT.
    let spawnError: Error | undefined;
    child.on("error", (error) => {
      spawnError = error;
    });
    const exit = new Promise<Exit>((resolve) => {
      child.once("close", (code, signal) => {
        mutation.ended();
        resolve({ code, signal, error: spawnError });
      });
    });
    const recorded = { child, exit };
    this.#processes.push(recorded);
    return recorded;
  }

  /**
   * Fails with `processes_alive` when a recorded process is still alive,
   * after killing each such process and waiting for it to end.
   */
  async close(): Promise<void> {
    const alive = this.#processes.filter(({ child }) => isAlive(child));
    for (const { child } of alive) {
      child.kill("SIGKILL");
    }
    await Promise.all(alive.map(({ exit }) => exit));
    if (alive.length > 0) {
      const processes = alive.map(({ child }) => `${child.spawnfile} (pid ${child.pid})`);
      throw new TestSupportError(
        "processes_alive",
        `processes the test started were alive after its teardown, and were killed: ${processes.join(", ")}`,
      );
    }
  }
}

function isAlive(child: RecordedProcess["child"]): boolean {
  return child.pid !== undefined && child.exitCode === null && child.signalCode === null;
}
