// The Connection (ADRs 0078, 0095 and 0099), declared once: what every
// Product's Connection type (`Collie`, `Foxhound`) offers, so a helper typed
// for it accepts either.
import type { EngineInfo, HandlerlessStartOptions, StartOptions } from "./client-connection.ts";
import type { CatalogTarget, SettingsExplanation } from "./generated/index.ts";
import type { JSONValue } from "./json.ts";
import type { RawCallOptions, RawParams } from "./raw-call.ts";
import type { Run } from "./run.ts";
import type { RunView } from "./run-view.ts";

/** A Client's open link to one engine, through which a program reaches its Runs. */
export interface Connection extends AsyncDisposable {
  /** The engine reached, fixed when the Connection opened. Read without a round trip. */
  engine(): EngineInfo;
  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint.
   * `finish()` ends it.
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
   * Fails with `closed` once the Connection is closed.
   *
   * In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK installs
   * no signal handler, and a signal the program does not handle ends it as a
   * crash would (ADRs 0078, 0093).
   *
   * @experimental
   */
  start(options?: StartOptions): Promise<Run>;
  /**
   * Starts a Recipe's run, a Run with no handler in the program, and answers
   * once it is started. Its Run ID works as `run.start`'s. With `detached`,
   * the Run outlives the program that started it: closing its value frees
   * only the value, `run.finish` waits for its end and `run.stop` stops it.
   *
   * @experimental
   */
  startRecipe(recipe: string, options?: HandlerlessStartOptions): Promise<Run>;
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
  startCatalogRun(target: CatalogTarget, options?: HandlerlessStartOptions): Promise<Run>;
  /**
   * Opens a view of the Run `runId` names, one another program started such
   * as a detached Run, with no round trip (ADR 0102). The view takes no
   * writer role, and closing it frees only the view. Fails with `closed`
   * once the Connection is closed.
   */
  openRun(runId: string): Promise<RunView>;
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
  explainSettings(keys?: readonly string[]): Promise<SettingsExplanation>;
  /**
   * Closes the Connection; idempotent. Closes every Run it opened first,
   * stopping one still running, cancels the calls in flight, and stops an
   * Attached engine, waiting for it. Any later call fails with `closed`.
   */
  close(): Promise<void>;
}
