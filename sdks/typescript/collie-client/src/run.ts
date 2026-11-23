// A Run, always opened by its program (ADRs 0078 and 0102). Its members are
// the catalog's, generated (ADR 0099): `finish()` ends it and answers its Run
// report. Hand-written here is what no catalog entry gives: its ID and how it
// started, read without a call, its Key-value store, and `close()`, which
// frees the value and, while the Run is still running, stops it as an abort
// by its Client, resumable by its Run ID, unless the Run was started
// `detached` and outlives its program. Any other call on a closed Run
// fails with `closed`. A Product's package also builds, with `pageOf`, the
// page the engine lends a handler of the Run for one attempt, and fails its
// own Run type's calls on a closed Run with `ensureRunOpen`. A listener a
// program registers on a value (ADR 0081) that throws is removed, and its
// failure waits in the Run for the next call on that value, or the `close()`
// that closes it, to raise `listener_failed`; a value released first logs it.
import type { CallOptions, Caller, OperationTypes } from "./caller.ts";
import type { Stream } from "./client-connection.ts";
import {
  CallCancelledError,
  ClosedError,
  ListenerFailedError,
  Run as RunMembers,
  collieOperations,
  type CollieOperationTypes,
  type PageId,
  type RunProgress,
  type RunStarted,
  type SessionId,
  type SettingsExplanation,
} from "./generated/index.ts";
import { FOLLOWED_VALUE, followed } from "./followed.ts";
import { Page, Session } from "./generated/handles.ts";
import { runtimeMessage, spelledOperation, type OperationNames } from "./goodfetch-error.ts";
import { collieWire } from "./generated/wire.ts";
import type { HandleHost, Listener, Opens, Subscription } from "./handle.ts";
import { KeyValueStore, type KeyValueStoreHost } from "./key-value-store.ts";
import { subscribe, subscribePublished, type Listening } from "./listeners.ts";
import type { Logger } from "./logger.ts";
import { Opened } from "./opened.ts";
import type { RawSend } from "./raw-call.ts";
import { warnEntry } from "./markers.ts";
import { holdInRealm } from "./realm.ts";
import type { ChannelOpenOptions } from "./rpc.ts";
import { Sequence } from "./sequence.ts";
import type { SharedSubscription } from "./shared-subscription.ts";
import { fromWire, toWire, type WirePath } from "./wire.ts";

/** What a Run reaches its engine through: its Connection. */
export interface RunHost {
  /** The Connection's calls. */
  readonly calls: Caller<CollieOperationTypes>;
  /** The Connection's raw calls (ADR 0099), whose params and results it leaves as sent. */
  readonly raw: RawSend;
  /** Whether the Connection can still reach the engine. */
  reachable(): boolean;
  /** Sends a call the engine streams to, such as a listener's `on`. */
  stream(method: string, params: unknown, options: ChannelOpenOptions): Stream;
  /** The Connection's own Subscription, which its listeners of published events share. */
  readonly published: SharedSubscription;
  /** Forgets a closed Run. */
  forget(run: Run): void;
  /** Each operation's name in TypeScript form, which heads a call's `closed` (ADR 0079). */
  readonly operationNames: OperationNames;
  /** Its Key-value store's calls. */
  readonly keyValueStore: KeyValueStoreHost;
  /** The host's logger, which gets a failed listener its value never raised. */
  readonly logger: Logger | undefined;
}

/** What the handles of a Run reach the engine through: its host's calls, streams, Subscription and logger. */
type HandlesHost = Pick<RunHost, "calls" | "raw" | "stream" | "published" | "logger">;

/** A listener that threw, which `listener_failed` has not yet raised. */
interface ListenerFailure {
  /** The value it listened to. */
  readonly value: ValueScope | undefined;
  readonly error: ListenerFailedError;
}

/** What a Run and the calls made through it share: its ID, the operations' names its `closed` is headed with, where it stands, and its listeners that failed. */
interface RunScope {
  readonly id: string;
  /** Each operation's name in TypeScript form, which heads a call's `closed`. */
  readonly operationNames: OperationNames;
  state: "running" | "ended" | "closed";
  readonly failures: ListenerFailure[];
}

