// Request correlation and cancellation over one transport (ADR 0038): any
// number of calls in flight, each answered by its id; an aborted call's
// `$/cancel`; the requests the engine sends its Client, such as Foxhound's
// `crawl_request.handle`, each answered by what its owner serves, and
// stopped by the engine's `$/cancel`; and the end of the transport, which
// fails every call still waiting with the reason the Connection gives and
// stops every request still served. What waits is bounded
// (ADR 0081: no SDK keeps a queue of calls): past its bound on calls waiting
// or on bytes waiting to be written, a new call is refused at once.
import { replyOfError } from "./errors.ts";
import { inCall, type OperationNames } from "./goodfetch-error.ts";
import { encodeMessage, readLines, type Message, type MessageId } from "./framing.ts";
import {
  CallCancelledError,
  ConnectionOverloadedError,
  SettingInvalidError,
  type GoodfetchError,
} from "./generated/index.ts";

/** The notification that cancels a call in flight (ADR 0038), `goodfetch-protocol`'s `connection::CANCEL`. */
const CANCEL = "$/cancel";

/**
 * The bytes a Connection runs over: the engine's to read and the Client's
 * to write, newline-delimited JSON-RPC each way. An Attached engine's stdio
 * gives one, and so does the Scripted engine's in-process player.
 */
export type Transport = ReadableWritablePair<Uint8Array, Uint8Array>;

/** How the engine's side of a transport ended. */
export type TransportEnd =
  /** Its bytes ended. */
  | { readonly kind: "ended" }
  /** Reading them failed. */
  | { readonly kind: "failed"; readonly cause: unknown }
  /** The engine's side rejected a write, whatever its bytes to read still do. */
  | { readonly kind: "write_failed"; readonly cause: unknown }
  /** It sent a line that is no JSON-RPC message. */
  | { readonly kind: "invalid"; readonly line: string };

/** What a channel's owner decides about its end and its replies. */
export interface ChannelOwner {
  /**
   * The error every waiting and later call fails with once the transport
   * ended, which each such call gets a copy of headed by its operation.
   */
  ended(end: TransportEnd): Promise<GoodfetchError>;
  /**
   * Each operation's name in TypeScript form, which heads the runtime's own
   * errors of a call (ADR 0079); a call whose operation it lacks is headed
   * by its catalog name.
   */
  readonly operationNames: OperationNames;
  /** The error a JSON-RPC `error` reply raises. */
  failure(error: unknown): GoodfetchError;
  /**
   * Serves a request the engine sent, such as Foxhound's
   * `crawl_request.handle`: answers its result, or rejects with the error
   * its reply carries. `signal` aborts once the engine cancels the request
   * or the transport ends. Answers `undefined` for a method the Client does
   * not serve, which is replied to as JSON-RPC's unknown method; when left
   * out, every request is.
   */
  readonly serve?:
    | ((method: string, params: unknown, signal: AbortSignal) => Promise<unknown> | undefined)
    | undefined;
  /**
   * Hears each notification that names no call, such as the engine's
   * `$/log`; when left out, they are dropped.
   */
  readonly notified?: ((method: string, params: unknown) => void) | undefined;
}

/** What a Connection lets wait before it refuses a new call with `connection_overloaded`. */
export interface ChannelBounds {
  /** How many calls may wait for their answer at once; 10,000 when left out. */
  readonly maxPendingCalls?: number | undefined;
  /**
   * How many bytes may wait to be written to the engine; 64 MiB when left
   * out. One message larger than this is still taken while nothing waits.
   */
  readonly maxQueuedWriteBytes?: number | undefined;
}

/**
 * One of the Client's own bounds, a setting of `connect()`'s own `settings`
 * alone (issue 2518): its key as the wire and the engine's details write
 * it, as a program writes it, and its default.
 */
export interface Bound {
  readonly key: string;
  readonly spelled: string;
  readonly fallback: number;
}

/** The bound on calls waiting for their answer. */
export const PENDING_CALLS: Bound = {
  key: "connection.max_pending_calls",
  spelled: "connection.maxPendingCalls",
  fallback: 10_000,
};

/** The bound on bytes waiting to be written. */
export const QUEUED_WRITE_BYTES: Bound = {
  key: "connection.max_queued_write_bytes",
  spelled: "connection.maxQueuedWriteBytes",
  fallback: 64 * 1024 * 1024,
};

/** A Connection's bounds, each given or its default. */
export interface CheckedBounds {
  readonly maxPendingCalls: number;
  readonly maxQueuedWriteBytes: number;
}

