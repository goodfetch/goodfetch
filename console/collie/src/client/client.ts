// The Console's Client (ADR 0068, items 2 and 10; spec #592): the protocol
// code the TypeScript SDK runs, through the Collie client package's browser
// entry, over a transport, a body fetcher and page visibility it takes as
// values. Production passes the WebSocket on the Console's own origin,
// `GET` by handle and `document.visibilityState`; tests pass the in-memory
// adapter of the protocol.
//
// Its `hello` names the surface `console`, the form `catalog` (ADR 0099),
// the Product of its build and its Product version (ADR 0095). It reaches
// every operation through the raw catalog call with wire JSON, typed by the
// operation map `xtask codegen` writes. A subscription is a sequence of the
// stream's events, each marked lossless or telemetry, and a `resync`
// carries the snapshot that replaces a screen's state (ADR 0031).
//
// When its Connection drops, it sends again only each subscription, after
// the last position it read, and each read or idempotent write waiting for
// its answer; an Effect waiting for its answer fails with
// `connection_lost`, an `UnavailableError`, and so does one called with no
// Connection, while a read or idempotent write waits for the next (ADR
// 0079). The Client is `reconnecting` while it opens a Connection,
// `read_only` while it waits to try again or for the page to be visible,
// and `connected` once `hello` is answered. It closes its Connection while
// the page is hidden, so a hidden tab never keeps a Daemon from stopping
// idle (ADR 0031), and opens one once the page is visible again.
import {
  Channel,
  PRODUCT_VERSION,
  errorOfReply,
  subscribeEvents,
  type EventsSubscription,
  type LastRead,
  type StreamedEvent,
  type Transport as ByteTransport,
  type TransportEnd,
} from "@goodfetch/collie-client/browser";
import {
  ConnectionLostError,
  PROTOCOL_VERSION,
  collieErrorClasses,
  type CollieEventPayloads,
  type GoodfetchError,
  type HelloParams,
  type HelloResult,
  type Product,
  type Resync,
} from "@goodfetch/collie-client/generated";
import {
  eventClasses,
  operationClasses,
  type ConsoleOperations,
  type EventClass,
} from "#generated/operations.ts";
import type { FoxhoundOperation } from "#generated/foxhound.ts";

export type { ConsoleOperations, EventClass, LastRead };

/** What hears an open Connection. */
export interface ConnectionListener {
  /** A message the engine sent: the text of one JSON-RPC message. */
  message(text: string): void;
  /** The Connection closed. */
  close(): void;
}

/** An open Connection to the engine. */
export interface Connection {
  /** Sends the text of one JSON-RPC message. */
  send(text: string): void;
  /** Closes the Connection; its listener then hears `close`. */
  close(): void;
}

/** What the Client opens its Connections with: the WebSocket, or the in-memory adapter. */
export interface Transport {
  open(listener: ConnectionListener): Connection;
}

/** What reads a page body, a screenshot or an Export by its handle, `<run_id>/<body>`. */
export type BodyFetcher = (handle: string) => Promise<Uint8Array>;

/** Whether the page is visible, as `document.visibilityState` says, and when that changes. */
export interface PageVisibility {
  readonly visible: boolean;
  /** Calls `changed` at each change, until the function it answers is called. */
  watch(changed: () => void): () => void;
}

/** The Client's link to its engine. */
export type ConnectionState = "connected" | "reconnecting" | "read_only";

/** The engine a Connection reached, as its `hello` answered. */
export interface EngineIdentity {
  readonly product: Product;
  readonly productVersion: string;
  readonly engineInstance: string;
}

/** The Client's configuration: its endpoint and authentication are the transport's (ADR 0031). */
export interface ConsoleClientOptions {
  readonly transport: Transport;
  readonly bodyFetcher: BodyFetcher;
  readonly visibility: PageVisibility;
  /** The Product of the Console's build. */
  readonly product: Product;
  /** How long to wait before each new attempt once one failed, the last repeating. */
  readonly retryDelays?: readonly number[] | undefined;
  /** How long a Connection may send nothing before it is asked something, and then before it counts as stalled. */
  readonly stallTimeout?: number | undefined;
}

/** One of the stream's events, as a subscription gives it. */
export type StreamItem =
  | {
      readonly kind: "event";
      readonly class: EventClass;
      readonly position: number;
      readonly event: Exclude<keyof CollieEventPayloads, "events.resync">;
      readonly data: unknown;
      /** How many telemetry events of its scope it stands for, beside itself. */
      readonly dropped: number;
    }
  | {
      /** The engine no longer holds the position the Client read after: this snapshot replaces the screen's state. */
      readonly kind: "resync";
      readonly position: number;
      readonly snapshot: Resync;
    };

