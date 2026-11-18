// The Connection every Product's Connection type is built on (ADRs 0078
// and 0095): it opens with `hello`, naming the Product the Client needs and
// its version, and holds the engine's answer; it starts Runs and closes them
// before itself; and once closed, every call fails with `closed`. The only
// check of the engine's answers is the protocol version's (STYLE.md). A
// Product's package reaches its own operations, streams and Run type
// through it, and serves the engine's requests to its program's handlers.
import { Buffer } from "node:buffer";
import { AttachedEngine, type EngineExit } from "./attached-engine.ts";
import type { CallOptions, Caller, OperationTypes } from "./caller.ts";
import { errorOfReply, type ErrorClasses } from "./errors.ts";
import {
  CallCancelledError,
  ClosedError,
  CollieOperations,
  DeadlineExceededError,
  EngineExitedError,
  EngineOutputInvalidError,
  PRODUCT_VERSION,
  PROTOCOL_VERSION,
  ProtocolVersionUnsupportedError,
  type CollieOperationTypes,
  type GoodfetchError,
  type HelloParams,
  type HelloResult,
  type KeyValueStoreSetParams,
  type Product,
  type RunStartParams,
  type RunStarted,
} from "./generated/index.ts";
import type { JsonValue } from "./key-value-store.ts";
import { holdInRealm } from "./realm.ts";
import { Run, type RunHost } from "./run.ts";
import {
  Channel,
  type ChannelBounds,
  type ChannelOpenOptions,
  type Transport,
  type TransportEnd,
} from "./rpc.ts";

/** How long `close()` waits for an Attached engine to stop: `timeout.close`'s default (ADR 0080). */
const CLOSE_TIMEOUT = 30_000;
/** How long an ended transport waits for its engine's exit status before reporting without it. */
const EXIT_GRACE = 2_000;

/** The engine a Connection reached, fixed when it opened (G6). */
export interface EngineInfo {
  /** How it is reached; an Attached engine is the one kind so far. */
  readonly kind: "attached" | "daemon" | "remote";
  /** The engine's Product: `foxhound` serves a Collie Connection too (ADR 0095). A newer one is kept. */
  readonly product: Product | (string & {});
  /** The engine's Product version, reported, never compared (ADR 0090). */
  readonly version: string;
  /** The protocol version `major.minor` it speaks. */
  readonly protocolVersion: string;
  /** The executable started, or `null` when none was. */
  readonly enginePath: string | null;
}

/** What starting a Run takes (ADR 0102). */
export interface StartOptions {
  /** The Run's ID; the engine generates one when it is left out. An interrupted Run's ID resumes it; a finished one's returns it as it ended. */
  readonly runId?: string | undefined;
  /** Its tags: at most 16, each at most 64 characters (ADR 0036). */
  readonly tags?: readonly string[] | undefined;
}

/** What opening a Connection takes. */
export interface ClientConnectionOptions {
  /** The bytes to the engine. */
  readonly transport: Transport;
  /** The Attached engine whose stdio `transport` is, when the Connection started one. */
  readonly engine?: AttachedEngine | undefined;
  /** The Product the Client needs, which `hello` names. */
  readonly needs: Product;
  /** The error class of each code the Client's Product knows. */
  readonly errorClasses: ErrorClasses;
  /** What the Connection lets wait before it refuses a new call; the defaults when left out. */
  readonly bounds?: ChannelBounds | undefined;
  /** Cancels the opening: `open` rejects with its reason once the transport and its engine are ended. */
  readonly signal?: AbortSignal | undefined;
}

/** What serves one kind of the engine's requests: it answers the result, or throws the error to answer. */
export type Served = (params: unknown, signal: AbortSignal) => Promise<unknown>;

/** A call the engine streams to, as many notifications as its Client grants credits for. */
export interface Stream {
  /** Its answer, once the engine has sent what it streams. */
  readonly result: Promise<unknown>;
  /** Lets the engine send `credits` more notifications. */
  grant(credits: number): void;
}

