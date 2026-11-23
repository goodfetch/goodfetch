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
  runtimeMessage,
  spelledOperation,
  type ErrorLimit,
  type OperationNames,
} from "./goodfetch-error.ts";
import {
  CallCancelledError,
  ClosedError,
  CollieOperations,
  collieOperations,
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
  type CatalogRunStartParams,
  type CatalogTarget,
  type Product,
  type ProjectSettings,
  type RecipeStartParams,
  type Recording,
  type RunLimit,
  type RunStartParams,
  type RunStarted,
  type SettingsExplanation,
  type UsageSetParams,
} from "./generated/index.ts";
import type { JSONValue } from "./json.ts";
import type { ProjectFile } from "./project-file.ts";
import { limitList, type Limits } from "./limits.ts";
import { logEngineEvent, logLevelOf, type Logger } from "./logger.ts";
import { collieWire, type CollieWireType } from "./generated/wire.ts";
import { rawCall, type RawCallOptions, type RawParams } from "./raw-call.ts";
import { warnEntry } from "./markers.ts";
import { holdInRealm } from "./realm.ts";
import { timeoutOf } from "./handle.ts";
import { Run, type RunHost } from "./run.ts";
import { RunView } from "./run-view.ts";
import { settingsOnWire, type RunSettings } from "./generated/settings.ts";
import { MAX_LISTENER_QUEUE } from "./listeners.ts";
import type { SettingsOnWire } from "./settings.ts";
import { SharedSubscription } from "./shared-subscription.ts";
import {
  Channel,
  phaseExceeded,
  type ChannelBounds,
  type ChannelOpenOptions,
  type OperationClasses,
  type Transport,
  type TransportEnd,
} from "./rpc.ts";
import { resumable } from "./resumed-stream.ts";
import {
  carry,
  traceContextSource,
  type TraceContext,
  type TraceContextSource,
} from "./trace-context.ts";
import { batchOf, type LastRead } from "./stream.ts";
import { fromWire, toWire, type Wire } from "./wire.ts";

/** How long opening waits for `hello`'s answer when `connect()`'s `timeout` is left out: `timeout.connect`'s default (ADR 0083). */
const CONNECT_TIMEOUT = 30_000;
/** How long `close()` waits for an Attached engine to stop: `timeout.close`'s default (ADR 0080). */
const CLOSE_TIMEOUT = 30_000;
/** How long an ended transport waits for its engine's exit status before reporting without it. */
const EXIT_GRACE = 2_000;
/** The notification that carries one of the engine's log events, `goodfetch-protocol`'s `log::LOG`. */
const LOG = "$/log";
/** What failed for a call on a closed Connection (ADR 0079's layout). */
const CLOSED = "this Connection is closed";
/** What failed for a call a Daemon's dropped Connection ended (ADR 0079's layout). */
const DROPPED = "the engine ended the Connection before it answered";
/** The fix of a call on a closed or dropped Connection. */
const CONNECT_AGAIN = "Connect again: a closed Connection never reopens.";

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
  /** The project file `connect()` read and carries to the engine (ADR 0083), by its absolute path, or `null` when it read none. */
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
  /** Why `auto` ran this engine in place of the Daemon it found (ADR 0098); none when left out. */
  readonly fallback?: EngineInfo["fallback"] | undefined;
}

/** What a Daemon's `hello` takes beyond an Attached engine's. */
export interface DaemonHello {
  /** The Daemon's token, which `hello` carries and nothing else holds. */
  readonly token: string;
  /** What fails the opening when the Daemon closes the Connection before it answers `hello`. */
  readonly closedAtHello: () => Error;
}

/**
 * How a Connection to a remote Daemon reaches it again once its link drops
 * (spec 606): within `timeout.connect` of the drop, it greets the Daemon
 * with the same `hello`, resubscribes after the last event it read, and
 * sends again each call in flight whose operation is a read or an
 * idempotent write; an Effect in flight fails with `connection_lost`.
 */