/** A subscription: the stream's events, in position order, across reconnects. */
export interface Subscription extends AsyncIterable<StreamItem> {
  /** The last event read, which a resubscription resumes after. */
  readonly lastRead: LastRead | undefined;
  /** Ends the subscription. */
  close(): void;
}

/** The streams a Client subscribes to. */
export type StreamName = "events";

/** What a call takes besides its params. */
export interface CallOptions {
  /** Cancels the call once aborted. */
  readonly signal?: AbortSignal | undefined;
}

/** The Console's Client. */
export interface ConsoleClient {
  /** Where it stands. */
  readonly state: ConnectionState;
  /** Calls `changed` at each change of state, until the function it answers is called. */
  watch(changed: (state: ConnectionState) => void): () => void;
  /** The engine of the current Connection, once its `hello` is answered. */
  readonly engine: EngineIdentity | undefined;
  /**
   * Calls `operation` of the catalog with its wire params and answers its
   * wire result; aborting `options.signal` cancels it, on the engine too,
   * and rejects it with the signal's reason.
   */
  call<Name extends keyof ConsoleOperations>(
    operation: Name,
    params: ConsoleOperations[Name]["params"],
    options?: CallOptions,
  ): Promise<ConsoleOperations[Name]["result"]>;
  /**
   * Calls `operation`, which only Foxhound carries, as a read, with its
   * wire params, and answers its wire result: Collie's engine refuses it
   * with `not_in_product`, whose fix names Foxhound (ADR 0098).
   */
  callFoxhound(
    operation: FoxhoundOperation,
    params: Readonly<Record<string, unknown>>,
    options?: CallOptions,
  ): Promise<unknown>;
  /** Subscribes to `stream`, after `after` when given, else from now. */
  subscribe(stream: StreamName, options?: { readonly after?: LastRead | undefined }): Subscription;
  /** The bytes of the body `handle` names. */
  body(handle: string): Promise<Uint8Array>;
  /** Closes its Connection for good. */
  close(): void;
}

// The messages of `connection_lost` the Client raises itself: English, as
// an engine's are, which the code's catalog message replaces on a screen
// (ADR 0079).
/* oxlint-disable lingui/no-unlocalized-strings -- an error's English message, never shown as it is (ADR 0079). */
const DROPPED = "the Connection to the engine dropped before the call was answered";
const INVALID =
  "the engine sent what is no JSON-RPC message, so the Connection closed before the call was answered";
const NOT_CONNECTED =
  "the Console has no Connection to the engine now; it connects again by itself";
/* oxlint-enable lingui/no-unlocalized-strings */

/** How many events a subscription lets the engine send ahead of what was read. */
const WINDOW = 64;

/** How long a silent Connection waits to be asked something, then to answer it, in milliseconds. */
const STALL_TIMEOUT = 10_000;

/** The waits between attempts once one failed, in milliseconds, the last repeating. */
const RETRY_DELAYS = [1_000, 2_000, 5_000, 10_000];

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * The byte stream a Channel runs over, made of a Connection's text
 * messages, one line each; `heard` is called at each message, and
 * `abandon` ends the stream at once and closes the Connection, without
 * waiting for a stalled one to say it closed.
 */
function byteTransport(
  transport: Transport,
  heard: () => void,
): { bytes: ByteTransport; connection: Connection; abandon: () => void } {
  let push: ((text: string) => void) | undefined;
  let end: (() => void) | undefined;
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      push = (text) => controller.enqueue(encoder.encode(`${text}\n`));
      end = () => controller.close();
    },
  });
  const closed = (): void => {
    const ending = end;
    end = undefined;
    push = undefined;
    ending?.();
  };
  const connection = transport.open({
    message: (text) => {
      heard();
      push?.(text);
    },
    close: closed,
  });
  let pending = "";
  const writable = new WritableStream<Uint8Array>({
    write(bytes) {
      pending += decoder.decode(bytes, { stream: true });
      let newline = pending.indexOf("\n");
      while (newline !== -1) {
        connection.send(pending.slice(0, newline));
        pending = pending.slice(newline + 1);
        newline = pending.indexOf("\n");
      }
    },
    close() {
      connection.close();
    },
  });
  const abandon = (): void => {
    closed();
    connection.close();
  };
  return { bytes: { readable, writable }, connection, abandon };
}