/**
 * `bounds` with each default filled in.
 *
 * @throws {SettingInvalidError} when a bound is no whole number of 1 or
 * more: such a bound would refuse every call, or none.
 */
export function boundsOf(bounds: ChannelBounds): CheckedBounds {
  return {
    maxPendingCalls: wholeBound(PENDING_CALLS, bounds.maxPendingCalls),
    maxQueuedWriteBytes: wholeBound(QUEUED_WRITE_BYTES, bounds.maxQueuedWriteBytes),
  };
}

/**
 * The Connection's `bound`, `given` or its default when left out.
 *
 * @throws {SettingInvalidError} when it is no whole number of 1 or more.
 */
export function wholeBound(bound: Bound, given: unknown): number {
  const value = given ?? bound.fallback;
  if (typeof value === "number" && Number.isSafeInteger(value) && value >= 1) {
    return value;
  }
  const shown = typeof value === "number" ? String(value) : (JSON.stringify(value) ?? typeof value);
  const reason = `${shown} is no whole number of 1 or more`;
  throw new SettingInvalidError(`\`${bound.spelled}\` is not valid: ${reason}`, {
    fix: `Set \`${bound.spelled}\` in connect()'s settings to a whole number of 1 or more, or leave it out for its default of ${bound.fallback}.`,
    details: { key: bound.key, layer: "run", reason },
  });
}

interface Waiting {
  /** The call's operation, by its catalog name. */
  readonly method: string;
  resolve(result: unknown): void;
  reject(error: unknown): void;
  /** Hears the notifications the engine sends for this call, such as a stream's `$/events`. */
  notified: ((method: string, params: unknown) => void) | undefined;
}

/** The options of one call. */
export interface ChannelCallOptions {
  /** Cancels the call in flight, rejecting it with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** The options of a call that stays in flight while the engine streams to it. */
export interface ChannelOpenOptions extends ChannelCallOptions {
  /** Hears each notification whose params name the call's id, such as `$/events` or `$/chunk`. */
  readonly notified?: ((method: string, params: unknown) => void) | undefined;
}

/** A call in flight: its id, which its notifications and credits name, and its answer. */
export interface OpenCall {
  readonly id: MessageId;
  readonly result: Promise<unknown>;
}

/** One transport's calls. */
export class Channel {
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>;
  readonly #reader: ReadableStreamDefaultReader<Uint8Array>;
  readonly #owner: ChannelOwner;
  readonly #waiting = new Map<MessageId, Waiting>();
  /** The engine's requests being served, each stopped by its `$/cancel`. */
  readonly #serving = new Map<MessageId, AbortController>();
  #nextId = 0;
  #failure: GoodfetchError | undefined;
  /** The first write the engine's side rejected, which ends the transport. */
  #writeFailure: { readonly cause: unknown } | undefined;
  readonly #bounds: CheckedBounds;
  /** How stopping the engine's bytes went, once it was asked for. */
  #readingStopped: Promise<{ readonly cause: unknown } | undefined> | undefined;
  /** The bytes handed to the writer that it has not written yet. */
  #queuedWriteBytes = 0;
  /** Settles once the engine's side has ended and every waiting call failed. */
  readonly ended: Promise<void>;

  constructor(transport: Transport, owner: ChannelOwner, bounds: ChannelBounds = {}) {
    this.#bounds = boundsOf(bounds);
    this.#writer = transport.writable.getWriter();
    this.#owner = owner;
    this.#reader = transport.readable.getReader();
    this.ended = this.#read();
  }

  /** Sends `method` and answers its result, or fails with its error. */
  async call(method: string, params: unknown, options: ChannelCallOptions = {}): Promise<unknown> {
    return this.open(method, params, options).result;
  }

