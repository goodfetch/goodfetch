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
import type { ErrorLimit } from "./goodfetch-error.ts";
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
  type LogLevel,
  type Product,
  type RunLimit,
  type RunStartParams,
  type RunStarted,
  type UsageSetParams,
} from "./generated/index.ts";
import type { JSONValue } from "./json.ts";
import { limitList, type Limits } from "./limits.ts";
import { logEngineEvent, logLevelOf, type Logger } from "./logger.ts";
import { collieTimes } from "./generated/times.ts";
import { rawCall, type RawCallOptions, type RawParams } from "./raw-call.ts";
import { holdInRealm } from "./realm.ts";
import { Run, type RunHost } from "./run.ts";
import { settingsOnWire, type RunSettings } from "./generated/settings.ts";
import {
  Channel,
  type ChannelBounds,
  type ChannelOpenOptions,
  type Transport,
  type TransportEnd,
} from "./rpc.ts";
import { withTimes, type Times } from "./times.ts";

/** How long opening waits for `hello`'s answer when `connect()`'s `timeout` is left out: `timeout.connect`'s default (ADR 0083). */
const CONNECT_TIMEOUT = 30_000;
/** How long `close()` waits for an Attached engine to stop: `timeout.close`'s default (ADR 0080). */
const CLOSE_TIMEOUT = 30_000;
/** How long an ended transport waits for its engine's exit status before reporting without it. */
const EXIT_GRACE = 2_000;
/** The notification that carries one of the engine's log events, `goodfetch-protocol`'s `log::LOG`. */
const LOG = "$/log";

/** The engine a Connection reached, fixed when it opened (G6). It never holds a Daemon's token. */
export interface EngineInfo {
  /** How it is reached: an Attached engine the Connection started, the Daemon of this machine, or a remote Daemon. */
  readonly kind: "attached" | "daemon" | "remote";
  /** The engine's Product: `foxhound` serves a Collie Connection too (ADR 0095). A newer one is kept. */
  readonly product: Product | (string & {});
  /** The engine's Product version, reported, never compared (ADR 0090). */
  readonly version: string;
  /** The protocol version `major.minor` it speaks. */
  readonly protocolVersion: string;
  /** The project file the engine read, or `null` when it read none. */
  readonly projectFile: string | null;
  /** The Goodfetch home the engine runs in, or `null` for a remote Daemon, or when none is known. */
  readonly home: string | null;
  /** The executable started, or `null` when none was. */
  readonly enginePath: string | null;
  /** Where a Daemon was reached: a remote one's URL, or this machine's socket path or loopback address; `null` for an Attached engine. */
  readonly url: string | null;
  /** Why `auto` ran another engine than the Daemon it found, and what to do; `null` when it ran the one it chose. */
  readonly fallback: { readonly cause: string; readonly fix: string } | null;
}

/** How a Connection's engine was reached, which `engine()` reports. */
export interface Reached {
  readonly kind: EngineInfo["kind"];
  readonly home: string | null;
  readonly url: string | null;
}

/** What a Daemon's `hello` takes beyond an Attached engine's. */
export interface DaemonHello {
  /** The Daemon's token, which `hello` carries and nothing else holds. */
  readonly token: string;
  /** What fails the opening when the Daemon closes the Connection before it answers `hello`. */
  readonly closedAtHello: () => Error;
}

/** What starting a Run takes (ADR 0102). */
export interface StartOptions {
  /**
   * Names the Run for good: a start with an existing Run ID resumes the Run
   * if it was interrupted, or returns it as it ended (`startedAs()` says
   * which), and the engine generates one when it is left out. A Run another
   * engine writes fails with `run_in_use`, one begun with other inputs with
   * `run_conflict`, and an ID off its pattern with `run_id_invalid`.
   */
  readonly runId?: string | undefined;
  /** Its tags: at most 16, each at most 64 characters (ADR 0036). */
  readonly tags?: readonly string[] | undefined;
  /**
   * The Run's Limits (ADR 0080), which only lower: starting an existing
   * Run again with the values it began with changes nothing, a lower one
   * lowers its Limit, and a higher one fails with `limit_raise_refused`.
   * An amount the SDK cannot turn exactly fails with `setting_invalid`
   * before anything is sent.
   */
  readonly limits?: Limits | undefined;
  /**
   * The Run's own settings (ADR 0083), which its Sessions and pages
   * inherit, nested by camelCase segment, such as
   * `{ timeout: { navigation: 30_000 } }`.
   */
  readonly settings?: RunSettings | undefined;
}

