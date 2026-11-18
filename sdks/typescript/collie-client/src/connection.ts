// The Connection (ADRs 0078, 0095 and 0099), declared once: what every
// Product's Connection type (`Collie`, `Foxhound`) offers, so a helper typed
// for it accepts either.
import type { EngineInfo, StartOptions } from "./client-connection.ts";
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
   * Closes the Connection; idempotent. Closes every Run it opened first,
   * stopping one still running, cancels the calls in flight, and stops an
   * Attached engine, waiting for it. Any later call fails with `closed`.
   */
  close(): Promise<void>;
}
