// The Connection (ADRs 0078, 0095 and 0099), declared once: what every
// Product's Connection type (`Collie`, `Foxhound`) offers, so a helper typed
// for it accepts either.
import type { EngineInfo, StartOptions } from "./client-connection.ts";
import type { SettingsExplanation } from "./generated/index.ts";
import type { JSONValue } from "./json.ts";
import type { RawCallOptions, RawParams } from "./raw-call.ts";
import type { Run } from "./run.ts";

/** A Client's open link to one engine, through which a program reaches its Runs. */
export interface Connection extends AsyncDisposable {
  /** The engine reached, fixed when the Connection opened. Read without a round trip. */
  engine(): EngineInfo;
  /**
   * Starts a Run and returns once it is started (ADR 0102); a `runId`
   * naming an interrupted Run resumes it, and one naming a finished Run
   * returns it as it ended. Fails with `closed` once the Connection is
   * closed.
   */
  start(options?: StartOptions): Promise<Run>;
  /**
   * Sends `operation`, one the SDK may not generate that acts on no value
   * or whose `params` name every value, with `params` as the wire's JSON in
   * snake_case, and answers its result as the engine sent it (ADR 0099).
   * `timeout` is sent as the operation's own; `signal` cancels the call. It
   * fails with the same classes as any call, `closed` once the Connection
   * is closed, and is never retried.
   */
  call(operation: string, params?: RawParams, options?: RawCallOptions): Promise<JSONValue>;
  /**
   * What the Connection runs with (ADR 0083): each setting under `keys`,
   * each a key or a prefix of keys such as `timeout`, every one when left
   * out, with its effective value, the layer and origin that set it and
   * the layer that locked it, as `goodfetch config explain` shows them: the
   * Connection's own `settings` over configuration and the Operator
   * policy, which each Run it starts inherits. Fails with `closed` once
   * the Connection is closed.
   */
  explainSettings(keys?: readonly string[]): Promise<SettingsExplanation>;
  /**
   * Closes the Connection; idempotent. Closes every Run it opened first,
   * stopping one still running, cancels the calls in flight, and stops an
   * Attached engine, waiting for it. Any later call fails with `closed`.
   */
  close(): Promise<void>;
}