/** A subscription's events waiting to be read, and its readers waiting for them. */
class Queue {
  readonly #items: StreamItem[] = [];
  readonly #waiting: ((item: IteratorResult<StreamItem>) => void)[] = [];
  #closed = false;

  push(item: StreamItem): void {
    const reader = this.#waiting.shift();
    if (reader === undefined) {
      this.#items.push(item);
    } else {
      reader({ done: false, value: item });
    }
  }

  close(): void {
    this.#closed = true;
    for (const reader of this.#waiting.splice(0)) {
      reader({ done: true, value: undefined });
    }
  }

  async next(): Promise<IteratorResult<StreamItem>> {
    const item = this.#items.shift();
    if (item !== undefined) {
      return { done: false, value: item };
    }
    if (this.#closed) {
      return { done: true, value: undefined };
    }
    return new Promise((resolve) => this.#waiting.push(resolve));
  }
}

function isResync(data: unknown): data is Resync {
  return typeof data === "object" && data !== null && "runs" in data && Array.isArray(data.runs);
}

/** Whether `name` is one of Collie's events but the resync, which the Client reads apart. */
function isEventName(name: string): name is Exclude<keyof CollieEventPayloads, "events.resync"> {
  return name !== "events.resync" && Object.hasOwn(eventClasses, name);
}

/** The item a streamed event becomes; none for an event this build does not know. */
function itemOf(streamed: StreamedEvent): StreamItem | undefined {
  if (streamed.event === "events.resync") {
    return isResync(streamed.data)
      ? { kind: "resync", position: streamed.position, snapshot: streamed.data }
      : undefined;
  }
  const { event } = streamed;
  if (!isEventName(event)) {
    return undefined;
  }
  return {
    kind: "event",
    class: eventClasses[event],
    position: streamed.position,
    event,
    data: streamed.data,
    dropped: streamed.dropped ?? 0,
  };
}

/** One subscription, kept across Connections. */
class KeptSubscription implements Subscription {
  readonly #queue = new Queue();
  readonly #client: Client;
  #lastRead: LastRead | undefined;
  #current: { subscription: EventsSubscription; abort: AbortController } | undefined;
  #closed = false;

  constructor(client: Client, after: LastRead | undefined) {
    this.#client = client;
    this.#lastRead = after;
  }

  get lastRead(): LastRead | undefined {
    return this.#lastRead;
  }

  /** Subscribes on `channel`, the Connection to `engineInstance`, after the last event read. */
  start(channel: Channel, engineInstance: string): void {
    if (this.#closed) {
      return;
    }
    const abort = new AbortController();
    const subscription = subscribeEvents(channel, {
      after: this.#lastRead,
      signal: abort.signal,
      received: (events) => {
        for (const streamed of events) {
          this.#lastRead = { engine_instance: engineInstance, position: streamed.position };
          const item = itemOf(streamed);
          if (item === undefined) {
            subscription.grant(1);
          } else {
            this.#queue.push(item);
          }
        }
      },
    });
    this.#current = { subscription, abort };
    subscription.grant(WINDOW);
  }

  /** The Connection it ran on dropped. */
  dropped(): void {
    this.#current = undefined;
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#current?.abort.abort();
    this.#current = undefined;
    this.#queue.close();
    this.#client.forget(this);
  }

  [Symbol.asyncIterator](): AsyncIterator<StreamItem> {
    return {
      next: async () => {
        const next = await this.#queue.next();
        // Each event read lets the engine send one more (spec #216).
        if (next.done !== true) {
          this.#current?.subscription.grant(1);
        }
        return next;
      },
      return: async () => {
        this.close();
        return { done: true, value: undefined };
      },
    };
  }
}

/** One Connection, opened and saying `hello`. */
interface Opened {
  readonly channel: Channel;
  readonly connection: Connection;
  /** What looks for a stall, until the Connection ends. */
  readonly watchdog: ReturnType<typeof setInterval>;
}

class Client implements ConsoleClient {
  readonly #options: ConsoleClientOptions;
  readonly #listeners = new Set<(state: ConnectionState) => void>();
  readonly #subscriptions = new Set<KeptSubscription>();
  /** The calls waiting for a Connection. */
  readonly #waiting = new Set<() => void>();
  readonly #stopWatching: () => void;
  #state: ConnectionState = "reconnecting";
  #engine: EngineIdentity | undefined;
  #opened: Opened | undefined;
  #failures = 0;
  #retry: ReturnType<typeof setTimeout> | undefined;
  #closed = false;