/** What opening a Connection takes. */
export interface ClientConnectionOptions {
  /** The bytes to the engine. */
  readonly transport: Transport;
  /** The Attached engine whose stdio `transport` is, when the Connection started one. */
  readonly engine?: AttachedEngine | undefined;
  /** How the engine was reached; an Attached engine with no home known when left out. */
  readonly reached?: Reached | undefined;
  /** The Daemon's token and what its closing at `hello` fails with, when the engine is a Daemon. */
  readonly daemon?: DaemonHello | undefined;
  /** The Product the Client needs, which `hello` names. */
  readonly needs: Product;
  /** The error class of each code the Client's Product knows. */
  readonly errorClasses: ErrorClasses;
  /** What the Connection lets wait before it refuses a new call; the defaults when left out. */
  readonly bounds?: ChannelBounds | undefined;
  /** Cancels the opening: `open` rejects with its reason once the transport and its engine are ended. */
  readonly signal?: AbortSignal | undefined;
  /** The Client connection's Limits, in the wire's form, set once `hello` is answered. */
  readonly limits?: readonly RunLimit[] | undefined;
  /**
   * How long `hello` waits for its answer, already checked by `timeoutOf`:
   * `timeout.connect`'s 30 s when left out, none for `null`.
   */
  readonly timeout?: number | null | undefined;
  /**
   * The host's logger, which gets a start that resumed a Run at info and
   * one that found it ended at warning, and the engine's log events at the
   * level it records (`logLevelOf`), read at `open` and again when a Run
   * starts.
   */
  readonly logger?: Logger | undefined;
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
  readonly #logger: Logger | undefined;
  readonly #operations: CollieOperations;
  readonly #runs = new Set<Run>();
  readonly #runHost: RunHost;
  readonly #served = new Map<string, Served>();
  /** The level of the engine's logs the engine was last told, by `hello` or `connection.set_log_level`. */
  #logLevel: LogLevel;
  readonly #reached: Reached;
  #info: EngineInfo | undefined;
  #ended = false;
  #closing: Promise<void> | undefined;