/** The common Connection under each Product's Connection type. */
export class ClientConnection implements AsyncDisposable {
  readonly #channel: Channel;
  readonly #engine: AttachedEngine | undefined;
  readonly #operations: CollieOperations;
  readonly #runs = new Set<Run>();
  readonly #runHost: RunHost;
  readonly #served = new Map<string, Served>();
  #info: EngineInfo | undefined;
  #ended = false;
  #closing: Promise<void> | undefined;

  private constructor(options: ClientConnectionOptions) {
    this.#engine = options.engine;
    const { errorClasses } = options;
    this.#channel = new Channel(
      options.transport,
      {
        ended: (end) => this.#endedWith(end),
        failure: (error) => errorOfReply(error, errorClasses),
        serve: (method, params, signal) => this.#served.get(method)?.(params, signal),
      },
      options.bounds,
    );
    this.#operations = new CollieOperations(this.caller<CollieOperationTypes>());
    this.#runHost = {
      calls: this.caller<CollieOperationTypes>(),
      reachable: () => !this.#ended,
      forget: (run) => this.#runs.delete(run),
      keyValueStore: {
        get: async (runId, key) => {
          const { value } = await this.#operations.keyValueStoreGet({ run_id: runId, key });
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): a stored value is JSON.
          return value as JsonValue;
        },
        set: async (runId, key, value) => {
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the generated layer types a schemaless JSON value as an object; any JSON value but null is the engine's to take.
          const stored = value as KeyValueStoreSetParams["value"];
          await this.#operations.keyValueStoreSet({ run_id: runId, key, value: stored });
        },
        getBytes: async (runId, key) => {
          const { bytes } = await this.#operations.keyValueStoreGetBytes({ run_id: runId, key });
          return bytes
            ? {
                content: new Uint8Array(Buffer.from(bytes.content, "base64")),
                contentType: bytes.content_type,
              }
            : null;
        },
        setBytes: async (runId, key, content, contentType) => {
          await this.#operations.keyValueStoreSetBytes({
            run_id: runId,
            key,
            content: Buffer.from(content.buffer, content.byteOffset, content.byteLength).toString(
              "base64",
            ),
            content_type: contentType,
          });
        },
        delete: async (runId, key) =>
          (await this.#operations.keyValueStoreDelete({ run_id: runId, key })).existed,
      },
    };
    holdInRealm(this);
  }

  /**
   * Opens a Connection over `transport` and finishes `hello` before it
   * returns: a refused Product or protocol version fails here, as does an
   * aborted `signal`, with its reason, and the transport and its engine
   * are ended before it rejects.
   */
  static async open(options: ClientConnectionOptions): Promise<ClientConnection> {
    const connection = new ClientConnection(options);
    try {
      await connection.#hello(options.needs, options.signal);
    } catch (error) {
      await connection.#end();
      throw error;
    }
    return connection;
  }

  /** The engine reached. */
  engine(): EngineInfo {
    if (this.#info === undefined) {
      throw new TypeError("invariant: an open Connection has finished its hello");
    }
    return this.#info;
  }

  /** Starts a Run and returns once it is started (ADR 0102). */
  async start(options: StartOptions = {}): Promise<Run> {
    const params: RunStartParams = {
      ...(options.runId === undefined ? {} : { run_id: options.runId }),
      ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
    };
    return this.startRun(
      "run.start",
      () => this.#operations.runStart(params),
      (host, started) => new Run(host, started),
    );
  }

  /**
   * Starts a Run of a Product's own type with `start`, such as a crawl's,
   * and holds the Run `make` builds on the engine's answer, so the
   * Connection's close closes it.
   */
  async startRun<Started extends Run>(
    operation: string,
    start: () => Promise<RunStarted>,
    make: (host: RunHost, started: RunStarted) => Started,
  ): Promise<Started> {
    this.#openOrThrow(operation);
    const run = make(this.#runHost, await start());
    this.#runs.add(run);
    return run;
  }

  /** The Connection's calls, typed by a Product's operation table. */
  caller<Types extends Record<keyof Types, OperationTypes>>(): Caller<Types> {
    const channel = this.#channel;
    return {
      call<Name extends keyof Types & string>(
        operation: Name,
        params: Types[Name]["params"],
        options: CallOptions,
      ): Promise<Types[Name]["result"]> {
        // Engine answers are not validated beyond `hello`'s protocol version (STYLE.md): the result is the operation's as the catalog types it.
        return channel.call(operation, params, options);
      },
    };
  }

  /**
   * Sends `method`, which the engine answers once it has streamed what the
   * Client granted credits for; `notified` hears each notification.
   */
  stream(method: string, params: unknown, options: ChannelOpenOptions): Stream {
    this.#openOrThrow(method);
    const channel = this.#channel;
    const { id, result } = channel.open(method, params, options);
    return { result, grant: (credits) => channel.notify("$/credit", { id, credits }) };
  }

  /** Serves the engine's requests of `method` with `served`; a method none serves is answered `-32601`. */
  serve(method: string, served: Served): void {
    this.#served.set(method, served);
  }

  /**
   * Closes the Connection; idempotent. It closes each Run it opened first,
   * stopping one still running, cancels the calls in flight, then ends its
   * Attached engine and waits for it to stop.
   */
  close(): Promise<void> {
    this.#closing ??= this.#close();
    return this.#closing;
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  #openOrThrow(operation: string): void {
    if (this.#closing !== undefined) {
      throw new ClosedError(`${operation}: this Connection is closed; connect again`, {
        fix: "Connect again: a closed Connection never reopens.",
      });
    }
  }

  async #hello(needs: Product, signal: AbortSignal | undefined): Promise<void> {
    const params: HelloParams = {
      protocol_version: PROTOCOL_VERSION,
      surface: "sdk_typescript",
      needs,
      product_version: PRODUCT_VERSION,
    };
    const answer = await this.#channel.call("hello", params, { signal });
    const spoken = protocolVersionOf(answer);
    if (spoken === undefined || !speaks(spoken)) {
      throw new ProtocolVersionUnsupportedError(
        spoken === undefined
          ? `this Client speaks protocol ${PROTOCOL_VERSION}, and the engine's answer names no protocol version`
          : `this Client speaks protocol ${PROTOCOL_VERSION}, and the engine answered with ${spoken}`,
        {
          fix:
            spoken === undefined
              ? `Use the engine and the SDK of one Product version: this SDK ${PRODUCT_VERSION} speaks protocol ${PROTOCOL_VERSION}.`
              : `Use the engine and the SDK of one Product version: this SDK ${PRODUCT_VERSION} speaks protocol ${PROTOCOL_VERSION}, and the engine ${productVersionOf(answer)} speaks ${spoken}.`,
          details: {
            client_protocol_version: PROTOCOL_VERSION,
            engine_protocol_version: spoken ?? null,
          },
        },
      );
    }
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the one answer checked is the protocol version it speaks, above (STYLE.md).
    const result = answer as HelloResult;
    this.#info = {
      kind: "attached",
      product: result.product,
      version: result.product_version,
      protocolVersion: result.protocol_version,
      enginePath: this.#engine?.path ?? null,
    };
  }

  /**
   * Closes the Runs, cancels the calls in flight and ends the engine, all
   * within `timeout.close`. Past it, whatever is still waiting is cancelled,
   * an Attached engine is killed, and the close fails with
   * `deadline_exceeded`.
   */
  async #close(): Promise<void> {
    const startedAt = Date.now();
    const closed = await within(this.#closeInOrder(), CLOSE_TIMEOUT);
    if (closed === "late") {
      this.#channel.cancelAll(cancelledByClose());
      this.#engine?.kill();
      await this.#channel.close();
      await this.#engine?.exited;
      throw new DeadlineExceededError(
        `the Connection did not close within timeout.close (${CLOSE_TIMEOUT} ms), so its engine was stopped`,
        {
          code: "deadline_exceeded",
          group: "deadline_exceeded",
          retryable: true,
          fix: "Raise timeout.close, or find why the engine stopped answering in the engine's stderr, which the logger got.",
          details: {
            limit: { key: "timeout.close", value: CLOSE_TIMEOUT, layer: "default" },
            elapsed: Date.now() - startedAt,
            waited_for: "close",
          },
        },
      );
    }
    if (closed.failure !== undefined) {
      throw closed.failure;
    }
  }

  /** Closes each Run, then cancels the calls in flight and ends the engine; answers the first Run's failure. */
  async #closeInOrder(): Promise<{ readonly failure: unknown }> {
    let failure: unknown;
    for (const run of this.#runs) {
      try {
        await run.close();
      } catch (error) {
        failure ??= error;
      }
    }
    this.#channel.cancelAll(cancelledByClose());
    await this.#channel.close();
    // An Attached engine stops at the end of its input.
    await this.#engine?.exited;
    await this.#channel.ended;
    return { failure };
  }

  /**
   * Ends a Connection that never opened: both sides at once, since nothing
   * it still holds to write or to read matters, then waits for the engine,
   * which stops at the end of its input, to be gone.
   */
  async #end(): Promise<void> {
    this.#channel.abandon();
    if (this.#engine !== undefined) {
      await this.#engine.stop(CLOSE_TIMEOUT);
    }
    await this.#channel.ended;
  }

  async #endedWith(end: TransportEnd): Promise<GoodfetchError> {
    this.#ended = true;
    if (end.kind === "invalid") {
      this.#engine?.kill();
      return new EngineOutputInvalidError(
        "the engine wrote a line that is not one JSON-RPC message, so the Connection ended",
        { details: { line: end.line } },
      );
    }
    if (this.#closing !== undefined) {
      return new ClosedError("this Connection is closed; connect again", {
        fix: "Connect again: a closed Connection never reopens.",
      });
    }
    const exit = await this.#exitOfEngine();
    return new EngineExitedError(
      exit.exitCode === null && exit.signal === null
        ? "the engine ended before its Connection was closed"
        : `the engine ended before its Connection was closed, with ${exit.signal ?? `exit code ${exit.exitCode}`}`,
      {
        details: { exit_code: exit.exitCode, signal: exit.signal, stderr_tail: exit.stderrTail },
        cause: end.kind === "ended" ? undefined : end.cause,
      },
    );
  }

  async #exitOfEngine(): Promise<EngineExit> {
    const unknown: EngineExit = { exitCode: null, signal: null, stderrTail: "" };
    if (this.#engine === undefined) {
      return unknown;
    }
    const exit = await within(this.#engine.exited, EXIT_GRACE);
    return exit === "late" ? unknown : exit;
  }
}

