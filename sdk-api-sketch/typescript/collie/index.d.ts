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
  start(options?: StartOptions): Promise<Run>;
  openRun(id: string, options?: CallOptions): Promise<RunView>;
  /** Cancels in-flight calls, closes every Run it opened and ends every borrow; stops an Attached engine, only disconnects from a Daemon. */
  close(options?: CallOptions): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
  /** The raw catalog call (ADR 0099): `params` names every value itself. */
  call(operation: string, params?: JSONObject, options?: CallOptions): Promise<JSONValue>;
}