/** A value a program holds under a Run: a Session, a page, or a Locator built on one. */
interface ValueScope {
  /** The value that opened or built it, when the Run did not. */
  readonly parent: ValueScope | undefined;
  /** Whether its program closed it. */
  closed: boolean;
  /** Its `close` in flight. */
  closing: Promise<void> | undefined;
  /** Whether its handle is watched for release, which a listener on it asks for. */
  watched: boolean;
  /** Whether its program released its handle: a listener of it that throws then is logged at once. */
  released: boolean;
}

/** A value a Run, or `parent`, opens or builds. */
function valueIn(parent: ValueScope | undefined): ValueScope {
  return { parent, closed: false, closing: undefined, watched: false, released: false };
}

/** The `closed` a call `operation` fails with, in ADR 0079's layout. */
function closedIn(
  scope: RunScope,
  operation: string,
  whatFailed: string,
  fix: string,
): ClosedError {
  return new ClosedError(
    runtimeMessage(whatFailed, {
      operation: spelledOperation(scope.operationNames, operation),
      fix,
    }),
    { operation, fix },
  );
}

function ensureOpen(scope: RunScope, operation: string): void {
  if (scope.state === "closed") {
    throw closedIn(
      scope,
      operation,
      `the Run ${scope.id} is closed`,
      "Start the Run again by its Run ID to reach it.",
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

/** Whether `value` is `within`, or a value `within` opened or built. */
function isWithin(value: ValueScope | undefined, within: ValueScope | undefined): boolean {
  if (within === undefined) return true;
  for (let held = value; held !== undefined; held = held.parent) {
    if (held === within) return true;
  }
  return false;
}

/** Takes from `run` the failed listeners `which` picks. */
function takeFailures(
  run: RunScope,
  which: (failure: ListenerFailure) => boolean,
): ListenerFailedError[] {
  const taken = run.failures.filter(which);
  const kept = run.failures.filter((failure) => !which(failure));
  run.failures.splice(0, run.failures.length, ...kept);
  return taken.map((failure) => failure.error);
}

/** Logs `message` at error level; a logger that throws loses it, since the SDK has nowhere else to say so. */
function logError(
  logger: Logger | undefined,
  message: string,
  fields: Record<string, unknown>,
): void {
  try {
    logger?.error(message, fields);
  } catch {
    // Nowhere else to say it.
  }
}

/** Logs each of `errors` at error level. */
function logFailures(logger: Logger | undefined, errors: readonly ListenerFailedError[]): void {
  for (const error of errors) {
    logError(logger, error.message, {
      code: error.code,
      details: error.details,
      cause: error.cause,
    });
  }
}

/**
 * Raises the first failed listener of `value` or of a value it opened or
 * built, once `value` is closed (ADR 0081), and logs the others, which no
 * later call on a closed value could raise.
 */
function raiseFailuresWithin(
  run: RunScope,
  value: ValueScope | undefined,
  logger: Logger | undefined,
): void {
  const [first, ...rest] = takeFailures(run, (failure) => isWithin(failure.value, value));
  logFailures(logger, rest);
  if (first !== undefined) {
    throw first;
  }
}

/** Logs a failed listener of `value` that nothing raised, once its handle is released (ADR 0081). */
const released = new FinalizationRegistry<{
  readonly run: RunScope;
  readonly value: ValueScope;
  readonly logger: Logger | undefined;
}>(({ run, value, logger }) => {
  value.released = true;
  logFailures(
    logger,
    takeFailures(run, (failure) => failure.value === value),
  );
});

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
  reach: HandlesHost,
  run: RunScope,
  value: ValueScope | undefined,
  id: Id,
): HandleHost<Id, CollieOperationTypes> {
  const { calls } = reach;
  // A call on `value` is sent only once nothing stops it: a closed Run or
  // value, or a listener that threw of `value` or of the value that opened
  // or built it, which it raises in place.
  const ensureCallable = (operation: string): void => {
    ensureOpen(run, operation);
    if (isClosed(value)) {
      throw closedIn(
        run,
        operation,
        "the value it is called on is closed, or the one that opened it is",
        "Open another from its Run: a closed value never reopens.",
      );
    }
    const [failed] = takeFailures(
      run,
      (failure) => failure.value !== undefined && isWithin(value, failure.value),
    );
    if (failed !== undefined) {
      throw failed;
    }
  };
  // A listener is registered only where a call could be sent, and its
  // handle is then watched, so a failure nothing raised is logged once the
  // program releases it.
  const ensureListenable = (operation: string): void => {
    ensureCallable(operation);
    if (value !== undefined && !value.watched) {
      value.watched = true;
      released.register(host, { run, value, logger: reach.logger });
    }
  };
  // What tells `value` that a listener of `event` registered through `operation` threw.
  const failedListener = (operation: string, event: string, cause: unknown): void => {
    const error = new ListenerFailedError(
      `${operation}: the listener of \`${event}\` threw, so it was removed`,
      {
        fix: "Fix the listener, then listen again with `on`.",
        details: { event },
        cause,
      },
    );
    // Released first, nothing could raise it: it is logged.
    if (value?.released === true) {
      logFailures(reach.logger, [error]);
    } else {
      run.failures.push({ value, error });
    }
  };
  // What a listener of `event` registered through `operation` tells
  // `value`; with `opens`, it gets the value each event opens, whose own
  // listeners' failures `value` hears too.
  const listeningTo = <Payload>(
    operation: string,
    event: string,
    listener: Listener<Payload>,
    opens?: Opens<Payload>,
  ): Listening<Payload> => {
    const heard: Omit<Listening<Payload>, "listener" | "opened"> = {
      event,
      failed: (cause) => {
        failedListener(operation, event, cause);
      },
      // A call its Connection's close cancelled, or that stopped as its
      // value closed (a handler's attempt ending), ended with everything
      // else; any other error, such as `page_events_lost`, ended this
      // listener alone, and nothing else would say so.
      ended: (error) => {
        if (error instanceof CallCancelledError || error instanceof ClosedError) {
          return;
        }
        logError(reach.logger, `${operation}: the listener of \`${event}\` stopped`, {
          event,
          error,
        });
      },
    };
    if (opens === undefined) {
      return { ...heard, listener: readingWire(event, listener) };
    }
    const opened = new Opened(opens, readOnWire, (cause, thrower) => {
      failedListener(operation, thrower, cause);
    });
    return {
      ...heard,
      listener: (data: unknown, dropped) => {
        const payload = readOnWire(event, data);
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine payloads are not validated beyond `hello`'s protocol version (STYLE.md): the payload is the event's, which `opens` builds its value from.
        return listener(opens.build(payload as never, opened.open(payload)), dropped);
      },
      opened,
    };
  };
  // The Run's own calls also read their answer, since a Run report says when the Run ended.
  const sent = async <Result>(operation: string, send: () => Promise<Result>): Promise<Result> => {
    ensureCallable(operation);
    const result = await send();
    if (value === undefined && run.state === "running" && saysEnded(result)) {
      run.state = "ended";
    }
    return result;
  };
  const host: HandleHost<Id, CollieOperationTypes> = {
    id,
    call<Name extends keyof CollieOperationTypes>(
      operation: Name,
      params: CollieOperationTypes[Name]["params"],
      options: CallOptions,
    ): Promise<CollieOperationTypes[Name]["result"]> {
      return sent(operation, () => calls.call(operation, params, options));
    },
    raw: (operation, params, options) =>
      sent(operation, () => reach.raw(operation, params, options)),
    child: <ChildId>(childId: ChildId) => hostIn(reach, run, valueIn(value), childId),
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
      raiseFailuresWithin(run, value, reach.logger);
    },
    listen<Name extends keyof CollieOperationTypes, Payload>(
      operation: Name,
      params: CollieOperationTypes[Name]["params"],
      event: string,
      listener: Listener<Payload>,
      opens?: Opens<Payload>,
    ): Subscription {
      ensureListenable(operation);
      return subscribe(
        (options) => reach.stream(operation, paramsOnWire(operation, params), options),
        listeningTo(operation, event, listener, opens),
      );
    },
    listenPublished<Name extends keyof CollieOperationTypes, Payload>(
      operation: Name,
      params: CollieOperationTypes[Name]["params"],
      event: string,
      param: string,
      listener: Listener<Payload>,
    ): Subscription {
      ensureListenable(operation);
      return subscribePublished(
        (options) => reach.stream(operation, paramsOnWire(operation, params), options),
        reach.published,
        (data) =>
          typeof data === "object" &&
          data !== null &&
          param in data &&
          Reflect.get(data, param) === id,
        listeningTo(operation, event, listener),
      );
    },
  };
  return host;
}

/** `params` of `operation` as the engine takes them, each declared field in its spelling (ADR 0099). */
function paramsOnWire(operation: keyof CollieOperationTypes, params: unknown): unknown {
  return toWire(params, collieWire.params[operation], collieWire.types);
}

/**
 * `listener`, which reads the payload of `event`, as the engine names it,
 * with each declared field in camelCase and each point in time a `Date`
 * (ADRs 0092 and 0099); the payload's own data keeps its keys.
 */
function readingWire<Payload>(event: string, listener: Listener<Payload>): Listener<Payload> {
  const path = wirePathOf(event);
  return path === undefined
    ? listener
    : (payload, dropped) =>
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine payloads are not validated beyond `hello`'s protocol version (STYLE.md): the payload is the event's as the catalog types it.
        listener(fromWire(payload, path, collieWire.types) as Payload, dropped);
}

/** The payload of `event`, as the engine names it, read as `readingWire` reads it. */
function readOnWire(event: string, payload: unknown): unknown {
  return fromWire(payload, wirePathOf(event), collieWire.types);
}

/** How the payload of `event` is read, when it has fields to read. */
function wirePathOf(event: string): WirePath | undefined {
  const events: Readonly<Record<string, WirePath | undefined>> = collieWire.events;
  return Object.hasOwn(events, event) ? events[event] : undefined;
}

/** What stops a call given `given`, its own signal, that also stops when `signal` aborts. */
function alsoStoppedBy(given: AbortSignal | undefined, signal: AbortSignal): AbortSignal {
  return given === undefined ? signal : AbortSignal.any([given, signal]);
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
      return calls.call(operation, params, {
        signal: alsoStoppedBy(options.signal, signal),
        deadline: options.deadline,
      });
    },
  };
}