/** What `promise` settles to, or `"late"` when it has not within `timeout` milliseconds. */
async function within<T>(promise: Promise<T>, timeout: number): Promise<T | "late"> {
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<"late">((resolve) => {
    timer = setTimeout(() => resolve("late"), timeout);
  });
  try {
    return await Promise.race([promise, late]);
  } finally {
    clearTimeout(timer);
  }
}

/** The error a call in flight fails with when its Connection closes: cancelled by its Client. */
function cancelledByClose(): CallCancelledError {
  return new CallCancelledError("the call's Connection was closed", {
    details: { by: "client" },
  });
}

/** The protocol version a `hello` answer names, when it names one. */
function protocolVersionOf(answer: unknown): string | undefined {
  const version: unknown =
    typeof answer === "object" && answer !== null && "protocol_version" in answer
      ? answer.protocol_version
      : undefined;
  return typeof version === "string" ? version : undefined;
}

/** The Product version a `hello` answer names, for a message. */
function productVersionOf(answer: unknown): string {
  const version: unknown =
    typeof answer === "object" && answer !== null && "product_version" in answer
      ? answer.product_version
      : undefined;
  return typeof version === "string" ? version : "of an unknown version";
}

/**
 * Whether an engine of `version` takes this Client: the same line, at the
 * same or a newer minor (ADR 0030). During `0.y` previews each `0.y` minor
 * is a line of its own, as `goodfetch-protocol`'s rule reads a `hello`.
 */
function speaks(version: string): boolean {
  const [major, minor] = version.split(".").map(Number);
  const [ownMajor, ownMinor] = PROTOCOL_VERSION.split(".").map(Number);
  if (major === undefined || minor === undefined || ownMinor === undefined) {
    return false;
  }
  if (major === 0) {
    return ownMajor === 0 && minor === ownMinor;
  }
  return major === ownMajor && minor >= ownMinor;
}
