// Request correlation and cancellation over one transport (ADR 0038): any
// number of calls in flight, each answered by its id; an aborted call's
// `$/cancel`; and the end of the transport, which fails every call still
// waiting with the reason the Connection gives.
import { encodeMessage, readLines, type Message, type MessageId } from "./framing.ts";
import type { GoodfetchError } from "./generated/index.ts";

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
  /** It sent a line that is no JSON-RPC message. */
  | { readonly kind: "invalid"; readonly line: string };

/** What a channel's owner decides about its end and its replies. */
export interface ChannelOwner {
  /** The error every waiting and later call fails with once the transport ended. */
  ended(end: TransportEnd): Promise<GoodfetchError>;
  /** The error a JSON-RPC `error` reply raises. */
  failure(error: unknown): GoodfetchError;
}

interface Waiting {
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
  readonly #owner: ChannelOwner;
  readonly #waiting = new Map<MessageId, Waiting>();
  #nextId = 0;
  #failure: GoodfetchError | undefined;
  /** Settles once the engine's side has ended and every waiting call failed. */
  readonly ended: Promise<void>;

  constructor(transport: Transport, owner: ChannelOwner) {
    this.#writer = transport.writable.getWriter();
    this.#owner = owner;
    this.ended = this.#read(transport.readable);
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
      return { id, result: Promise.reject(this.#failure) };
    }
    // Encoded before the call waits, so params that cannot be encoded, such
    // as a BigInt, fail this call alone and leave no waiter behind.
    let request: Uint8Array;
    try {
      request = encodeMessage({ id, method, params });
    } catch (error) {
      return { id, result: Promise.reject(error) };
    }
    const answer = new Promise<unknown>((resolve, reject) => {
      this.#waiting.set(id, { resolve, reject, notified: options.notified });
    });
    const abort = (): void => {
      const waiting = this.#waiting.get(id);
      if (waiting !== undefined && signal !== undefined) {
        this.#waiting.delete(id);
        this.#send({ method: "$/cancel", params: { id } });
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

  /** Fails every waiting call with `error`, sending each its `$/cancel`. */
  cancelAll(error: GoodfetchError): void {
    for (const [id, waiting] of this.#waiting) {
      this.#send({ method: "$/cancel", params: { id } });
      waiting.reject(error);
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

  #send(message: Readonly<Record<string, unknown>>): void {
    this.#write(encodeMessage(message));
  }

  #write(bytes: Uint8Array): void {
    // A write fails only once the engine's side has gone, which `ended`
    // reports to every waiting call; the rejection itself carries nothing more.
    this.#writer.write(bytes).catch(() => {});
  }

  async #read(readable: ReadableStream<Uint8Array>): Promise<void> {
    let end: TransportEnd = { kind: "ended" };
    try {
      for await (const line of readLines(readable)) {
        if (line.kind === "invalid") {
          end = { kind: "invalid", line: line.line };
          break;
        }
        this.#receive(line.message);
      }
    } catch (cause) {
      end = { kind: "failed", cause };
    }
    const failure = await this.#owner.ended(end);
    this.#failure = failure;
    for (const waiting of this.#waiting.values()) {
      waiting.reject(failure);
    }
    this.#waiting.clear();
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
        // No handler request reaches a Collie Connection yet: answer it as
        // JSON-RPC's unknown method, so the engine never waits on it.
        this.#send({
          id: message.id,
          error: { code: -32601, message: `this Client serves no \`${message.method}\`` },
        });
        return;
      case "notification": {
        const params = message.params;
        const id =
          typeof params === "object" && params !== null && "id" in params ? params.id : undefined;
        const waiting =
          typeof id === "number" || typeof id === "string" ? this.#waiting.get(id) : undefined;
        waiting?.notified?.(message.method, params);
        return;
      }
      default: {
        const unknown: never = message;
        return unknown;
      }
    }
  }
}
