// A Run, always opened by its program (ADRs 0078 and 0102). Its members are
// the catalog's, generated (ADR 0099): `finish()` ends it and answers its Run
// report. Hand-written here is what no catalog entry gives: its ID and how it
// started, read without a call, its Key-value store, and `close()`, which
// frees the value and, while the Run is still running, stops it as an abort
// by its Client, resumable by its Run ID. Any other call on a closed Run
// fails with `closed`. A Product's package also builds, with `pageOf`, the
// page the engine lends a handler of the Run for one attempt.
import type { CallOptions, Caller, OperationTypes } from "./caller.ts";
import {
  ClosedError,
  Run as RunMembers,
  type CollieOperationTypes,
  type PageId,
  type RunStarted,
  type StartedAs,
} from "./generated/index.ts";
import { Page } from "./generated/handles.ts";
import type { HandleHost } from "./handle.ts";
import { KeyValueStore, type KeyValueStoreHost } from "./key-value-store.ts";
import { holdInRealm } from "./realm.ts";

/** What a Run reaches its engine through: its Connection. */
export interface RunHost {
  /** The Connection's calls. */
  readonly calls: Caller<CollieOperationTypes>;
  /** Whether the Connection can still reach the engine. */
  reachable(): boolean;
  /** Forgets a closed Run. */
  forget(run: Run): void;
  /** Its Key-value store's calls. */
  readonly keyValueStore: KeyValueStoreHost;
}

/** What a Run and the calls made through it share: its ID and where it stands. */
interface RunScope {
  readonly id: string;
  state: "running" | "ended" | "closed";
}

/** A value a program holds under a Run: a Session, a page, or a Locator built on one. */
interface ValueScope {
  /** The value that opened or built it, when the Run did not. */
  readonly parent: ValueScope | undefined;
  /** Whether its program closed it. */
  closed: boolean;
  /** Its `close` in flight. */
  closing: Promise<void> | undefined;
}

function ensureOpen(scope: RunScope, operation: string): void {
  if (scope.state === "closed") {
    throw new ClosedError(
      `${operation}: the Run ${scope.id} is closed; start it again by its Run ID`,
      {
        fix: "Start the Run again by its Run ID to reach it.",
      },
    );
  }
}

/** Whether `value`, or a value that opened or built it, is closed. */
function isClosed(value: ValueScope | undefined): boolean {
  for (let held = value; held !== undefined; held = held.parent) {
    if (held.closed) return true;
  }
  return false;
}

/** Whether `result` is an answer that says its Run is no longer running. */
function saysEnded(result: unknown): boolean {
  return (
    typeof result === "object" &&
    result !== null &&
    "lifecycle" in result &&
    result.lifecycle !== "running"
  );
}

/**
 * What a handle of ID `id` in the Run of `run` reaches the engine through
 * (ADR 0078): each call fails with `closed`, with no round trip, once the
 * Run, the handle's `value` or a value above it is closed. The Run's own
 * calls, which have no `value`, also read their answer, since a Run report
 * says when the Run ended.
 */
function hostIn<Id>(
  calls: Caller<CollieOperationTypes>,
  run: RunScope,
  value: ValueScope | undefined,
  id: Id,
): HandleHost<Id, CollieOperationTypes> {
  return {
    id,
    async call<Name extends keyof CollieOperationTypes>(
      operation: Name,
      params: CollieOperationTypes[Name]["params"],
      options: CallOptions,
    ): Promise<CollieOperationTypes[Name]["result"]> {
      ensureOpen(run, operation);
      if (isClosed(value)) {
        throw new ClosedError(
          `${operation}: the value it is called on is closed, or the one that opened it is`,
          { fix: "Open another from its Run: a closed value never reopens." },
        );
      }
      const result = await calls.call(operation, params, options);
      if (value === undefined && run.state === "running" && saysEnded(result)) {
        run.state = "ended";
      }
      return result;
    },
    child: <ChildId>(childId: ChildId) =>
      hostIn(calls, run, { parent: value, closed: false, closing: undefined }, childId),
    async close<Name extends keyof CollieOperationTypes>(
      operation: Name,
      params: CollieOperationTypes[Name]["params"],
    ): Promise<void> {
      if (value === undefined) {
        throw new TypeError("invariant: a Run closes through its own close(), never its host's");
      }
      // What its Run or the value above it closed is closed with it, and
      // the engine holds it no more: nothing is left to send.
      if (run.state === "closed" || isClosed(value)) {
        value.closed = true;
        return;
      }
      // A close the engine refuses, or that was never sent, closed nothing:
      // the next one sends again.
      value.closing ??= (async (): Promise<void> => {
        await calls.call(operation, params, {});
        value.closed = true;
      })();
      try {
        await value.closing;
      } catch (error) {
        value.closing = undefined;
        throw error;
      }
    },
  };
}

/** `calls`, each of which also stops when `signal` aborts. */
function signalled<Types extends Record<keyof Types, OperationTypes>>(
  calls: Caller<Types>,
  signal: AbortSignal,
): Caller<Types> {
  return {
    call<Name extends keyof Types & string>(
      operation: Name,
      params: Types[Name]["params"],
      options: CallOptions,
    ): Promise<Types[Name]["result"]> {
      const stops =
        options.signal === undefined ? signal : AbortSignal.any([options.signal, signal]);
      return calls.call(operation, params, { signal: stops });
    },
  };
}

let pageIn: (run: Run, pageId: PageId, signal: AbortSignal) => Page;

/**
 * The page of ID `pageId` the engine lends a handler of `run` for one
 * attempt (ADR 0093): a Collie `Page`, whose calls fail with `closed` once
 * the Run is, and each of which stops when `signal`, the attempt's, aborts.
 */
export function pageOf(run: Run, pageId: PageId, signal: AbortSignal): Page {
  return pageIn(run, pageId, signal);
}

/** A Run its program drives, opened by `start()` on a Connection. */
export class Run extends RunMembers implements AsyncDisposable {
  static {
    pageIn = (run, pageId, signal) =>
      new Page(
        hostIn(
          signalled(run.#host.calls, signal),
          run.#scope,
          { parent: undefined, closed: false, closing: undefined },
          pageId,
        ),
      );
  }

  readonly #host: RunHost;
  readonly #scope: RunScope;
  readonly #startedAs: StartedAs;
  #closing: Promise<void> | undefined;
  /** The Run's Key-value store (ADR 0103), kept in its run state across a resume. */
  readonly keyValueStore: KeyValueStore;

  /** @internal The Connection's start opens a Run; a program never constructs one. */
  constructor(host: RunHost, started: RunStarted) {
    const scope: RunScope = {
      id: started.run_id,
      state: started.lifecycle === "running" ? "running" : "ended",
    };
    super(hostIn(host.calls, scope, undefined, started.run_id));
    this.#host = host;
    this.#scope = scope;
    this.#startedAs = started.started_as;
    this.keyValueStore = new KeyValueStore(host.keyValueStore, scope.id, (operation) => {
      ensureOpen(scope, operation);
    });
    holdInRealm(this);
  }

  /** The Run's ID, which names it for good. Read without a round trip. */
  id(): string {
    return this.#scope.id;
  }

  /** Whether the start began the Run (`new`) or found it (`existing`). Read without a round trip. */
  startedAs(): StartedAs {
    return this.#startedAs;
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

  async #close(): Promise<void> {
    const running = this.#scope.state === "running";
    this.#scope.state = "closed";
    try {
      if (running && this.#host.reachable()) {
        await this.#host.calls.call("run.abort", { run_id: this.#scope.id }, {});
      }
    } finally {
      this.#host.forget(this);
    }
  }
}