  private constructor(options: ClientConnectionOptions) {
    this.#engine = options.engine;
    this.#reached = options.reached ?? { kind: "attached", home: null, url: null };
    this.#logger = options.logger;
    this.#logLevel = logLevelOf(options.logger);
    const { errorClasses } = options;
    this.#channel = new Channel(
      options.transport,
      {
        ended: (end) => this.#endedWith(end),
        failure: (error) => errorOfReply(error, errorClasses),
        serve: (method, params, signal) => this.#served.get(method)?.(params, signal),
        notified: (method, params) => {
          if (method === LOG) {
            logEngineEvent(this.#logger, params);
          }
        },
      },
      options.bounds,
    );
    const calls = this.caller<CollieOperationTypes>(collieTimes);
    this.#operations = new CollieOperations(calls);
    this.#runHost = {
      calls,
      raw: (operation, params, callOptions) => this.#raw(operation, params, callOptions),
      reachable: () => !this.#ended,
      stream: (method, params, opened) => this.stream(method, params, opened),
      forget: (run) => this.#runs.delete(run),
      logger: this.#logger,
      keyValueStore: {
        get: async (runId, key) => {
          const { value } = await this.#operations.keyValueStoreGet({ run_id: runId, key });
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): a stored value is JSON.
          return value as JSONValue;
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
   * Opens a Connection over `transport`, finishes `hello` and sets its
   * Limits before it returns: a refused Product, protocol version or Limit
   * fails here, as does an aborted `signal`, with its reason, and a `hello`
   * unanswered within `timeout`, with `deadline_exceeded`; the transport
   * and its engine are ended before it rejects.
   */
  static async open(options: ClientConnectionOptions): Promise<ClientConnection> {
    const connection = new ClientConnection(options);
    try {
      await connection.#hello(options.needs, options.daemon, options.signal, options.timeout);
      await connection.#setLimits(options.limits ?? [], options.signal);
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

  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint;
   * `finish()` ends it. Starts a Run and returns once it is started
   * (ADR 0102).
   */
  async start(options: StartOptions = {}): Promise<Run> {
    const limits = limitList(options.limits);
    const settings = settingsOnWire(options.settings);
    const params: RunStartParams = {
      ...(options.runId === undefined ? {} : { run_id: options.runId }),
      ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
      ...(limits.length === 0 ? {} : { limits }),
      ...(settings === undefined ? {} : { settings }),
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
    // The start goes out at once when the level did not change, so a close
    // right after it still finds it in flight and cancels it.
    const told = this.#followLogLevel();
    if (told !== undefined) {
      await told;
      this.#openOrThrow(operation);
    }
    const started = await start();
    this.#logStart(started);
    const run = make(this.#runHost, started);
    this.#runs.add(run);
    return run;
  }

  /**
   * Tells the host's logger how a start with an existing Run ID reached its
   * Run: resumed at info, ended at warning, since the Run then takes no new
   * work (ADR 0102). A new Run is not logged.
   */
  #logStart({ run_id: runId, started_as: startedAs }: RunStarted): void {
    const fields = { runId, startedAs };
    switch (startedAs) {
      case "new":
        return;
      case "resumed":
        this.#logger?.info(
          `The start resumed the Run \`${runId}\` from its committed state.`,
          fields,
        );
        return;
      case "ended":
        this.#logger?.warn(
          `The start found the Run \`${runId}\` ended: it is returned as it ended and takes no new work.`,
          fields,
        );
        return;
    }
  }

  /**
   * Tells the engine the level the host's logger records now, with
   * `connection.set_log_level`, when it changed since the engine was last
   * told, and answers once the engine answered; nothing is sent and nothing
   * is answered when it did not change, and nothing polls it.
   */
  #followLogLevel(): Promise<void> | undefined {
    const level = logLevelOf(this.#logger);
    if (level === this.#logLevel) {
      return undefined;
    }
    return this.#tellLogLevel(level);
  }

  /**
   * Tells the engine `level`. The engine's logs are telemetry, so a refusal
   * fails no start: it goes to the host's logger at warning, and the level
   * the engine kept stays the one told, so the next Run's start tells it
   * again. A Connection that ended or is closing fails with its own error.
   */
  async #tellLogLevel(level: LogLevel): Promise<void> {
    const before = this.#logLevel;
    this.#logLevel = level;
    try {
      await this.#operations.connectionSetLogLevel({ log_level: level });
    } catch (error) {
      // A start begun meanwhile may have told the engine a newer level.
      if (this.#logLevel === level) {
        this.#logLevel = before;
      }
      if (this.#ended || this.#closing !== undefined) {
        throw error;
      }
      this.#logger?.warn(
        `The engine refused the log level \`${level}\`; it keeps sending its logs at \`${before}\`.`,
        { logLevel: level, error },
      );
    }
  }

  /**
   * The Connection's calls, typed by a Product's operation table, whose
   * results read the points in time `times` places as `Date`s.
   */
  caller<Types extends Record<keyof Types, OperationTypes>>(
    times: Times<keyof Types & string>,
  ): Caller<Types> {
    const channel = this.#channel;
    return {
      async call<Name extends keyof Types & string>(
        operation: Name,
        params: Types[Name]["params"],
        options: CallOptions,
      ): Promise<Types[Name]["result"]> {
        const result = await channel.call(operation, params, options);
        const path = times.results[operation];
        // Engine answers are not validated beyond `hello`'s protocol version (STYLE.md): the result is the operation's as the catalog types it.
        return path === undefined ? result : withTimes(result, path, times.types);
      },
    };
  }

  /**
   * The raw catalog call on the Connection (ADR 0099): sends `operation`,
   * one that acts on no value or whose params name every value, with
   * `params` as the wire's JSON, and answers its result as the engine sent
   * it, both in snake_case. Fails with `closed` once the Connection is.
   */
  call(
    operation: string,
    params: RawParams = {},
    options: RawCallOptions = {},
  ): Promise<JSONValue> {
    return rawCall(
      (name, sent, callOptions) => this.#raw(name, sent, callOptions),
      operation,
      params,
      options,
    );
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

  /** Sends `operation` as the main path does, its params and result unconverted. */
  #raw(operation: string, params: unknown, options: CallOptions): Promise<unknown> {
    this.#openOrThrow(operation);
    return this.#channel.call(operation, params, options);
  }

  #openOrThrow(operation: string): void {
    if (this.#closing !== undefined) {
      throw new ClosedError(`${operation}: this Connection is closed; connect again`, {
        fix: "Connect again: a closed Connection never reopens.",
      });
    }
  }