let pageIn: (run: Run, pageId: PageId, sessionId: SessionId, signal: AbortSignal) => Page;
let openIn: (run: Run, operation: string) => void;

/**
 * The page of ID `pageId` the engine lends a handler of `run` for one
 * attempt (ADR 0093), in its Session `sessionId`: a Collie `Page`, whose
 * calls fail with `closed` once the Run is, and each of which stops when
 * `signal`, the attempt's, aborts. Its `session()` is that Session, answered
 * without a call (ADR 0092), whose calls stop with `signal` too.
 */
export function pageOf(run: Run, pageId: PageId, sessionId: SessionId, signal: AbortSignal): Page {
  return pageIn(run, pageId, sessionId, signal);
}

/**
 * Fails `operation` with `closed` once `run` is closed, as each of a Run's
 * own calls does: for a call a Product's Run type adds, such as a crawl's
 * `records()`.
 */
export function ensureRunOpen(run: Run, operation: string): void {
  openIn(run, operation);
}

/** A Run its program drives, opened by `start()` on a Connection. */
export class Run extends RunMembers implements AsyncDisposable {
  static {
    openIn = (run, operation) => {
      ensureOpen(run.#scope, operation);
    };
    pageIn = (run, pageId, sessionId, signal) => {
      const sessionHost = hostIn(
        {
          calls: signalled(run.#host.calls, signal),
          raw: (operation, params, options) =>
            run.#host.raw(operation, params, { signal: alsoStoppedBy(options.signal, signal) }),
          stream: (method, params, options) =>
            run.#host.stream(method, params, {
              ...options,
              signal: alsoStoppedBy(options.signal, signal),
            }),
          published: run.#host.published,
          logger: run.#host.logger,
        },
        run.#scope,
        valueIn(undefined),
        sessionId,
      );
      return new Page(sessionHost.child(pageId), new Session(sessionHost));
    };
  }

