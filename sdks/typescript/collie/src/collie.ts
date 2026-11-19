// Collie's Connection type (ADRs 0078, 0090 and 0095): `Collie.connect()`
// finds Collie's executable, starts it as an Attached engine with exactly the
// program's environment map, and finishes `hello`, naming Collie, before it
// returns. Everything else is the client package's common Connection.
import type {
  ConnectOptions,
  ConnectOverOptions,
  Connection,
  EngineInfo,
  JSONValue,
  RawCallOptions,
  RawParams,
  Run,
  SettingsExplanation,
  StartOptions,
} from "@goodfetch/collie-client";
import {
  holdInRealm,
  openEngine,
  openOver,
  type ClientConnection,
  type Transport,
} from "@goodfetch/collie-client/runtime";
import { collie } from "./product.ts";

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
    return open(await openEngine(collie, options));
  }

  /** The engine reached: its kind, Product, Product version and protocol version. Read without a round trip. */
  engine(): EngineInfo {
    return this.#connection.engine();
  }

  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint;
   * `finish()` ends it. Starts a Run and returns once it is started
   * (ADR 0102): a `runId` naming an interrupted Run resumes it, logged at
   * info, and one naming a finished Run returns it as it ended, logged at
   * warning, through the host's logger.
   */
  start(options?: StartOptions): Promise<Run> {
    return this.#connection.start(options);
  }

  /**
   * Sends `operation`, one the SDK may not generate that acts on no value
   * or whose `params` name every value, with `params` as the wire's JSON in
   * snake_case, and answers its result as the engine sent it (ADR 0099).
   * `timeout` is sent as the operation's own; `signal` cancels the call. It
   * fails with the same classes as any call, `closed` once the Connection
   * is closed, and is never retried.
   */
  call(operation: string, params?: RawParams, options?: RawCallOptions): Promise<JSONValue> {
    return this.#connection.call(operation, params, options);
  }

  /**
   * What the Connection runs with (ADR 0083): each setting under `keys`,
   * each a key or a prefix of keys such as `timeout`, every one when left
   * out, with its effective value, the layer and origin that set it and
   * the layer that locked it, as `goodfetch config explain` shows them: the
   * Connection's own `settings` over configuration and the Operator
   * policy, which each Run it starts inherits. Fails with `closed` once
   * the Connection is closed.
   */
  explainSettings(keys?: readonly string[]): Promise<SettingsExplanation> {
    return this.#connection.explainSettings(keys);
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
  return open(await openOver(collie, transport, options));
}