  /**
   * Sends `method` and answers its id at once, with its result to come:
   * a call that stays in flight while the engine streams to it, such as
   * `events.subscribe`, whose notifications `notified` hears.
   */
  open(method: string, params: unknown, options: ChannelOpenOptions = {}): OpenCall {
    const { signal } = options;
    const id = this.#nextId;
    this.#nextId += 1;
    if (signal?.aborted === true) {
      return { id, result: Promise.reject(signal.reason) };
    }
    if (this.#failure !== undefined) {
      return { id, result: Promise.reject(this.#inCall(this.#failure, method)) };
    }
    // Encoded before the call waits, so params that cannot be encoded, such
    // as a BigInt, fail this call alone and leave no waiter behind.
    let request: Uint8Array;
    try {
      request = encodeMessage({ id, method, params });
    } catch (error) {
      return { id, result: Promise.reject(error) };
    }
    const overloaded = this.#overloadedBy(request.length);
    if (overloaded !== undefined) {
      return { id, result: Promise.reject(overloaded) };
    }
    const answer = new Promise<unknown>((resolve, reject) => {
      this.#waiting.set(id, { method, resolve, reject, notified: options.notified });
    });
    const abort = (): void => {
      const waiting = this.#waiting.get(id);
      if (waiting !== undefined && signal !== undefined) {
        this.#waiting.delete(id);
        this.#send({ method: CANCEL, params: { id } });
        waiting.reject(signal.reason);
      }
    };
    signal?.addEventListener("abort", abort, { once: true });
    this.#write(request);
    const result = answer.finally(() => signal?.removeEventListener("abort", abort));
    return { id, result };
  }

  /** Sends the notification `method`, such as `$/credit`. */
  notify(method: string, params: unknown): void {
    if (this.#failure === undefined) {
      this.#send({ method, params });
    }
  }

  /** Fails every waiting call with its copy of `error`, sending each its `$/cancel`. */
  cancelAll(error: GoodfetchError): void {
    for (const [id, waiting] of this.#waiting) {
      this.#send({ method: CANCEL, params: { id } });
      waiting.reject(this.#inCall(error, waiting.method));
    }
    this.#waiting.clear();
  }

  /** Ends the Client's side; the engine's ends when the engine stops. */
  async close(): Promise<void> {
    try {
      await this.#writer.close();
    } catch {
      // The engine's side already ended, which `ended` reports.
    }
  }

  /**
   * The error a new call of `length` bytes is refused with, when a bound is
   * reached. Notifications are never refused: each belongs to a call
   * already counted, and a `$/cancel` is what frees one.
   */
  #overloadedBy(length: number): ConnectionOverloadedError | undefined {
    const { maxPendingCalls, maxQueuedWriteBytes } = this.#bounds;
    if (this.#waiting.size >= maxPendingCalls) {
      return new ConnectionOverloadedError(
        `the Connection already holds ${this.#waiting.size} calls waiting for their answer, its bound, so this call was not sent`,
        {
          fix: `Wait for earlier calls to end, or raise \`${PENDING_CALLS.spelled}\` in connect()'s settings.`,
          details: { bound: "pending_calls", value: maxPendingCalls },
        },
      );
    }
    const queued = this.#queuedWriteBytes;
    if (queued > 0 && queued + length > maxQueuedWriteBytes) {
      return new ConnectionOverloadedError(
        `the Connection already holds ${queued} bytes waiting to be written to the engine, and this call's ${length} would pass its bound of ${maxQueuedWriteBytes}, so it was not sent`,
        {
          fix: `Wait for the engine to read earlier calls, or raise \`${QUEUED_WRITE_BYTES.spelled}\` in connect()'s settings.`,
          details: { bound: "queued_write_bytes", value: maxQueuedWriteBytes },
        },
      );
    }
    return undefined;
  }

  /**
   * Ends the Client's side without waiting for what is still to be written,
   * which is dropped, and stops reading the engine's bytes, so `ended`
   * settles though the engine's side never ends them.
   */
  abandon(): void {
    // A transport that fails to drop its writes ends as one that rejected one.
    void this.#writer.abort().catch((cause: unknown) => {
      this.#writeFailure ??= { cause };
    });
    void this.#stopReading(undefined);
  }