  constructor(options: ConsoleClientOptions) {
    this.#options = options;
    // A hidden page holds no Connection, so a forgotten tab counts as no
    // Client and the Daemon may stop idle; a visible one holds one, which
    // keeps the Daemon running while someone looks (ADR 0031).
    this.#stopWatching = options.visibility.watch(() => {
      if (!options.visibility.visible) {
        clearTimeout(this.#retry);
        this.#retry = undefined;
        this.#opened?.connection.close();
        return;
      }
      if (this.#state === "read_only" && this.#retry === undefined) {
        this.#connect();
      }
    });
    this.#connect();
  }

  get state(): ConnectionState {
    return this.#state;
  }

  get engine(): EngineIdentity | undefined {
    return this.#engine;
  }

  watch(changed: (state: ConnectionState) => void): () => void {
    this.#listeners.add(changed);
    return () => this.#listeners.delete(changed);
  }

  async call<Name extends keyof ConsoleOperations>(
    operation: Name,
    params: ConsoleOperations[Name]["params"],
    options: CallOptions = {},
  ): Promise<ConsoleOperations[Name]["result"]> {
    // A read or an idempotent write waits for a Connection and is sent
    // again on the next one when its own drops; an Effect never is (ADR
    // 0079).
    const result = await this.#send(
      operation,
      params,
      operationClasses[operation] !== "effect",
      options.signal,
    );
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): the result is the operation's as the catalog types it.
    return result as ConsoleOperations[Name]["result"];
  }

  async callFoxhound(
    operation: FoxhoundOperation,
    params: Readonly<Record<string, unknown>>,
    options: CallOptions = {},
  ): Promise<unknown> {
    return this.#send(operation, params, true, options.signal);
  }

  /** Sends `operation`, again on the next Connection when `sentAgain` and its own drops. */
  async #send(
    operation: string,
    params: unknown,
    sentAgain: boolean,
    signal: AbortSignal | undefined,
  ): Promise<unknown> {
    for (;;) {
      signal?.throwIfAborted();
      const opened = sentAgain
        ? await this.#connected(operation, signal)
        : this.#current(operation);
      try {
        return await opened.channel.call(operation, params, { signal });
      } catch (error) {
        const dropped = error instanceof ConnectionLostError && this.#opened !== opened;
        if (!sentAgain || !dropped || this.#closed) {
          throw error;
        }
      }
    }
  }

  /** The Connection a call goes on now; `connection_lost` when there is none. */
  #current(operation: string): Opened {
    const opened = this.#opened;
    if (opened === undefined || this.#state !== "connected") {
      throw lost(operation);
    }
    return opened;
  }

  /**
   * The Connection a call goes on, once there is one; `connection_lost`
   * once the Client is closed, and the signal's reason once it is aborted.
   */
  async #connected(operation: string, signal: AbortSignal | undefined): Promise<Opened> {
    while (!this.#closed) {
      signal?.throwIfAborted();
      const opened = this.#opened;
      if (opened !== undefined && this.#state === "connected") {
        return opened;
      }
      await new Promise<void>((resolve) => {
        const woken = (): void => {
          signal?.removeEventListener("abort", woken);
          this.#waiting.delete(woken);
          resolve();
        };
        this.#waiting.add(woken);
        signal?.addEventListener("abort", woken, { once: true });
      });
    }
    throw lost(operation);
  }

  /** Wakes every call waiting for a Connection, to look again. */
  #wake(): void {
    const waiting = [...this.#waiting];
    this.#waiting.clear();
    for (const resolve of waiting) {
      resolve();
    }
  }

  subscribe(
    _stream: StreamName,
    options: { readonly after?: LastRead | undefined } = {},
  ): Subscription {
    const subscription = new KeptSubscription(this, options.after);
    this.#subscriptions.add(subscription);
    const opened = this.#opened;
    if (opened !== undefined && this.#state === "connected" && this.#engine !== undefined) {
      subscription.start(opened.channel, this.#engine.engineInstance);
    }
    return subscription;
  }

  async body(handle: string): Promise<Uint8Array> {
    return this.#options.bodyFetcher(handle);
  }

  close(): void {
    this.#closed = true;
    this.#stopWatching();
    if (this.#retry !== undefined) {
      clearTimeout(this.#retry);
      this.#retry = undefined;
    }
    for (const subscription of this.#subscriptions) {
      subscription.close();
    }
    const opened = this.#opened;
    this.#opened = undefined;
    if (opened !== undefined) {
      clearInterval(opened.watchdog);
      void opened.channel.close();
    }
    this.#wake();
  }

  /** Forgets a closed subscription. */
  forget(subscription: KeptSubscription): void {
    this.#subscriptions.delete(subscription);
  }

  #enter(state: ConnectionState): void {
    if (this.#state !== state) {
      this.#state = state;
      for (const listener of this.#listeners) {
        listener(state);
      }
      this.#wake();
    }
  }

  /** Opens a Connection and says `hello`; once answered, subscribes again. */
  #connect(): void {
    if (this.#closed) {
      return;
    }
    this.#retry = undefined;
    this.#enter("reconnecting");
    // A Connection that sends nothing for a stall timeout is asked
    // something, `hello` first; one still silent a stall timeout later is
    // stalled, and abandoned as a dropped one is.
    let heard = false;
    let asked = true;
    const { bytes, connection, abandon } = byteTransport(this.#options.transport, () => {
      heard = true;
    });
    let current: Opened | undefined;
    const channel = new Channel(bytes, {
      ended: async (end) => this.#ended(current, end),
      failure: (error: unknown) => errorOfReply(error, collieErrorClasses),
    });
    const watchdog = setInterval(() => {
      if (heard) {
        heard = false;
        asked = false;
      } else if (asked) {
        abandon();
      } else {
        asked = true;
        channel.call("engine.info", {}).catch(() => undefined);
      }
    }, this.#options.stallTimeout ?? STALL_TIMEOUT);
    current = { channel, connection, watchdog };
    this.#opened = current;
    const hello: HelloParams = {
      protocol_version: PROTOCOL_VERSION,
      surface: "console",
      form: "catalog",
      needs: this.#options.product,
      product_version: PRODUCT_VERSION,
    };
    void this.#hello(current, hello);
  }

  /** Says `hello` on `opened`; once answered, the Client is connected and subscribes again. */
  async #hello(opened: Opened, hello: HelloParams): Promise<void> {
    let answered: HelloResult;
    try {
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the one answer checked is the protocol version it speaks (STYLE.md), which the engine refuses itself when it differs.
      answered = (await opened.channel.call("hello", hello)) as HelloResult;
    } catch {
      // The Connection's end, which `ended` hears, says what comes next.
      opened.connection.close();
      return;
    }
    if (this.#opened !== opened || this.#closed) {
      return;
    }
    this.#engine = {
      product: answered.product,
      productVersion: answered.product_version,
      engineInstance: answered.engine_instance,
    };
    this.#failures = 0;
    this.#enter("connected");
    for (const subscription of this.#subscriptions) {
      subscription.start(opened.channel, answered.engine_instance);
    }
  }

  /** The Connection `opened` ended: every waiting call fails, and the Client tries again. */
  async #ended(opened: Opened | undefined, end: TransportEnd): Promise<GoodfetchError> {
    const failure = new ConnectionLostError(end.kind === "invalid" ? INVALID : DROPPED);
    if (opened !== undefined) {
      clearInterval(opened.watchdog);
    }
    if (this.#opened !== opened || this.#closed) {
      return failure;
    }
    const wasConnected = this.#state === "connected";
    this.#opened = undefined;
    for (const subscription of this.#subscriptions) {
      subscription.dropped();
    }
    if (!wasConnected) {
      this.#failures += 1;
    }
    this.#retryLater(wasConnected);
    return failure;
  }

  /** Tries again: at once after a Connection that worked, else after a wait; only while the page is visible. */
  #retryLater(atOnce: boolean): void {
    if (this.#closed) {
      return;
    }
    if (atOnce && this.#options.visibility.visible) {
      this.#connect();
      return;
    }
    this.#enter("read_only");
    if (!this.#options.visibility.visible) {
      return;
    }
    const delays = this.#options.retryDelays ?? RETRY_DELAYS;
    const delay = delays[Math.min(this.#failures - 1, delays.length - 1)] ?? 0;
    this.#retry = setTimeout(() => this.#connect(), delay);
  }
}

/** `connection_lost` for a call made while no Connection is open. */
function lost(operation: string): GoodfetchError {
  return new ConnectionLostError(`${operation}: ${NOT_CONNECTED}`);
}

/** The Console's Client, which opens its first Connection at once. */
export function createConsoleClient(options: ConsoleClientOptions): ConsoleClient {
  return new Client(options);
}
