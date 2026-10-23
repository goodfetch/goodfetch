// A Run, always opened by its program (ADRs 0078 and 0102): `finish()` ends
// it and answers its Run report; `close()` frees the value and, while the Run
// is still running, stops it as an abort by its Client, resumable by its Run
// ID. Any other call on a closed Run fails with `closed`.
import { ClosedError, type RunReport, type RunStarted, type StartedAs } from "./generated/index.ts";
import type { CallOptions } from "./generated/index.ts";
import { KeyValueStore, type KeyValueStoreHost } from "./key-value-store.ts";
import { holdInRealm } from "./realm.ts";

/** What a Run reaches its engine through: its Connection. */
export interface RunHost {
  /** Ends the Run as `finished`, answering its report. */
  finish(runId: string, options: CallOptions): Promise<RunReport>;
  /** Stops the Run as an abort by its Client, answering its report. */
  abort(runId: string): Promise<RunReport>;
  /** Whether the Connection can still reach the engine. */
  reachable(): boolean;
  /** Forgets a closed Run. */
  forget(run: Run): void;
  /** Its Key-value store's calls. */
  readonly keyValueStore: KeyValueStoreHost;
}

type RunState = "running" | "ended" | "closed";

/** A Run its program drives, opened by `start()` on a Connection. */
export class Run implements AsyncDisposable {
  readonly #host: RunHost;
  readonly #id: string;
  readonly #startedAs: StartedAs;
  #state: RunState;
  #closing: Promise<void> | undefined;
  /** The Run's Key-value store (ADR 0103), kept in its run state across a resume. */
  readonly keyValueStore: KeyValueStore;

  /** @internal The Connection's start opens a Run; a program never constructs one. */
  constructor(host: RunHost, started: RunStarted) {
    this.#host = host;
    this.#id = started.run_id;
    this.#startedAs = started.started_as;
    this.#state = started.lifecycle === "running" ? "running" : "ended";
    this.keyValueStore = new KeyValueStore(host.keyValueStore, this.#id, (operation) =>
      this.#ensureOpen(operation),
    );
    holdInRealm(this);
  }

  /** The Run's ID, which names it for good. Read without a round trip. */
  id(): string {
    return this.#id;
  }

  /** Whether the start began the Run (`new`) or found it (`existing`). Read without a round trip. */
  startedAs(): StartedAs {
    return this.#startedAs;
  }

  /**
   * Lets the Run finish and answers its Run report: a Run its program
   * drives ends at once, `finished` and `complete`. Aborting `signal` ends
   * only the wait. Fails with `closed` once the Run is closed.
   */
  async finish(options: CallOptions = {}): Promise<RunReport> {
    this.#ensureOpen("run.finish");
    const report = await this.#host.finish(this.#id, options);
    if (this.#state === "running" && report.lifecycle !== "running") {
      this.#state = "ended";
    }
    return report;
  }

  /**
   * Closes the Run's value; idempotent. A Run still running is stopped
   * first, `stopped` and `incomplete` as an abort by its Client, and can be
   * resumed by starting its Run ID again.
   */
  close(): Promise<void> {
    this.#closing ??= this.#close();
    return this.#closing;
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  #ensureOpen(operation: string): void {
    if (this.#state === "closed") {
      throw new ClosedError(
        `${operation}: the Run ${this.#id} is closed; start it again by its Run ID`,
        {
          fix: "Start the Run again by its Run ID to reach it.",
        },
      );
    }
  }

  async #close(): Promise<void> {
    const running = this.#state === "running";
    this.#state = "closed";
    try {
      if (running && this.#host.reachable()) {
        await this.#host.abort(this.#id);
      }
    } finally {
      this.#host.forget(this);
    }
  }
}