  /**
   * Stops reading the engine's bytes, once, for `reason`: answers the cause
   * when the transport failed to stop them, which `ended` reports.
   */
  #stopReading(reason: unknown): Promise<{ readonly cause: unknown } | undefined> {
    this.#readingStopped ??= this.#reader.cancel(reason).then(
      () => undefined,
      (cause: unknown) => ({ cause }),
    );
    return this.#readingStopped;
  }

  #send(message: Readonly<Record<string, unknown>>): void {
    this.#write(encodeMessage(message));
  }

  #write(bytes: Uint8Array): void {
    this.#queuedWriteBytes += bytes.length;
    this.#writer
      .write(bytes)
      .finally(() => {
        this.#queuedWriteBytes -= bytes.length;
      })
      .catch((cause: unknown) => {
        // The engine's bytes to read may stay open for ever after a rejected
        // write, so the write ends the transport itself: reading stops, and
        // `ended` reports the first rejection's cause to every waiting call.
        this.#writeFailure ??= { cause };
        void this.#stopReading(cause);
      });
  }

  /** Each chunk of the engine's bytes, until they end or reading is cancelled. */
  async *#chunks(): AsyncGenerator<Uint8Array> {
    for (;;) {
      const { done, value } = await this.#reader.read();
      if (done) {
        return;
      }
      yield value;
    }
  }

  async #read(): Promise<void> {
    let end: TransportEnd = { kind: "ended" };
    try {
      for await (const line of readLines(this.#chunks())) {
        if (line.kind === "invalid") {
          end = { kind: "invalid", line: line.line };
          break;
        }
        this.#receive(line.message);
      }
    } catch (cause) {
      end = { kind: "failed", cause };
    }
    // Whatever ended the reading, the engine's bytes are let go of: a line
    // that is no message leaves them open otherwise.
    const unstopped = await this.#stopReading(undefined);
    if (end.kind === "ended" && this.#writeFailure !== undefined) {
      end = { kind: "write_failed", cause: this.#writeFailure.cause };
    } else if (end.kind === "ended" && unstopped !== undefined) {
      end = { kind: "failed", cause: unstopped.cause };
    }
    const failure = await this.#owner.ended(end);
    this.#failure = failure;
    for (const waiting of this.#waiting.values()) {
      waiting.reject(this.#inCall(failure, waiting.method));
    }
    this.#waiting.clear();
    // No answer reaches the engine any more: what serves its requests stops.
    for (const serving of this.#serving.values()) {
      serving.abort(failure);
    }
    this.#serving.clear();
  }

  /**
   * Serves the engine's request `id` through the owner, and replies with
   * its answer, unless the engine cancelled it first. A method the owner
   * does not serve is answered as JSON-RPC's unknown method, so the engine
   * never waits on it.
   */
  #serve(id: MessageId, method: string, params: unknown): void {
    const serving = new AbortController();
    const served = this.#owner.serve?.(method, params, serving.signal);
    if (served === undefined) {
      this.#send({ id, error: { code: -32601, message: `this Client serves no \`${method}\`` } });
      return;
    }
    this.#serving.set(id, serving);
    void served.then(
      (result: unknown) => this.#answerServed(id, serving, { id, result }),
      (error: unknown) => this.#answerServed(id, serving, { id, error: replyOfError(error) }),
    );
  }

  /** Sends a served request's answer, unless it was cancelled or the transport ended first. */
  #answerServed(
    id: MessageId,
    serving: AbortController,
    answer: Readonly<Record<string, unknown>>,
  ): void {
    if (this.#serving.get(id) !== serving) {
      return;
    }
    this.#serving.delete(id);
    this.#send(answer);
  }

  /** `error`, which the runtime raised, as the call `method` fails with it. */
  #inCall(error: GoodfetchError, method: string): GoodfetchError {
    return inCall(error, method, this.#owner.operationNames);
  }

  /** Stops the served request the engine cancelled, and answers it `call_cancelled` (ADR 0038). */
  #cancelServed(id: MessageId): void {
    const serving = this.#serving.get(id);
    if (serving === undefined) {
      return;
    }
    this.#serving.delete(id);
    const cancelled = new CallCancelledError("the engine cancelled its request", {
      details: { by: "engine" },
    });
    serving.abort(cancelled);
    this.#send({ id, error: replyOfError(cancelled) });
  }

  #receive(message: Message): void {
    switch (message.kind) {
      case "result":
      case "error": {
        const waiting = message.id === null ? undefined : this.#waiting.get(message.id);
        if (waiting === undefined || message.id === null) {
          return;
        }
        this.#waiting.delete(message.id);
        if (message.kind === "result") {
          waiting.resolve(message.result);
        } else {
          waiting.reject(this.#owner.failure(message.error));
        }
        return;
      }
      case "request":
        this.#serve(message.id, message.method, message.params);
        return;
      case "notification": {
        const params = message.params;
        const named =
          typeof params === "object" && params !== null && "id" in params ? params.id : undefined;
        const id = typeof named === "number" || typeof named === "string" ? named : undefined;
        if (id === undefined) {
          this.#owner.notified?.(message.method, params);
          return;
        }
        // The engine's `$/cancel` names a request of its own, whose ids are
        // counted apart from the Client's calls.
        if (message.method === CANCEL) {
          this.#cancelServed(id);
          return;
        }
        this.#waiting.get(id)?.notified?.(message.method, params);
        return;
      }
      default: {
        const unknown: never = message;
        return unknown;
      }
    }
  }
}