  readonly #host: RunHost;
  readonly #scope: RunScope;
  /** Whether the Run outlives its program (ADR 0102): its close then frees only the value. */
  readonly #detached: boolean;
  #closing: Promise<void> | undefined;
  /** The Run's Key-value store (ADR 0103), kept in its run state across a resume. */
  readonly keyValueStore: KeyValueStore;

  /**
   * @internal The Connection's start opens a Run; a program never
   * constructs one. A `detached` start's Run outlives its program.
   */
  constructor(host: RunHost, started: RunStarted, detached = false) {
    const scope: RunScope = {
      id: started.runId,
      operationNames: host.operationNames,
      state: started.lifecycle === "running" ? "running" : "ended",
      failures: [],
    };
    super(hostIn(host, scope, undefined, started.runId), started);
    this.#host = host;
    this.#scope = scope;
    this.#detached = detached;
    this.keyValueStore = new KeyValueStore(host.keyValueStore, scope.id, (operation) => {
      ensureOpen(scope, operation);
    });
    holdInRealm(this);
  }

  /** The Run's ID, which names it for good. Read without a round trip. */
  id(): string {
    return this.#scope.id;
  }

  /**
   * Shows each setting's effective value, the layer that set it and the
   * layer that locked it, from the built-in defaults, the Operator policy,
   * user configuration, the project configuration given and the run layer:
   * the Connection's own settings with the run values given over them, or a
   * live Run's as it opened when `run_id` names one; and lists the keys only
   * the other Product honours as that Product's.
   *
   * What the Run runs with (ADR 0083), its own `settings` over its
   * Connection's, named by its Run ID: each setting under `keys`, each a
   * key or a prefix of keys such as `timeout`, every one when left out,
   * with the origin that set it too, as `goodfetch config explain` shows
   * them. A Run that is no longer live fails with `run_not_found`, and a
   * closed Run's value with `closed`.
   */
  async explainSettings(keys?: readonly string[]): Promise<SettingsExplanation> {
    ensureOpen(this.#scope, "config.explain");
    return this.#host.calls.call(
      "config.explain",
      { runId: this.#scope.id, ...(keys === undefined ? {} : { keys: [...keys] }) },
      {},
    );
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
   * Read with `for await` (ADR 0102): its first read, without `after`, is
   * sent when it is called, and each next one, after the last value read,
   * once the program asks for it, so values it did not read in time
   * coalesce; `stopReason` is the value's `stop_reason`. `break`,
   * `return()`, `close()`, `using`, the signal or releasing it unread
   * cancel its read at once. On a closed Run it throws `closed`, sending
   * nothing.
   *
   * @experimental
   */
  progress(options: { readonly signal?: AbortSignal | undefined } = {}): Sequence<RunProgress> {
    warnEntry(collieOperations, "run.progress");
    ensureOpen(this.#scope, "run.progress");
    const scope = this.#scope;
    const calls = this.#host.calls;
    let take: (() => RunProgress | undefined) | undefined;
    return new Sequence({
      open: (opened) => {
        const reading = followed<RunProgress>(
          {
            read: async (after, signal) => {
              ensureOpen(scope, "run.progress");
              const params = { runId: scope.id, ...(after === undefined ? {} : { after }) };
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
      // One read at a time, the next sent once the program read the last.
      credits: 1,
      regrant: 1,
      signal: options.signal,
    });
  }

  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint.
   * `finish()` ends it.
   *
   * Closes the Run's value; idempotent. A Run still running is stopped
   * first, `stopped` and `incomplete` as an abort by its Client, and can be
   * resumed by starting its Run ID again; a Run started `detached` is not
   * stopped, since it outlives its program. A listener of one of its values
   * that threw and that nothing raised yet is raised then, as
   * `listener_failed`, once the Run is closed.
   *
   * Stops a Run at its next safe point, `stopped` and `incomplete`: no
   * attempt in flight commits, and its work items return to pending.
   * Answers its Run report; an ended Run's report is answered as it is. A
   * Run whose engine died is stopped by this engine, which takes its writer
   * role at the next run epoch; a Run another engine writes is stopped by
   * that engine, the owner the Run index names.
   *
   * @experimental
   */
  async close(): Promise<void> {
    warnEntry(collieOperations, "run.abort");
    this.#closing ??= this.#close();
    await this.#closing;
    // After the close, so each failure is raised once, by whichever close takes it.
    raiseFailuresWithin(this.#scope, undefined, this.#host.logger);
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  async #close(): Promise<void> {
    const stops = this.#scope.state === "running" && !this.#detached;
    this.#scope.state = "closed";
    try {
      if (stops && this.#host.reachable()) {
        await this.#host.calls.call("run.abort", { runId: this.#scope.id }, {});
      }
    } finally {
      this.#host.forget(this);
    }
  }
}