  /**
   * Sets each of `limits` on this Client connection's own scope, one
   * `usage.set` each, in order (ADR 0080's note of issue 2830).
   */
  async #setLimits(limits: readonly RunLimit[], signal: AbortSignal | undefined): Promise<void> {
    for (const limit of limits) {
      const params: UsageSetParams = { scope: "connection", ...limit };
      await this.#channel.call("usage.set", params, { signal });
    }
  }

  async #hello(
    needs: Product,
    daemon: DaemonHello | undefined,
    signal: AbortSignal | undefined,
    timeout: number | null | undefined,
  ): Promise<void> {
    const params: HelloParams = {
      protocol_version: PROTOCOL_VERSION,
      surface: "sdk_typescript",
      needs,
      product_version: PRODUCT_VERSION,
      log_level: this.#logLevel,
      ...(daemon === undefined ? {} : { token: daemon.token }),
    };
    let answer: unknown;
    try {
      answer = await this.#answerWithin(this.#channel.call("hello", params, { signal }), timeout);
    } catch (error) {
      throw daemon !== undefined && error instanceof EngineExitedError
        ? daemon.closedAtHello()
        : error;
    }
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
      kind: this.#reached.kind,
      product: result.product,
      version: result.product_version,
      protocolVersion: result.protocol_version,
      projectFile: null,
      home: this.#reached.home,
      enginePath: this.#engine?.path ?? null,
      url: this.#reached.url,
      fallback: null,
    };
  }

  /**
   * `hello`'s answer, once `called` gives it within `timeout`, its
   * `timeout.connect` (ADR 0083).
   *
   * @throws {DeadlineExceededError} past it, its `limit` naming
   * `timeout.connect` at the run layer, given as an argument, or at its
   * default, and `waited_for` `hello`.
   */
  async #answerWithin(
    called: Promise<unknown>,
    timeout: number | null | undefined,
  ): Promise<unknown> {
    if (timeout === null) {
      return called;
    }
    const value = timeout ?? CONNECT_TIMEOUT;
    const limit: ErrorLimit =
      timeout === undefined
        ? { key: "timeout.connect", value, layer: "default" }
        : { key: "timeout.connect", value, layer: "run", origin: "argument" };
    const startedAt = Date.now();
    const answered = await within(
      called.then((answer) => ({ answer })),
      value,
    );
    if (answered !== "late") {
      return answered.answer;
    }
    throw phaseExceeded(
      `timeout.connect (${value} ms) passed before the engine answered hello`,
      "Raise connect()'s timeout, or find why the engine does not answer in the engine's stderr, which the logger got.",
      limit,
      startedAt,
      "hello",
    );
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
      throw phaseExceeded(
        `the Connection did not close within timeout.close (${CLOSE_TIMEOUT} ms), so its engine was stopped`,
        "Raise timeout.close, or find why the engine stopped answering in the engine's stderr, which the logger got.",
        { key: "timeout.close", value: CLOSE_TIMEOUT, layer: "default" },
        startedAt,
        "close",
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

/**
 * The `deadline_exceeded` of a phase `limit` bounds, whose wait for
 * `waitedFor` started at `startedAt` (ADR 0080).
 */
function phaseExceeded(
  message: string,
  fix: string,
  limit: ErrorLimit,
  startedAt: number,
  waitedFor: "hello" | "close",
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
