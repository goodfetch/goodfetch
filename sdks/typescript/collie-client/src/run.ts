// A Run, always opened by its program (ADRs 0078 and 0102). Its members are
// the catalog's, generated (ADR 0099): `finish()` ends it and answers its Run
// report. Hand-written here is what no catalog entry gives: its ID and how it
// started, read without a call, its Key-value store, and `close()`, which
// frees the value and, while the Run is still running, stops it as an abort
// by its Client, resumable by its Run ID. Any other call on a closed Run
// fails with `closed`.
import type { CallOptions, Caller } from "./caller.ts";
import {
  ClosedError,
  Run as RunMembers,
  type CollieOperationTypes,
  type RunStarted,
  type StartedAs,
} from "./generated/index.ts";
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
  /** The namespace of its operations, such as `page`. */
  readonly namespace: string;
  readonly id: unknown;
  /** The value that opened or built it, when the Run did not. */
  readonly parent: ValueScope | undefined;
  /** Its `close`'s answer, from the moment its program closes it. */
  closing: Promise<unknown> | undefined;
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

/** Fails with `closed` when `value`, or a value it was opened or built by, is closed. */
function ensureValueOpen(value: ValueScope | undefined, operation: string): void {
  for (let held = value; held !== undefined; held = held.parent) {
    if (held.closing !== undefined) {
      const named = typeof held.id === "string" ? `${held.namespace} ${held.id}` : held.namespace;
      throw new ClosedError(`${operation}: the ${named} is closed; a closed value never reopens`, {
        fix: `Open another ${held.namespace} from its Run: a closed one never reopens.`,
      });
    }
  }
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
 * Run, the handle's `value` or a value above it is closed. A value's own
 * `close` is idempotent: it is sent once, and each later one answers what
 * the first did. The Run's own calls, which have no `value`, also read
 * their answer, since a Run report says when the Run ended.
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
      if (value === undefined) {
        const result = await calls.call(operation, params, options);
        if (run.state === "running" && saysEnded(result)) {
          run.state = "ended";
        }
        return result;
      }
      if (operation !== `${value.namespace}.close`) {
        ensureValueOpen(value, operation);
        return calls.call(operation, params, options);
      }
      if (value.closing === undefined) {
        ensureValueOpen(value.parent, operation);
        value.closing = calls.call(operation, params, options);
      }
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `closing` holds the answer of this same operation, the value's own `close`, sent once.
      return value.closing as Promise<CollieOperationTypes[Name]["result"]>;
    },
    child: <ChildId>(namespace: string, childId: ChildId) =>
      hostIn(calls, run, { namespace, id: childId, parent: value, closing: undefined }, childId),
  };
}

/** A Run its program drives, opened by `start()` on a Connection. */
export class Run extends RunMembers implements AsyncDisposable {
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
