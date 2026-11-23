// Collie's Connection type (ADRs 0078, 0090 and 0095): `Collie.connect()`
// finds Collie's executable, starts it as an Attached engine with exactly the
// program's environment map, and finishes `hello`, naming Collie, before it
// returns. Everything else is the client package's common Connection.
import type {
  CatalogTarget,
  ConnectOptions,
  ConnectOverOptions,
  Connection,
  EngineInfo,
  HandlerlessStartOptions,
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
   * before returning, so a `GOODFETCH_` variable off the closed list
   * (`setting_unknown`) or one set but empty (`setting_invalid`), a missing
   * executable (`engine_not_found`) or a refused Product or version fails
   * here, with a `fix`. An aborted
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
   * Leaving the scope or `close()` stops the Run with a Checkpoint;
   * `finish()` ends it. A resumed Run is logged at info, and an ended one
   * at warning, through the host's logger.
   *
   * @experimental
   */
  start(options?: StartOptions): Promise<Run> {
    return this.#connection.start(options);
  }

  /**
   * Starts a Recipe's run, a Run with no handler in the program, and answers
   * once it is started. Its Run ID works as `run.start`'s. With `detached`,
   * the Run outlives the program that started it: closing its value frees
   * only the value, `run.finish` waits for its end and `run.stop` stops it.
   *
   * @experimental
   */
  startRecipe(recipe: string, options?: HandlerlessStartOptions): Promise<Run> {
    return this.#connection.startRecipe(recipe, options);
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
  startCatalogRun(target: CatalogTarget, options?: HandlerlessStartOptions): Promise<Run> {
    return this.#connection.startCatalogRun(target, options);
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
   * Shows each setting's effective value, the layer that set it and the
   * layer that locked it, from the built-in defaults, the Operator policy,
   * user configuration, the project configuration given and the run layer:
   * the Connection's own settings with the run values given over them, or a
   * live Run's as it opened when `run_id` names one; and lists the keys only
   * the other Product honours as that Product's.
   *
   * What the Connection runs with (ADR 0083), which each Run it starts
   * inherits: each setting under `keys`, each a key or a prefix of keys
   * such as `timeout`, every one when left out, with the origin that set
   * it too, as `goodfetch config explain` shows them. Fails with `closed`
   * once the Connection is closed.
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
