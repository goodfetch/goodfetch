// Where the main types live (ADR 0095): this package, `@goodfetch/collie`, holds `Collie`; the Collie
// client package it re-exports, `@goodfetch/collie-client`, holds `Page`, `Locator`, `Run`, `Session`
// and the results and errors; `@goodfetch/foxhound` holds `Foxhound`, `Crawl`, `Label`, `Dataset`
// and `CrawlContext`.
//
// `@goodfetch/collie` (ADRs 0090, 0095): Collie's TypeScript SDK and its `goodfetch-collie` bin. It
// re-exports the Collie client package, whose values (Run, Session, Page, errors) a Foxhound handler
// gets too, and adds the connection, which finds Collie's executable through its own platform
// packages and reads only `GOODFETCH_COLLIE_ENGINE_PATH` (ADR 0090).
export * from "@goodfetch/collie-client";

import type {
  CallOptions,
  ConnectOptions,
  Connection,
  EngineInfo,
  ExplainedSetting,
  Identities,
  JSONObject,
  JSONValue,
  Run,
  RunView,
  SettingKey,
  StartOptions,
} from "@goodfetch/collie-client";

/**
 * A connection to an engine, Collie's or Foxhound's (ADR 0095): the entry point (ADR 0078). Each one
 * is independent, and many may be open in one process. A Collie connection announces Collie in
 * `hello`, so a Foxhound engine accepts it too.
 */
export declare class Collie implements Connection {
  private constructor();
  /** Starts or finds the engine and completes `hello` before returning. */
  static connect(options?: ConnectOptions): Promise<Collie>;
  engine(): EngineInfo;
  /** The install's Saved identities (ADR 0092). */
  readonly identities: Identities;
  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
   *
   * Starts a Run and returns once it is started; a `runId` naming an existing Run resumes it or
   * returns it as it ended (ADR 0102), as `Connection.start` says. Fails with `model_price_unknown` when a currency Limit or `ai.allowance.price` applies and the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).
   */
  start(options?: StartOptions): Promise<Run>;
  openRun(runId: string, options?: CallOptions): Promise<RunView>;
  /** Cancels in-flight calls, closes every Run it opened and ends every borrow; stops an Attached engine, only disconnects from a Daemon. */
  close(options?: CallOptions): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
  /** The raw catalog call (ADR 0099): `params` names every value itself. */
  call(operation: string, params?: JSONObject, options?: CallOptions): Promise<JSONValue>;
}
