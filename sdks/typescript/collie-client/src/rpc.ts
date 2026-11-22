// Request correlation and cancellation over one transport (ADR 0038): any
// number of calls in flight, each answered by its id; an aborted call's
// `$/cancel`; the requests the engine sends its Client, such as Foxhound's
// `crawl_request.handle`, each answered by what its owner serves, and
// stopped by the engine's `$/cancel`; and the end of the transport, which
// fails every call still waiting with the reason the Connection gives and
// stops every request still served. At most the engine's cap of calls,
// which `hello`'s result advertises, are in flight; the rest wait in a
// queue, sent in the order they were made as answers free their places,
// each failing alone when its own timeout passes or its signal aborts
// there, never sent (ADR 0038). What waits is bounded: past its bound on
// calls waiting, sent or queued, or on bytes waiting to be written, a new
// call is refused at once. Once told how (`relinkWith`), as a remote
// Daemon's Connection is, a Channel whose link drops reaches its engine
// again by itself, within `timeout.connect` (spec 606): it holds every call
// meanwhile, greets the new link ahead of them, then sends again, by the
// same id, each call in flight at the drop that is a read or an idempotent
// write, and a stream it can resume; every other one fails with
// `connection_lost`, never sent again (ADR 0079).
import type { CallDeadline } from "./caller.ts";
import { replyOfError } from "./errors.ts";
import { inCall, type ErrorLimit, type OperationNames } from "./goodfetch-error.ts";
import { encodeMessage, readLines, type Message, type MessageId } from "./framing.ts";
import {
  CallCancelledError,
  ConnectionLostError,
  ConnectionOverloadedError,
  DeadlineExceededError,
  GoodfetchError,
  SettingInvalidError,
} from "./generated/index.ts";
import { reconnectDelay } from "./reconnect.ts";

/** The notification that cancels a call in flight (ADR 0038), `goodfetch-protocol`'s `connection::CANCEL`. */
const CANCEL = "$/cancel";

/** The longest delay a timer keeps, in milliseconds: a longer one fires at once. */
const LONGEST_TIMER = 2 ** 31 - 1;

/**
 * The `deadline_exceeded` of a phase `limit` bounds, whose wait for
 * `waitedFor` started at `startedAt` (ADR 0080).
 */
export function phaseExceeded(
  message: string,
  fix: string,
  limit: ErrorLimit,
  startedAt: number,
  waitedFor: "hello" | "close" | "calls_in_flight",
): DeadlineExceededError {
  return new DeadlineExceededError(message, {
    code: "deadline_exceeded",
    group: "deadline_exceeded",
    retryable: true,
    fix,
    limit,
    details: {
      elapsed: Date.now() - startedAt,
      waited_for: waitedFor,
    },
  });
}

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

/** What an operation is to a reconnect (ADR 0011): a read or an idempotent write is sent again, an Effect never (ADR 0079). */
export type OperationClass = "read" | "idempotent_write" | "effect";

/** Each operation's class, by its catalog name. */
export type OperationClasses = Readonly<Record<string, OperationClass>>;

/** How a Channel reaches its engine again once its link dropped (spec 606). */
export interface ChannelRelinking {
  /** Whether a call of `method` in flight at the drop is sent again: a read's or an idempotent write's (ADR 0079). */
  resends(method: string): boolean;
  /** How long reaching the engine again may take, counted from the drop, in milliseconds: `timeout.connect`, or `null` for no bound. */
  readonly within: number | null;
  /** Aborts once the Connection closes, which ends the Channel as though it had not reconnected. */
  readonly signal: AbortSignal;
  /** Opens a new link to the engine; `signal` aborts once the attempt is given up. */
  reopen(signal: AbortSignal): Promise<Transport>;
  /**
   * Greets the engine over a new link, as `hello` does, through `call`,
   * whose calls go ahead of every call held meanwhile and are never sent
   * again. Rejecting gives up reaching the engine, with that error.
   */
  greet(call: (method: string, params: unknown) => Promise<unknown>): Promise<void>;
  /**
   * The error every waiting and later call fails with once no attempt
   * reached the engine within `within`: `cause` is why the last one failed.
   */
  gaveUp(cause: unknown): GoodfetchError;
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
  /** What it is sent again with on a new link; `undefined` when it never is. */
  readonly again: (() => Resent) | undefined;
  /** Its own timeout, which fails it while the Client holds it back; `undefined` for none. */
  readonly deadline: HeldDeadline | undefined;
}

