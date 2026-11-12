// Collie's Connection type (ADRs 0078, 0090 and 0095): `Collie.connect()`
// finds Collie's executable, starts it as an Attached engine with exactly the
// program's environment map, and finishes `hello`, naming Collie, before it
// returns. Everything else is the client package's common Connection.
import {
  collieErrorClasses,
  type Connection,
  type EngineInfo,
  type Logger,
  type Run,
  type StartOptions,
} from "@goodfetch/collie-client";
import {
  AttachedEngine,
  boundsOf,
  ClientConnection,
  holdInRealm,
  type ChannelBounds,
  type Transport,
} from "@goodfetch/collie-client/runtime";
import { locateEngine } from "./engine-path.ts";

/** Options of opening a Connection, over an Attached engine's stdio or a transport a test holds. */
export interface ConnectOverOptions {
  /**
   * Cancels the opening: it rejects with the signal's reason, once the
   * Attached engine it started has stopped and its streams are closed.
   */
  readonly signal?: AbortSignal | undefined;
  /**
   * How many calls may wait for their answer at once, 10,000 when left
   * out: past it a new call fails at once with `connection_overloaded`. A
   * whole number of 1 or more, as is the next bound, or the opening fails
   * with `setting_invalid`.
   */
  readonly maxPendingCalls?: ChannelBounds["maxPendingCalls"];
  /**
   * How many bytes may wait to be written to the engine, 64 MiB when left
   * out: past it a new call fails at once with `connection_overloaded`. One
   * message larger than this is still taken while nothing waits.
   */
  readonly maxQueuedWriteBytes?: ChannelBounds["maxQueuedWriteBytes"];
}

/** Options of `connect()`. */
export interface ConnectOptions extends ConnectOverOptions {
  /**
   * The environment map, read here and never at import: the process's own
   * when left out (ADRs 0062, 0083). Only `GOODFETCH_COLLIE_ENGINE_PATH` is
   * read, and the Attached engine gets exactly this map as its environment.
   */
  readonly env?: Readonly<Record<string, string>> | undefined;
  /** The host's logger, which gets the engine's stderr lines at warning. With none, the SDK is silent. */
  readonly logger?: Logger | undefined;
}

let open: (connection: ClientConnection) => Collie;

/**
 * A Connection to an engine, Collie's or Foxhound's (ADR 0095): the entry
 * point (ADR 0078). Each one is independent, and many may be open in one
 * process. It belongs to the realm that opened it: a `worker_threads`
 * worker connects on its own (ADR 0081).
 */
export class Collie implements Connection {
  static {
    open = (connection) => new Collie(connection);
  }

  readonly #connection: ClientConnection;

  private constructor(connection: ClientConnection) {
    this.#connection = connection;
    holdInRealm(this);
  }

  /**
   * Starts Collie's engine as an Attached engine and completes `hello`
   * before returning, so a missing executable (`engine_not_found`) or a
   * refused Product or version fails here, with a `fix`. An aborted
   * `signal` fails it with the signal's reason, leaving no engine behind.
   */
  static async connect(options: ConnectOptions = {}): Promise<Collie> {
    options.signal?.throwIfAborted();
    // Read before the engine starts, so a refused option leaves none behind.
    const common = opening(options);
    const env = options.env ?? processEnvironment();
    const location = locateEngine(env);
    const engine = await AttachedEngine.start({
      path: location.path,
      env,
      notFoundFix: location.notFoundFix,
      logger: options.logger,
    });
    return open(
      await ClientConnection.open({
        transport: engine.transport,
        engine,
        needs: "collie",
        errorClasses: collieErrorClasses,
        ...common,
      }),
    );
  }

  /** The engine reached: its kind, Product, Product version and protocol version. Read without a round trip. */
  engine(): EngineInfo {
    return this.#connection.engine();
  }

  /**
   * Starts a Run and returns once it is started (ADR 0102); a `runId`
   * naming an existing Run returns that Run. `finish()` ends it; closing it
   * unfinished stops it, resumable by its Run ID.
   */
  start(options?: StartOptions): Promise<Run> {
    return this.#connection.start(options);
  }

  /**
   * Closes the Connection; idempotent. Closes every Run it opened first,
   * stopping one still running, cancels the calls in flight, then stops the
   * Attached engine and waits for it. Any later call fails with `closed`.
   */
  close(): Promise<void> {
    return this.#connection.close();
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

/**
 * Opens a Collie Connection over `transport`, such as the Scripted engine's
 * in-memory player in a program's tests (ADR 0069): the same Connection as
 * `connect()`'s, with no process behind it. Programs reach it through
 * `@goodfetch/collie/testing`.
 */
export async function connectOver(
  transport: Transport,
  options: ConnectOverOptions = {},
): Promise<Collie> {
  return open(
    await ClientConnection.open({
      transport,
      needs: "collie",
      errorClasses: collieErrorClasses,
      ...opening(options),
    }),
  );
}

/**
 * The common Connection's part of `options`.
 *
 * @throws {SettingInvalidError} when a bound is no whole number of 1 or more.
 */
function opening(options: ConnectOverOptions): {
  readonly bounds: ChannelBounds;
  readonly signal: AbortSignal | undefined;
} {
  const { signal, maxPendingCalls, maxQueuedWriteBytes } = options;
  return { bounds: boundsOf({ maxPendingCalls, maxQueuedWriteBytes }), signal };
}

/** The process's own environment, its unset entries left out. */
function processEnvironment(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  );
}