export interface Reconnect {
  /** Each operation's class by its catalog name (ADR 0011). */
  readonly operationClasses: OperationClasses;
  /** Opens a new link to the Daemon; `signal` aborts once the attempt is given up. */
  readonly reopen: (signal: AbortSignal) => Promise<Transport>;
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

/**
 * What starting a handler-less Run takes, a Recipe's or a code-free catalog
 * run's (ADRs 0036 and 0102): a start's options, and how it records, how
 * long it may last and whether it outlives its program.
 */
export interface HandlerlessStartOptions extends StartOptions {
  /** The Response recording it replays or records, a WARC file on the program's machine. */
  readonly recording?: Recording | undefined;
  /**
   * How long the Run may last, in milliseconds, at least 1, as
   * `timeout.run` for this Run: left out for the setting's value, `null`
   * for none. Any other number fails with `setting_invalid` before
   * anything is sent.
   */
  readonly timeout?: number | null | undefined;
  /**
   * Whether the Run outlives its program: closing its value then frees
   * only the value, `finish()` waits for its end and `stop()` stops it.
   */
  readonly detached?: boolean | undefined;
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
  /** How a remote Daemon is reached again once the link drops; a dropped link ends the Connection when left out. */
  readonly reconnect?: Reconnect | undefined;
  /** The Product the Client needs, which `hello` names. */
  readonly needs: Product;
  /** The error class of each code the Client's Product knows. */
  readonly errorClasses: ErrorClasses;
  /**
   * Each of the Client's Product's operations' names in TypeScript form,
   * which heads the runtime's own errors of a call (ADR 0079).
   */
  readonly operationNames: OperationNames;
  /** What the Connection lets wait before it refuses a new call; the defaults when left out. */
  readonly bounds?: ChannelBounds | undefined;
  /** How many events each listener of published events holds in its own queue, already checked: `MAX_LISTENER_QUEUE` when left out. */
  readonly listenerQueue?: number | undefined;
  /** Cancels the opening: `open` rejects with its reason once the transport and its engine are ended. */
  readonly signal?: AbortSignal | undefined;
  /** The Client connection's Limits, in the wire's form, set once `hello` is answered. */
  readonly limits?: readonly RunLimit[] | undefined;
  /** The Connection's own settings (ADR 0083), in the wire's form, which `hello` carries. */
  readonly settings?: SettingsOnWire | undefined;
  /**
   * How long `hello` waits for its answer, already checked by `timeoutOf`:
   * `timeout.connect`'s 30 s when left out, none for `null`.
   */
  readonly timeout?: number | null | undefined;
  /**
   * When `timeout` started counting, for a `hello` that follows a wait the
   * same `timeout.connect` bounds, as for the pick a Daemon hands over to
   * (ADR 0098); as `hello` is sent when left out.
   */
  readonly connectStartedAt?: number | undefined;
  /**
   * The project file `connect()` read (ADR 0083), which each Run's start
   * and `config.explain` carry, and whose `timeout.connect` bounds `hello`
   * when `timeout` is left out; none when left out.
   */
  readonly project?: ProjectFile | undefined;
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
  readonly #project: ProjectFile | undefined;
  readonly #operationNames: OperationNames;
  #info: EngineInfo | undefined;
  /** The engine instance the last `hello` answered, which the events read since were published by. */
  #engineInstance: string | undefined;
  /** Aborted once the Connection closes its link, which then ends it rather than reaching the engine again. */
  readonly #closingLink = new AbortController();
  #ended = false;
  #closing: Promise<void> | undefined;
  /** Reads the Trace context each call the program makes carries (ADR 0132), set before `hello`. */
  #traceContext: TraceContextSource = () => undefined;