/** A call's own timeout, counted from when the call was made. */
interface HeldDeadline {
  readonly limit: { readonly key: string; readonly value: number };
  readonly startedAt: number;
}

/** `deadline` counted from now, or `undefined` when it bounds nothing. */
function heldDeadline(deadline: CallDeadline | undefined): HeldDeadline | undefined {
  const value = deadline?.value;
  return deadline === undefined || value === undefined || value === null
    ? undefined
    : { limit: { key: deadline.key, value }, startedAt: Date.now() };
}

/** A call as it is sent again on a new link: its request, then the notifications naming it. */
interface Resent {
  readonly request: Uint8Array;
  readonly notes: Uint8Array[];
}

/** What a stream is sent again with on a new link, such as `events.subscribe` after the last event read. */
export interface Resumed {
  readonly params: unknown;
  /** The credits the stream still had at the drop, granted again with `$/credit`. */
  readonly credits: number;
}

/** The options of one call. */
export interface ChannelCallOptions {
  /** Cancels the call in flight, rejecting it with the signal's reason; a queued call is never sent. */
  readonly signal?: AbortSignal | undefined;
  /**
   * The call's own timeout, which fails it with `deadline_exceeded` while
   * the Client holds it back: in the queue, or while it reaches the engine
   * again.
   */
  readonly deadline?: CallDeadline | undefined;
}

/** A call the Client holds back while the engine's cap of calls is in flight. */
interface Queued {
  /** Its encoded request, sent once a place frees. */
  readonly request: Uint8Array;
  /** Fails it once its own timeout passes in the queue. */
  timer: ReturnType<typeof setTimeout> | undefined;
  /** The encoded notifications naming it, such as its `$/credit`, sent right after it. */
  readonly notes: Uint8Array[];
}

/** The id a notification's params name, such as a `$/credit`'s call. */
function namedId(params: unknown): MessageId | undefined {
  const named =
    typeof params === "object" && params !== null && "id" in params ? params.id : undefined;
  return typeof named === "number" || typeof named === "string" ? named : undefined;
}

/** The options of a call that stays in flight while the engine streams to it. */
export interface ChannelOpenOptions extends ChannelCallOptions {
  /** Hears each notification whose params name the call's id, such as `$/events` or `$/chunk`. */
  readonly notified?: ((method: string, params: unknown) => void) | undefined;
  /**
   * What the stream is sent again with on a new link, read at the drop.
   * A stream without one is never sent again: its engine would stream it
   * from its start.
   */
  readonly resume?: (() => Resumed) | undefined;
}

/** The ends of one link to the engine, and how they went. */
interface Link {
  readonly writer: WritableStreamDefaultWriter<Uint8Array>;
  readonly reader: ReadableStreamDefaultReader<Uint8Array>;
  /** The first write the engine's side rejected, which ends the link. */
  writeFailure: { readonly cause: unknown } | undefined;
  /** How stopping the engine's bytes went, once it was asked for. */
  readingStopped: Promise<{ readonly cause: unknown } | undefined> | undefined;
}

function linkOf(transport: Transport): Link {
  return {
    writer: transport.writable.getWriter(),
    reader: transport.readable.getReader(),
    writeFailure: undefined,
    readingStopped: undefined,
  };
}

/** How an attempt to reach the engine again went. */
type Attempt =
  /** The new link is greeted, and read until `reading` settles with its end. */
  | { readonly kind: "greeted"; readonly reading: Promise<TransportEnd> }
  /** The greeting was refused: no attempt follows. */
  | { readonly kind: "refused"; readonly failure: GoodfetchError }
  /** The new link sent a line that is no message, the engine's fault: no attempt follows. */
  | { readonly kind: "invalid"; readonly end: TransportEnd }
  /** No link came of it, for `cause`. */
  | { readonly kind: "failed"; readonly cause: unknown };

