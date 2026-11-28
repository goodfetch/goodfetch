// A view of a Run another program started (ADR 0102), such as a detached
// one, opened by `openRun()` on a Connection with no round trip. It takes no
// writer role, reads `progress()`, `report()`, `trace()` and the Key-value
// store's reads (`get`, `getBytes`, `keys`), and closing it frees only the
// view: the Run goes on.
import { FOLLOWED_VALUE, followed } from "./followed.ts";
import {
  ClosedError,
  collieOperations,
  type RunProgress,
  type RunReport,
} from "./generated/index.ts";
import { runtimeMessage, spelledOperation } from "./goodfetch-error.ts";
import { KeyValueStore, KeyValueStoreView, keyValueStoreHost } from "./key-value-store.ts";
import { warnEntry } from "./markers.ts";
import { holdInRealm } from "./realm.ts";
import { RunTrace } from "./run-copies.ts";
import type { RunHost } from "./run.ts";
import { Sequence } from "./sequence.ts";

/** A Run named by its Run ID, opened as a view: its close never stops it. */
export class RunView implements AsyncDisposable {
  readonly #host: RunHost;
  readonly #id: string;
  #closed = false;
  /**
   * Read-only view of the Run's Key-value store (ADRs 0102 and 0103): `get`,
   * `getBytes` and `keys`, taking no writer lock.
   */
  readonly keyValueStore: KeyValueStoreView;

  /** @internal A Connection's `openRun()` opens a view; a program never constructs one. */
  constructor(host: RunHost, id: string) {
    this.#host = host;
    this.#id = id;
    this.keyValueStore = new KeyValueStoreView(
      new KeyValueStore(
        keyValueStoreHost(host.calls, (method, params, opened) =>
          host.stream(method, params, opened),
        ),
        id,
        (operation) => {
          this.#ensureOpen(operation);
        },
      ),
    );
    holdInRealm(this);
  }

  #ensureOpen(operation: string): void {
    if (this.#closed) {
      const fix = "Open a new RunView with `openRun()` and make the call on it.";
      throw new ClosedError(
        runtimeMessage("The RunView is closed.", {
          operation: spelledOperation(this.#host.operationNames, operation),
          fix,
        }),
        { operation, fix },
      );
    }
  }

  /** The Run's ID, which names it for good. Read without a round trip. */
  id(): string {
    return this.#id;
  }

  /**
   * Reads a Run's progress: what it does and why it waits. Without `after`,
   * it answers at once; given `after`, the position of the value the reader
   * last read, it waits for a change and answers the latest value, so values
   * in between are coalesced. The value whose `last` is true ends the
   * sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
   * or no engine works it (`writer: none`) until a start with its Run ID
   * resumes it. A Run another engine works is read through that engine, the
   * owner the Run index names, and one no engine works from its committed
   * state; a read never takes the Run's writer lock. A Run ID the Run index
   * does not list fails with `run_not_found`.
   *
   * @experimental
   */
  progress(options: { readonly signal?: AbortSignal | undefined } = {}): Sequence<RunProgress> {
    warnEntry(collieOperations, "run.progress");
    this.#ensureOpen("run.progress");
    const calls = this.#host.calls;
    const id = this.#id;
    let take: (() => RunProgress | undefined) | undefined;
    return new Sequence({
      open: (opened) => {
        const reading = followed<RunProgress>(
          {
            read: async (after, signal) => {
              this.#ensureOpen("run.progress");
              const params = { runId: id, ...(after === undefined ? {} : { after }) };
              return calls.call("run.progress", params, { signal });
            },
            position: (value) => value.position,
            last: (value) => value.last,
          },
          opened,
        );
        take = reading.take;
        return reading.stream;
      },
      itemOf: (method) => (method === FOLLOWED_VALUE ? take?.() : undefined),
      credits: 1,
      regrant: 1,
      signal: options.signal,
    });
  }

  /**
   * Reads a Run's report: at once for a Run that ended, and at its end for a
   * live one, which the read never ends. A Run another engine works is read
   * through that engine, and one no engine works from its committed state,
   * never taking its writer lock. A Run that has no live writer and has not
   * ended fails with `run_interrupted`, whose `fix` names a start with its
   * Run ID, and a Run ID the Run index does not list with `run_not_found`.
   *
   * @experimental
   */
  async report(options: { readonly signal?: AbortSignal | undefined } = {}): Promise<RunReport> {
    warnEntry(collieOperations, "run.report");
    this.#ensureOpen("run.report");
    return this.#host.calls.call("run.report", { runId: this.#id }, { signal: options.signal });
  }

  /**
   * Reads a Run's trace copy as recorded so far, or null when the Run keeps
   * no trace (ADRs 0102 and 0116). Returns only once the copy is whole, with
   * its size and SHA-256 digest; a write the engine cannot make fails with
   * `file_write_failed`, side `engine`, and leaves no unfinished file. Takes
   * no writer lock, so a view reads a Run no program holds without resuming
   * it. A Run ID the Run index does not list fails with `run_not_found`.
   *
   * @experimental
   */
  async trace(
    options: { readonly signal?: AbortSignal | undefined } = {},
  ): Promise<RunTrace | null> {
    warnEntry(collieOperations, "run.trace");
    this.#ensureOpen("run.trace");
    const result = await this.#host.calls.call(
      "run.trace",
      { runId: this.#id },
      { signal: options.signal },
    );
    const descriptor = result.trace ?? null;
    return descriptor === null ? null : new RunTrace(this.#host.connection, descriptor);
  }

  /** Closes the view; idempotent. Frees only the view: the Run goes on. */
  close(): Promise<void> {
    this.#closed = true;
    return Promise.resolve();
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}