  private constructor(options: ClientConnectionOptions) {
    this.#engine = options.engine;
    this.#reached = options.reached ?? { kind: "attached", home: null, url: null };
    this.#project = options.project;
    this.#logger = options.logger;
    this.#logLevel = logLevelOf(options.logger);
    const { errorClasses, operationNames } = options;
    this.#operationNames = operationNames;
    this.#channel = new Channel(
      options.transport,
      {
        ended: (end) => this.#endedWith(end),
        failure: (error) => errorOfReply(error, errorClasses),
        operationNames,
        serve: (method, params, signal) => this.#served.get(method)?.(params, signal),
        notified: (method, params) => {
          if (method === LOG) {
            logEngineEvent(this.#logger, params);
          }
        },
      },
      options.bounds,
    );
    const calls = this.caller<CollieOperationTypes>(collieWire);
    this.#operations = new CollieOperations(calls);
    this.#runHost = {
      calls,
      raw: (operation, params, callOptions) => this.#raw(operation, params, callOptions),
      reachable: () => !this.#ended,
      stream: (method, params, opened) => this.stream(method, params, opened),
      published: new SharedSubscription(
        (opened) => this.#subscribe(opened),
        this.#logger,
        options.listenerQueue ?? MAX_LISTENER_QUEUE,
      ),
      forget: (run) => this.#runs.delete(run),
      operationNames,
      logger: this.#logger,
      keyValueStore: {
        get: async (runId, key) => {
          const { value } = await this.#operations.keyValueStoreGet({ runId, key });
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): a stored value is JSON.
          return value as JSONValue;
        },
        set: async (runId, key, value) => {
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the generated layer types a schemaless JSON value as an object; any JSON value but null is the engine's to take.
          const stored = value as KeyValueStoreSetParams["value"];
          await this.#operations.keyValueStoreSet({ runId, key, value: stored });
        },
        getBytes: async (runId, key) => {
          const { bytes } = await this.#operations.keyValueStoreGetBytes({ runId, key });
          return bytes
            ? {
                content: new Uint8Array(Buffer.from(bytes.content, "base64")),
                contentType: bytes.contentType,
              }
            : null;
        },
        setBytes: async (runId, key, content, contentType) => {
          await this.#operations.keyValueStoreSetBytes({
            runId,
            key,
            content: Buffer.from(content.buffer, content.byteOffset, content.byteLength).toString(
              "base64",
            ),
            contentType,
          });
        },
        delete: async (runId, key) =>
          (await this.#operations.keyValueStoreDelete({ runId, key })).existed,
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
      connection.#traceContext = await traceContextSource();
      await connection.#hello(options);
      await connection.#setLimits(options.limits ?? [], options.signal);
    } catch (error) {
      await connection.#end();
      throw error;
    }
    // Only an engine that answered `hello` is reached again: one that closes
    // the link before it answers refuses the Client, as for a wrong token.
    if (options.reconnect !== undefined) {
      connection.#reconnectWith(options.reconnect, options);
    }
    return connection;
  }

  /**
   * Reaches the engine again through `reconnect` whenever the link drops,
   * within `timeout.connect` of the drop, greeting it with the same `hello`
   * and setting the Client connection's Limits again (spec 606).
   */
  #reconnectWith(reconnect: Reconnect, options: ClientConnectionOptions): void {
    const { operationClasses, reopen } = reconnect;
    const limits = options.limits ?? [];
    this.#channel.relinkWith({
      resends: (method) => {
        const operationClass = operationClasses[method];
        return operationClass === "read" || operationClass === "idempotent_write";
      },
      within: connectLimit(options.timeout, options.project)?.value ?? null,
      signal: this.#closingLink.signal,
      reopen,
      greet: async (call) => {
        const answer = await call("hello", this.#helloParams(options));
        this.#greeted(answer);
        for (const limit of limits) {
          await call("usage.set", usageSetParams(limit));
        }
        this.#logger?.info(
          `The Connection reached the engine at ${this.#reached.url ?? "its address"} again after its link dropped.`,
          { url: this.#reached.url },
        );
      },
      gaveUp: (cause) =>
        new ClosedError(
          runtimeMessage(
            "the Connection's link to the engine dropped, and reaching the engine again failed within timeout.connect",
            { fix: CONNECT_AGAIN },
          ),
          { fix: CONNECT_AGAIN, cause },
        ),
    });
  }

  /**
   * `events.subscribe`, which a reconnect sends again after the last event
   * it read, as the engine instance that published it, with the credits it
   * had left, so its readers lose no event (spec 606).
   */
  #subscribe(opened: ChannelOpenOptions): Stream {
    // It belongs to no one caller, so it carries no Trace context (ADR 0132).
    return resumable<LastRead>((options) => this.#open("events.subscribe", {}, options), opened, {
      read: (method, params) => {
        if (method !== "$/events") {
          return undefined;
        }
        const events = batchOf(params);
        const newest = events.at(-1);
        const instance = this.#engineInstance;
        return {
          count: events.length,
          last:
            newest === undefined || instance === undefined
              ? undefined
              : { engine_instance: instance, position: newest.position },
        };
      },
      params: (last) => (last === undefined ? {} : { after: last }),
    });
  }

  /** The engine reached. */
  engine(): EngineInfo {
    if (this.#info === undefined) {
      throw new TypeError("invariant: an open Connection has finished its hello");
    }
    return this.#info;
  }

  /**
   * The project file `connect()` read (ADR 0083), as each Run's start and
   * `config.explain` carry it; none when it read none.
   */
  project(): ProjectSettings | undefined {
    return this.#project?.carried;
  }

  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint.
   * `finish()` ends it.
   *
   * In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK installs
   * no signal handler, and a signal the program does not handle ends it as a
   * crash would (ADRs 0078, 0093).
   *
   * Starts a Run its program drives, and answers once it is started. A Run
   * ID names the Run for good, and a start under one acts on what the Run
   * index holds (ADR 0102): no such Run begins it, `new`; an interrupted
   * Run, stopped, failed, or running with no live writer, resumes from its
   * committed state at the next run epoch, `resumed`, as `run.resume` does;
   * a finished Run is answered as it ended with no handler called, `ended`.
   * A Run live with a writer, in this engine or another, fails with
   * `run_in_use`; a Run started with other inputs or by another caller with
   * `run_conflict` listing each difference; a Run the Run index lists whose
   * state is missing or unreadable with `run_state_unavailable`, never
   * starting afresh.
   *
   * @experimental
   */
  async start(options: StartOptions = {}): Promise<Run> {
    warnEntry(collieOperations, "run.start");
    const params = startParams(options, this.project());
    return this.startRun(
      "run.start",
      () => this.#operations.runStart(params),
      (host, started) => new Run(host, started),
    );
  }

  /**
   * Starts a Recipe's run, a Run with no handler in the program, and answers
   * once it is started. Its Run ID works as `run.start`'s. With `detached`,
   * the Run outlives the program that started it: closing its value frees
   * only the value, `run.finish` waits for its end and `run.stop` stops it.
   *
   * @experimental
   */
  async startRecipe(recipe: string, options: HandlerlessStartOptions = {}): Promise<Run> {
    warnEntry(collieOperations, "recipe.start");
    const params: RecipeStartParams = {
      recipe,
      ...handlerlessParams(options, this.project()),
    };
    return this.startRun(
      "recipe.start",
      () => this.#operations.recipeStart(params),
      (host, started) => new Run(host, started, options.detached === true),
    );
  }

  /**
   * Starts a code-free catalog run, a Run with no handler in the program
   * whose verbs come from the catalog (ADR 0036): an Extraction of a page by
   * a schema, `extract`, an Agent task, `agent`, or a Plan replayed with no
   * model call, `plan_replay`; and answers once it is started. Its Run ID
   * works as `run.start`'s. With `detached`, the Run outlives the program
   * that started it: closing its value frees only the value, `run.finish`
   * waits for its end and `run.stop` stops it.
   *
   * @experimental
   */
  async startCatalogRun(
    target: CatalogTarget,
    options: HandlerlessStartOptions = {},
  ): Promise<Run> {
    warnEntry(collieOperations, "catalog_run.start");
    const params: CatalogRunStartParams = {
      target,
      ...handlerlessParams(options, this.project()),
    };
    return this.startRun(
      "catalog_run.start",
      () => this.#operations.catalogRunStart(params),
      (host, started) => new Run(host, started, options.detached === true),
    );
  }

  /**
   * Opens a view of the Run `runId` names, one another program started such
   * as a detached Run, with no round trip (ADR 0102). The view takes no
   * writer role, and closing it frees only the view. Fails with `closed`
   * once the Connection is closed.
   */
  async openRun(runId: string): Promise<RunView> {
    this.#openOrThrow("openRun");
    return new RunView(runId);
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
  #logStart({ runId, startedAs }: RunStarted): void {
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
      await this.#operations.connectionSetLogLevel({ logLevel: level });
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
   * The Connection's calls, typed by a Product's operation table, which
   * send each declared field in the engine's spelling and read each in
   * camelCase, each point in time as a `Date`, by `wire` (ADRs 0092 and
   * 0099).
   */
  caller<Types extends Record<keyof Types, OperationTypes>>(
    wire: Wire<keyof Types & string>,
  ): Caller<Types> {
    const channel = this.#channel;
    const traceContext = (): TraceContext | undefined => this.#traceContext();
    return {
      async call<Name extends keyof Types & string>(
        operation: Name,
        params: Types[Name]["params"],
        options: CallOptions,
      ): Promise<Types[Name]["result"]> {
        const sent = carry(toWire(params, wire.params[operation], wire.types), traceContext());
        const result = await channel.call(operation, sent, options);
        // Engine answers are not validated beyond `hello`'s protocol version (STYLE.md): the result is the operation's as the catalog types it.
        return fromWire(result, wire.results[operation], wire.types);
      },
    };
  }

  /**
   * Shows each setting's effective value, the layer that set it and the
   * layer that locked it, from the built-in defaults, the Operator policy,
   * user configuration, the project configuration given and the run layer:
   * the Connection's own settings with the run values given over them, or a
   * live Run's as it opened when `run_id` names one; and lists the keys only
   * the other Product honours as that Product's.
   *
   * What the Connection runs with (ADR 0083): each setting under `keys`,
   * each a key or a prefix of keys such as `timeout`, every one when left
   * out, with the origin that set it too, as `goodfetch config explain`
   * shows them, the project file `connect()` read as the project
   * configuration. Fails with `closed` once the Connection is closed.
   */
  explainSettings(keys?: readonly string[]): Promise<SettingsExplanation> {
    this.#openOrThrow("config.explain");
    const project = this.project();
    return this.#operations.configExplain({
      ...(keys === undefined ? {} : { keys: [...keys] }),
      ...(project === undefined ? {} : { project }),
    });
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
    return this.#open(method, carry(params, this.#traceContext()), options);
  }

  /** Sends the streamed call `method` with `params` as they are. */
  #open(method: string, params: unknown, options: ChannelOpenOptions): Stream {
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
    return this.#channel.call(operation, carry(params, this.#traceContext()), options);
  }

  #openOrThrow(operation: string): void {
    if (this.#closing !== undefined) {
      throw new ClosedError(
        runtimeMessage(CLOSED, {
          operation: spelledOperation(this.#operationNames, operation),
          fix: CONNECT_AGAIN,
        }),
        { operation, fix: CONNECT_AGAIN },
      );
    }
  }

  /**
   * Sets each of `limits` on this Client connection's own scope, one
   * `usage.set` each, in order (ADR 0080's note of issue 2830).
   */
  async #setLimits(limits: readonly RunLimit[], signal: AbortSignal | undefined): Promise<void> {
    for (const limit of limits) {
      await this.#channel.call("usage.set", usageSetParams(limit), { signal });
    }
  }

  async #hello(options: ClientConnectionOptions): Promise<void> {
    const { daemon, signal, connectStartedAt } = options;
    let answer: unknown;
    try {
      answer = await this.#answerWithin(
        this.#channel.call("hello", this.#helloParams(options), { signal }),
        connectLimit(options.timeout, options.project),
        connectStartedAt ?? Date.now(),
      );
    } catch (error) {
      throw daemon !== undefined &&
        (error instanceof EngineExitedError || error instanceof ClosedError)
        ? daemon.closedAtHello()
        : error;
    }
    this.#greeted(answer);
  }

  /** `hello`'s params in the wire's form, naming the log level the engine was last told. */
  #helloParams({ needs, daemon, settings }: ClientConnectionOptions): unknown {
    const params: HelloParams = {
      protocolVersion: PROTOCOL_VERSION,
      surface: "sdk_typescript",
      form: "typescript",
      needs,
      productVersion: PRODUCT_VERSION,
      logLevel: this.#logLevel,
      ...(daemon === undefined ? {} : { token: daemon.token }),
      ...(settings === undefined ? {} : { settings }),
    };
    return toWire(params, { type: "HelloParams" satisfies CollieWireType }, collieWire.types);
  }

  /**
   * Takes `hello`'s answer: the engine it reached, and its cap of calls.
   *
   * @throws {ProtocolVersionUnsupportedError} when the engine speaks no
   * protocol version this Client takes.
   */
  #greeted(answer: unknown): void {
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
    const result = fromWire(
      answer,
      { type: "HelloResult" satisfies CollieWireType },
      collieWire.types,
    ) as HelloResult;
    // The engine refuses a call past its cap with `too_many_calls`: the
    // Channel queues the rest under it (ADR 0038; issue 2529).
    if (result.maxCallsInFlight !== undefined && result.maxCallsInFlight !== null) {
      this.#channel.capCallsInFlight(result.maxCallsInFlight);
    }
    // A call queued under that cap with no timeout of its own waits no
    // longer than its setting's value (ADR 0038; issue 3193).
    this.#channel.boundByTimeoutSettings(result.timeouts ?? []);
    this.#engineInstance = result.engineInstance;
    this.#info = {
      kind: this.#reached.kind,
      product: result.product,
      version: result.productVersion,
      protocolVersion: result.protocolVersion,
      projectFile: this.#project?.carried.path ?? null,
      home: this.#reached.home,
      enginePath: this.#engine?.path ?? null,
      url: this.#reached.url,
      fallback: this.#reached.fallback ?? null,
    };
  }

  /**
   * `hello`'s answer, once `called` gives it within `connect`, its
   * `timeout.connect` (ADR 0083), counted from `startedAt`.
   *
   * @throws {DeadlineExceededError} past it, its `limit` naming
   * `timeout.connect` and the layer that set it, and `waited_for` `hello`.
   */
  async #answerWithin(
    called: Promise<unknown>,
    connect: ConnectLimit | null,
    startedAt: number,
  ): Promise<unknown> {
    if (connect === null) {
      return called;
    }
    const { value, limit } = connect;
    const answered = await within(
      called.then((answer) => ({ answer })),
      Math.max(0, startedAt + value - Date.now()),
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
      this.#closingLink.abort();
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
    // The link this close ends is not reached again; a Run closed above
    // waited for a link that had dropped to be reached again.
    this.#closingLink.abort();
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
    // Each call it fails gets a copy headed by its operation (`inCall`).
    if (this.#closing !== undefined) {
      return new ClosedError(runtimeMessage(CLOSED, { fix: CONNECT_AGAIN }), {
        fix: CONNECT_AGAIN,
      });
    }
    // A Daemon, this machine's or a remote one, outlives the Connection it
    // drops: the Connection is closed, not its engine ended (as in Rust).
    if (this.#reached.kind !== "attached") {
      return new ClosedError(runtimeMessage(DROPPED, { fix: CONNECT_AGAIN }), {
        fix: CONNECT_AGAIN,
        cause: end.kind === "ended" ? undefined : end.cause,
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

/** How long opening a Connection waits for `hello`'s answer, and the setting that says so. */
interface ConnectLimit {
  readonly value: number;
  readonly limit: ErrorLimit;
}

/**
 * The `timeout.connect` limit of a `connect()` whose `timeout` is
 * `timeout`, already checked by `timeoutOf`, and which read `project`: at
 * the run layer, given as an argument; when left out, the project file's,
 * or its 30 s default; `null` for none.
 */
export function connectLimit(
  timeout: number | null | undefined,
  project: ProjectFile | undefined,
): ConnectLimit | null {
  const key = "timeout.connect";
  if (timeout === null) {
    return null;
  }
  if (timeout !== undefined) {
    return { value: timeout, limit: { key, value: timeout, layer: "run", origin: "argument" } };
  }
  const value = project?.connectTimeout;
  return value === undefined
    ? { value: CONNECT_TIMEOUT, limit: { key, value: CONNECT_TIMEOUT, layer: "default" } }
    : { value, limit: { key, value, layer: "project_configuration" } };
}

/**
 * The error a call in flight fails with when its program closes its
 * Connection: cancelled by the program, its Connection closed under it
 * (ADRs 0078 and 0113). A call made once it is closed fails with `closed`.
 */
function cancelledByClose(): CallCancelledError {
  return new CallCancelledError("the call's Connection was closed", {
    details: { by: "program", closed: "connection" },
  });
}

/** The params every start carries, from `options` and the project file `connect()` read. */
function startParams(options: StartOptions, project: ProjectSettings | undefined): RunStartParams {
  const limits = limitList(options.limits);
  const settings = settingsOnWire(options.settings);
  return {
    ...(options.runId === undefined ? {} : { runId: options.runId }),
    ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
    ...(limits.length === 0 ? {} : { limits }),
    ...(settings === undefined ? {} : { settings }),
    ...(project === undefined ? {} : { project }),
  };
}

/**
 * The params a handler-less start carries besides its target, from
 * `options`; a `timeout` off its form fails with `setting_invalid` before
 * anything is sent.
 */
function handlerlessParams(
  options: HandlerlessStartOptions,
  project: ProjectSettings | undefined,
): Omit<RecipeStartParams, "recipe"> {
  const timeout = timeoutOf(options.timeout, "timeout.run");
  return {
    ...startParams(options, project),
    ...(options.recording === undefined ? {} : { recording: options.recording }),
    ...(timeout === undefined ? {} : { timeout }),
    ...(options.detached === true ? { detached: true } : {}),
  };
}

/** The `usage.set` params, in the wire's form, setting `limit` on the Client connection's own scope (ADR 0080's note of issue 2830). */
function usageSetParams(limit: RunLimit): unknown {
  const params: UsageSetParams = { scope: "connection", ...limit };
  return toWire(params, collieWire.params["usage.set"], collieWire.types);
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