/** Why the engine's side of a link ended, as a cause. */
function causeOf(end: TransportEnd): unknown {
  switch (end.kind) {
    case "ended":
      return undefined;
    case "failed":
    case "write_failed":
      return end.cause;
    case "invalid":
      return new TypeError(`the engine sent a line that is no JSON-RPC message: ${end.line}`);
    default: {
      const unknown: never = end;
      return unknown;
    }
  }
}

/** Waits `delay` milliseconds: `false` once `signal` aborted first. */
async function waited(delay: number, signal: AbortSignal): Promise<boolean> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve(false);
      return;
    }
    const stop = (): void => {
      clearTimeout(timer);
      resolve(false);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", stop);
      resolve(true);
    }, delay);
    signal.addEventListener("abort", stop, { once: true });
  });
}

/** Settles with `signal`'s reason once it aborts. */
async function abortedWith(signal: AbortSignal): Promise<unknown> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve(signal.reason);
      return;
    }
    signal.addEventListener("abort", () => resolve(signal.reason), { once: true });
  });
}

/**
 * The error a call in flight when its link dropped fails with, never sent
 * again: an Effect, whose journal says whether it happened (ADR 0079).
 */
function lostCall(): ConnectionLostError {
  return new ConnectionLostError(
    "the Connection to the engine dropped before the call was answered, so whether it took effect is unknown, and it was not sent again",
    {
      fix: "Check the Run's Effect journal for whether it happened before making the call again: the Client reaches the engine again by itself.",
    },
  );
}

/** The error a stream open when its link dropped fails with, when it cannot be resumed. */
function lostStream(): ConnectionLostError {
  return new ConnectionLostError(
    "the Connection to the engine dropped while the stream was open, and it cannot be resumed, so it was not sent again",
    { fix: "Open the stream again: the Client reaches the engine again by itself." },
  );
}

/** A call in flight: its id, which its notifications and credits name, and its answer. */
export interface OpenCall {
  readonly id: MessageId;
  readonly result: Promise<unknown>;
}

/** One transport's calls. */
export class Channel {
  /** The link calls are sent over: the first, then each one reached again. */
  #link: Link;
  /** How the Channel reaches its engine again once its link dropped; never when left out. */
  #relinking: ChannelRelinking | undefined;
  /** Whether its calls are held, from a drop until a new link is greeted. */
  #held = false;
  readonly #owner: ChannelOwner;
  readonly #waiting = new Map<MessageId, Waiting>();
  /**
   * The calls sent and not answered yet, a cancelled one included until
   * the engine answers its `$/cancel`, so they never pass its cap.
   */
  readonly #inFlight = new Set<MessageId>();
  /** The calls held back past the cap, in the order they were made. */
  readonly #queued = new Map<MessageId, Queued>();
  /** How many calls the engine keeps in flight, none until `hello`'s result names it. */
  #cap = Number.POSITIVE_INFINITY;
  /** The engine's requests being served, each stopped by its `$/cancel`. */
  readonly #serving = new Map<MessageId, AbortController>();
  #nextId = 0;
  #failure: GoodfetchError | undefined;
  readonly #bounds: CheckedBounds;
  /** The bytes handed to the writer that it has not written yet, and those of the calls held back. */
  #queuedWriteBytes = 0;
  /**
   * Settles once the engine's side has ended, with no new link reached,
   * and every waiting call failed.
   */
  readonly ended: Promise<void>;

  constructor(transport: Transport, owner: ChannelOwner, bounds: ChannelBounds = {}) {
    this.#bounds = boundsOf(bounds);
    this.#owner = owner;
    this.#link = linkOf(transport);
    this.ended = this.#run(this.#read(this.#link));
  }

  /**
   * Reaches the engine again through `relinking` whenever the link drops
   * from now on, sending again the calls made from now on that it says
   * may be (spec 606). Without it, the Channel ends with its link.
   */
  relinkWith(relinking: ChannelRelinking): void {
    this.#relinking = relinking;
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
    const again = this.#againOf(id, method, request, options);
    const deadline = heldDeadline(options.deadline);
    const answer = new Promise<unknown>((resolve, reject) => {
      this.#waiting.set(id, {
        method,
        resolve,
        reject,
        notified: options.notified,
        again,
        deadline,
      });
    });
    const abort = (): void => {
      const waiting = this.#waiting.get(id);
      if (waiting !== undefined && signal !== undefined) {
        this.#waiting.delete(id);
        // A queued call leaves the queue unsent; a sent one is cancelled.
        if (this.#unqueue(id) === undefined) {
          this.#send({ method: CANCEL, params: { id } });
        }
        waiting.reject(signal.reason);
      }
    };
    signal?.addEventListener("abort", abort, { once: true });
    if (!this.#held && this.#inFlight.size < this.#cap) {
      this.#dispatch(id, request);
    } else {
      this.#enqueue(id, method, { request, timer: undefined, notes: [] }, deadline);
    }
    const result = answer.finally(() => signal?.removeEventListener("abort", abort));
    return { id, result };
  }

  /**
   * How the call `id` of `method` is sent again on a new link: as it was,
   * or a stream with what `resume` gives; `undefined` when the Channel does
   * not reach its engine again, or the call is never sent again.
   */
  #againOf(
    id: MessageId,
    method: string,
    request: Uint8Array,
    { notified, resume }: ChannelOpenOptions,
  ): (() => Resent) | undefined {
    if (this.#relinking?.resends(method) !== true) {
      return undefined;
    }
    if (resume !== undefined) {
      return () => {
        const { params, credits } = resume();
        const notes =
          credits > 0 ? [encodeMessage({ method: "$/credit", params: { id, credits } })] : [];
        return { request: encodeMessage({ id, method, params }), notes };
      };
    }
    return notified === undefined ? () => ({ request, notes: [] }) : undefined;
  }

  /**
   * Sends `method` over a new link ahead of every call held, as its
   * greeting does: never queued, and never sent again.
   */
  async #ahead(method: string, params: unknown): Promise<unknown> {
    const id = this.#nextId;
    this.#nextId += 1;
    const request = encodeMessage({ id, method, params });
    const answer = new Promise<unknown>((resolve, reject) => {
      this.#waiting.set(id, {
        method,
        resolve,
        reject,
        notified: undefined,
        again: undefined,
        deadline: undefined,
      });
    });
    this.#dispatch(id, request);
    return answer;
  }

  /**
   * Keeps at most `max` calls in flight from now on, the cap the engine's
   * `hello` result advertises, queueing the rest (issue 2529).
   */
  capCallsInFlight(max: number): void {
    this.#cap = max;
    this.#drain();
  }

  /** Sends the call `id`, which holds a place in flight until it is answered. */
  #dispatch(id: MessageId, request: Uint8Array): void {
    this.#inFlight.add(id);
    this.#write(request);
  }

  /**
   * Holds the call `id` back, last, until a place frees and no call is
   * held, failing it with `deadline_exceeded` once its own timeout passes
   * first. Its bytes and its notes' count as waiting to be written.
   */
  #enqueue(
    id: MessageId,
    method: string,
    queued: Queued,
    deadline: HeldDeadline | undefined,
  ): void {
    this.#queuedWriteBytes += queued.notes.reduce(
      (bytes, note) => bytes + note.length,
      queued.request.length,
    );
    this.#queued.set(id, queued);
    if (deadline !== undefined) {
      this.#arm(queued, deadline.startedAt + deadline.limit.value, () => {
        this.#expire(id, method, deadline);
      });
    }
  }

  /**
   * Arms `queued`'s timer to `expire` it at `expiresAt`, in steps no longer
   * than a timer keeps.
   */
  #arm(queued: Queued, expiresAt: number, expire: () => void): void {
    const left = expiresAt - Date.now();
    queued.timer = setTimeout(
      () => {
        if (left > LONGEST_TIMER) {
          this.#arm(queued, expiresAt, expire);
        } else {
          expire();
        }
      },
      Math.min(left, LONGEST_TIMER),
    );
  }

  /** Takes the call `id` out of the queue: what was queued, if it was. */
  #unqueue(id: MessageId): Queued | undefined {
    const queued = this.#queued.get(id);
    if (queued === undefined) {
      return undefined;
    }
    this.#queued.delete(id);
    clearTimeout(queued.timer);
    this.#queuedWriteBytes -= queued.notes.reduce(
      (bytes, note) => bytes + note.length,
      queued.request.length,
    );
    return queued;
  }

  /**
   * Fails the queued call `id`, never sent on the link it waits for, once
   * its own `deadline` passed: what it waited for is a new link's greeting
   * while calls are held, and a place among the calls in flight otherwise.
   */
  #expire(id: MessageId, method: string, { limit, startedAt }: HeldDeadline): void {
    const waiting = this.#waiting.get(id);
    if (this.#unqueue(id) === undefined || waiting === undefined) {
      return;
    }
    this.#waiting.delete(id);
    const given = { key: limit.key, value: limit.value, layer: "run", origin: "argument" } as const;
    const exceeded = this.#held
      ? phaseExceeded(
          `the call waited ${limit.value} ms, its \`${limit.key}\`, while the Client reached the engine again after the link dropped, so it was not sent`,
          "Give the call a longer `timeout`, or make it again once the engine is reached.",
          given,
          startedAt,
          "hello",
        )
      : phaseExceeded(
          `the call waited ${limit.value} ms, its \`${limit.key}\`, for a place among the engine's calls in flight, so it was not sent`,
          "Make fewer calls at once on this Connection, or give the call a longer `timeout`.",
          given,
          startedAt,
          "calls_in_flight",
        );
    waiting.reject(this.#inCall(exceeded, method));
  }

  /** Sends queued calls, in the order they were made, while places are free and no call is held. */
  #drain(): void {
    while (!this.#held && this.#inFlight.size < this.#cap) {
      const next = this.#queued.entries().next();
      if (next.done === true) {
        return;
      }
      const [id, { request, notes }] = next.value;
      this.#unqueue(id);
      this.#dispatch(id, request);
      for (const note of notes) {
        this.#write(note);
      }
    }
  }

  /**
   * Sends the notification `method`, such as `$/credit`. One that names a
   * queued call waits to follow it, since the engine drops a notification
   * for a call it has not received; while calls are held, one naming no
   * held call is dropped, as its link is.
   */
  notify(method: string, params: unknown): void {
    if (this.#failure !== undefined) {
      return;
    }
    const id = namedId(params);
    const queued = id === undefined ? undefined : this.#queued.get(id);
    if (queued === undefined) {
      if (!this.#held) {
        this.#send({ method, params });
      }
      return;
    }
    const note = encodeMessage({ method, params });
    this.#queuedWriteBytes += note.length;
    queued.notes.push(note);
  }

  /**
   * Fails every waiting call with its copy of `error`, sending each sent
   * one its `$/cancel`; a queued one is never sent.
   */
  cancelAll(error: GoodfetchError): void {
    for (const [id, waiting] of this.#waiting) {
      if (this.#unqueue(id) === undefined) {
        this.#send({ method: CANCEL, params: { id } });
      }
      waiting.reject(this.#inCall(error, waiting.method));
    }
    this.#waiting.clear();
  }

  /** Ends the Client's side; the engine's ends when the engine stops. */
  async close(): Promise<void> {
    try {
      await this.#link.writer.close();
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
        `the Connection already holds ${this.#waiting.size} calls waiting for their answer, sent or queued, its bound, so this call was not sent`,
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
    const link = this.#link;
    // A transport that fails to drop its writes ends as one that rejected one.
    void link.writer.abort().catch((cause: unknown) => {
      link.writeFailure ??= { cause };
    });
    void this.#stopReading(link, undefined);
  }

  /**
   * Stops reading `link`'s bytes, once, for `reason`: answers the cause
   * when the transport failed to stop them, which `ended` reports.
   */
  #stopReading(link: Link, reason: unknown): Promise<{ readonly cause: unknown } | undefined> {
    link.readingStopped ??= link.reader.cancel(reason).then(
      () => undefined,
      (cause: unknown) => ({ cause }),
    );
    return link.readingStopped;
  }

  #send(message: Readonly<Record<string, unknown>>): void {
    this.#write(encodeMessage(message));
  }

  #write(bytes: Uint8Array): void {
    const link = this.#link;
    this.#queuedWriteBytes += bytes.length;
    link.writer
      .write(bytes)
      .finally(() => {
        this.#queuedWriteBytes -= bytes.length;
      })
      .catch((cause: unknown) => {
        // The engine's bytes to read may stay open for ever after a rejected
        // write, so the write ends the link itself: reading stops, and its
        // end reports the first rejection's cause.
        link.writeFailure ??= { cause };
        void this.#stopReading(link, cause);
      });
  }

  /** Each chunk of `reader`'s bytes, until they end or reading is cancelled. */
  async *#chunks(reader: ReadableStreamDefaultReader<Uint8Array>): AsyncGenerator<Uint8Array> {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        return;
      }
      yield value;
    }
  }

  /** Reads `link` until the engine's side of it ends: how it ended. */
  async #read(link: Link): Promise<TransportEnd> {
    let end: TransportEnd = { kind: "ended" };
    try {
      for await (const line of readLines(this.#chunks(link.reader))) {
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
    const unstopped = await this.#stopReading(link, undefined);
    if (end.kind === "ended" && link.writeFailure !== undefined) {
      return { kind: "write_failed", cause: link.writeFailure.cause };
    }
    if (end.kind === "ended" && unstopped !== undefined) {
      return { kind: "failed", cause: unstopped.cause };
    }
    return end;
  }

  /**
   * Reads each link in turn, `reading` the first: once one drops, reaches
   * the engine again when the Channel does, until none is reached or the
   * Connection closes, then fails every waiting call.
   */
  async #run(first: Promise<TransportEnd>): Promise<void> {
    let reading: Promise<TransportEnd> | undefined = first;
    while (reading !== undefined) {
      const end: TransportEnd = await reading;
      const relinking = this.#relinking;
      // A line that is no message is the engine's fault, which another link would not mend.
      if (relinking === undefined || end.kind === "invalid" || relinking.signal.aborted) {
        this.#end(await this.#owner.ended(end));
        return;
      }
      this.#dropped();
      reading = (await this.#relink(relinking, end))?.reading;
    }
  }

  /**
   * Reaches the engine again after the link ended with `end`, waiting the
   * backoff before each attempt: the new link's reading, its held calls
   * sent; `undefined` once the Channel ended instead.
   */
  async #relink(
    relinking: ChannelRelinking,
    end: TransportEnd,
  ): Promise<{ readonly reading: Promise<TransportEnd> } | undefined> {
    const droppedAt = Date.now();
    let cause = causeOf(end);
    for (let made = 0; ; made += 1) {
      const delay = reconnectDelay(made, Date.now() - droppedAt, relinking.within);
      if (delay === undefined) {
        this.#end(relinking.gaveUp(cause));
        return undefined;
      }
      if (!(await waited(delay, relinking.signal))) {
        this.#end(await this.#owner.ended(end));
        return undefined;
      }
      const left = relinking.within === null ? null : relinking.within - (Date.now() - droppedAt);
      const attempt = await this.#attempt(relinking, left);
      switch (attempt.kind) {
        case "greeted":
          this.#held = false;
          this.#drain();
          return { reading: attempt.reading };
        case "refused":
          this.#end(attempt.failure);
          return undefined;
        case "invalid":
          this.#end(await this.#owner.ended(attempt.end));
          return undefined;
        case "failed":
          cause = attempt.cause;
          break;
        default: {
          const unknown: never = attempt;
          return unknown;
        }
      }
    }
  }

  /**
   * Holds every call from now on, once the link dropped: sends again by its
   * id, first, each call in flight that may be, and fails every other one
   * with `connection_lost`; what serves the engine's requests stops.
   */
  #dropped(): void {
    this.#held = true;
    const earlier = new Map(this.#queued);
    this.#queued.clear();
    for (const [id, waiting] of this.#waiting) {
      if (earlier.has(id)) {
        continue;
      }
      const again = waiting.again?.();
      if (again === undefined) {
        this.#waiting.delete(id);
        const lost = waiting.notified === undefined ? lostCall() : lostStream();
        waiting.reject(this.#inCall(lost, waiting.method));
      } else {
        const queued = { request: again.request, timer: undefined, notes: again.notes };
        this.#enqueue(id, waiting.method, queued, waiting.deadline);
      }
    }
    // The calls queued before the drop follow, their timers still running.
    for (const [id, queued] of earlier) {
      this.#queued.set(id, queued);
    }
    // A call cancelled and not answered yet holds no place on the new link.
    this.#inFlight.clear();
    for (const serving of this.#serving.values()) {
      serving.abort(lostCall());
    }
    this.#serving.clear();
  }

  /**
   * Opens a new link through `relinking` and greets it, given up once
   * `left` milliseconds pass, when bounded, or the Connection closes. A
   * link that is not greeted is let go of, and the calls it carried fail.
   */
  async #attempt(relinking: ChannelRelinking, left: number | null): Promise<Attempt> {
    const attempt = new AbortController();
    const close = (): void => attempt.abort(relinking.signal.reason);
    relinking.signal.addEventListener("abort", close, { once: true });
    const timer =
      left === null
        ? undefined
        : setTimeout(
            () =>
              attempt.abort(new Error("timeout.connect passed while reaching the engine again")),
            Math.min(left, LONGEST_TIMER),
          );
    try {
      let transport: Transport;
      try {
        transport = await relinking.reopen(attempt.signal);
      } catch (cause) {
        return { kind: "failed", cause };
      }
      const link = linkOf(transport);
      this.#link = link;
      const reading = this.#read(link);
      const outcome = await Promise.race<Attempt>([
        relinking
          .greet(async (method, params) => this.#ahead(method, params))
          .then(
            () => ({ kind: "greeted", reading }),
            (error: unknown) => ({
              kind: "refused",
              failure: error instanceof GoodfetchError ? error : relinking.gaveUp(error),
            }),
          ),
        reading.then((end) =>
          end.kind === "invalid"
            ? { kind: "invalid", end }
            : { kind: "failed", cause: causeOf(end) },
        ),
        abortedWith(attempt.signal).then((cause) => ({ kind: "failed", cause })),
      ]);
      if (outcome.kind !== "greeted") {
        this.abandon();
        this.#dropped();
      }
      return outcome;
    } finally {
      clearTimeout(timer);
      relinking.signal.removeEventListener("abort", close);
    }
  }

  /** Fails every waiting call with its copy of `failure`, and every later one. */
  #end(failure: GoodfetchError): void {
    this.#failure = failure;
    for (const [id, waiting] of this.#waiting) {
      this.#unqueue(id);
      waiting.reject(this.#inCall(failure, waiting.method));
    }
    this.#waiting.clear();
    this.#inFlight.clear();
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

  /**
   * Stops the served request the engine cancelled, and answers it `call_cancelled` (ADR 0038).
   * The engine cancels through `$/cancel` as the requesting Client, so `by` is `client` (ADR 0113).
   */
  #cancelServed(id: MessageId): void {
    const serving = this.#serving.get(id);
    if (serving === undefined) {
      return;
    }
    this.#serving.delete(id);
    const cancelled = new CallCancelledError("the engine cancelled its request", {
      details: { by: "client" },
    });
    serving.abort(cancelled);
    this.#send({ id, error: replyOfError(cancelled) });
  }

  #receive(message: Message): void {
    switch (message.kind) {
      case "result":
      case "error": {
        // Only a sent call is answered: a queued one keeps its place.
        if (message.id === null || !this.#inFlight.delete(message.id)) {
          return;
        }
        const waiting = this.#waiting.get(message.id);
        this.#waiting.delete(message.id);
        if (message.kind === "result") {
          waiting?.resolve(message.result);
        } else {
          waiting?.reject(this.#owner.failure(message.error));
        }
        this.#drain();
        return;
      }
      case "request":
        this.#serve(message.id, message.method, message.params);
        return;
      case "notification": {
        const params = message.params;
        const id = namedId(params);
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
